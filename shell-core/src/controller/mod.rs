//! 殻の判断と 1 本の口（ADR 0041 §4.1。レーン L-A2）。
//!
//! 今の `IkariInputController`・`AppContext`・`SettingsStore` の OS に依存しない判断
//! （`selectMode`、`leaveCurrentMode`、`recordModeSwitch`、`syncSettings`、`syncEmojiTable`、`handleEnglish`、
//! `flushEnglishShortcode`、`flushEnglishPool`、`requestPostalLookup`、`apply(EnglishTextOutcome)`、
//! `settingsDidChange`、`restartEngine`、`refreshEngineInfo` のリトライ、`perform(MenuCommand)`、`nextSessionID`）を持ち、
//! 状態機械（`machine`）、エンジン（`engine`）、設定（`settings`）をつなぐ。
//!
//! **契約**: `Shell::new`、`Shell::dispatch`、`Shell::engine`、`ShellConfig`、`SettingsSource`、`ShellError` は M1-a で固めた。
//! M1-a の口で表せなかった OS とのやりとり（入力モードの通知と選択、候補ウィンドウの上下、IMK の `commitComposition`、
//! 英数・かなキー、設定アプリとの接続）は、L-E で `ShellEvent`・`ShellAction` に足した。同じ働きの公開の関数も残す
//! （シナリオの `host` の手順と Windows が使う）。モードの選択の依頼は `take_mode_request` で取り出す
//! （macOS は `shell-ffi` が `ShellAction::SelectMode` にする）。

mod anchor;
mod background;
mod bridge_link;
mod settings_backend;
#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

pub use anchor::AnchorMemory;
pub use background::{BackgroundMode, EMOJI_TABLE_TIMEOUT, HELLO_TIMEOUT};
pub use settings_backend::{ROMAJI_FILE_NAME, UpdateFailed};

use background::{Background, Job, JobResult};
use bridge_link::{BridgeLink, BridgeWork};
use settings_backend::{SettingsBackend, load_keymap};

use crate::diag::DiagEnd;
use crate::engine::EngineLink;
use crate::english::{
    EnglishKeyClass, EnglishMode, EnglishShortcodeEnvironment, EnglishTextOutcome, InputMode,
};
use crate::host::{Clock, EngineTransport, TextAccess};
use crate::keymap::Keymap;
use crate::machine::{Composer, InputStateMachine, MachineEngine, RomajiSource, SessionInfo};
use crate::menu::{EngineLmState, InputMenuState, ProfileDomain};
use crate::panel::{AnchorSource, PanelSide};
use crate::postal::PostalLookupRequest;
use crate::settings::{
    SettingsChange, SettingsEntry, SettingsError, engine_settings_differ, preset_name,
};
use crate::shortcode::{ShortcodeEntry, ShortcodeTable, jis_ascii};
use crate::types::{
    ConnId, InputMethodKind, KeyInput, KeyKind, MenuCommand, MenuModel, SettingsSnapshot,
    ShellAction, ShellEvent, TimerId,
};

/// 設定の読み元。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsSource {
    /// `settings.tsv` のファイル（本番）。`SettingsFileChanged` で読み直す
    File(PathBuf),
    /// 文字列（シナリオ、テスト）
    Inline(String),
}

/// `Shell::new` の設定（ADR 0041 §4.4 の `ikari_shell_new(config, …)` の `config`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellConfig {
    /// 殻の版（エンジンとの `hello` の照合に使う）
    pub shell_version: String,
    pub settings: SettingsSource,
    /// 同梱の資源の置き場所（`keymap/<preset>.tsv` など。macOS は `.app` の `Resources`）
    pub resources_dir: PathBuf,
}

/// `Shell::new` の誤り。
#[derive(Debug)]
pub enum ShellError {
    Settings(SettingsError),
    /// 同梱の資源（キー割り当ての表など）が読めない
    Resources(std::io::Error),
}

impl std::fmt::Display for ShellError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Settings(error) => write!(f, "settings: {error}"),
            Self::Resources(error) => write!(f, "resources: {error}"),
        }
    }
}

impl std::error::Error for ShellError {}

/// LM の読み込み中に `Hello` で状態を取り直すタイマー（今の `AppContext.lmPollInterval` の `asyncAfter`）。
pub const LM_POLL_TIMER: TimerId = TimerId(1);
/// 主スレッドの外の問い合わせの結果を見回るタイマー（本番のスレッドで流すときだけ使う）。
pub const BACKGROUND_POLL_TIMER: TimerId = TimerId(2);
/// LM の読み込み中に状態を取り直す間隔と上限（今の `AppContext.lmPollInterval`・`lmPollLimit`）。
pub const LM_POLL_INTERVAL_MS: u32 = 2000;
pub const LM_POLL_LIMIT: u32 = 30;
/// 結果を見回る間隔。暫定（計測なし。郵便番号の住所が出るまでの遅れの上限になる。L-E の実機で見直す）
pub const BACKGROUND_POLL_INTERVAL_MS: u32 = 20;
/// キー割り当てでモードを頼んでから macOS の通知が来るまでの許容（今の `keySwitchWindow`）。
pub const KEY_SWITCH_WINDOW: Duration = Duration::from_secs(1);

/// Bundle ID とセッションの ID の対応を覚える数の上限（ADR 0046 E4）。動作に効かない上限なので計測しない。
/// エンジンの `SESSION_LIMIT` より小さく、日常に開くアプリの数より大きい
const SESSION_ID_MEMORY_MAX: usize = 64;

/// いまの入力先（IMK のコントローラ 1 つ分）。
struct Client {
    machine: InputStateMachine,
    bundle_id: String,
    /// 状態機械に適用済みの設定の版
    applied_settings_version: u64,
    /// 状態機械に渡した絵文字の表の版
    applied_emoji_version: u64,
}

/// 絵文字のショートコードの表（今の `AppContext` の `emojiTable` ほか）。
#[derive(Default)]
struct EmojiTableState {
    table: Arc<ShortcodeTable>,
    data_label: String,
    version: u64,
    fetching: bool,
}

/// 殻の本体。1 つのスレッドから呼ぶ（`Send` だが `Sync` ではない。ADR 0041 §4.2）。
pub struct Shell {
    config: ShellConfig,
    clock: Arc<dyn Clock>,
    engine: Arc<EngineLink>,
    settings: SettingsBackend,
    keymap: Arc<Keymap>,
    keymap_version: u64,
    romaji: RomajiSource,
    client: Option<Client>,
    last_session_id: u64,
    /// Bundle ID → セッションの ID（使うたびに末尾へ。`SESSION_ID_MEMORY_MAX` を超えたら古いものから忘れる。メモリだけ）
    session_ids: Vec<(String, u64)>,

    input_mode: InputMode,
    english: EnglishMode,
    /// `mode.*` で選択を頼んだ時刻（今の `keySwitchRequestedAt`）
    key_switch_requested_at: Option<Duration>,
    /// OS に選んでもらうモード（`take_mode_request`）
    mode_request: Option<InputMode>,

    emoji: EmojiTableState,
    lm_state: Option<EngineLmState>,
    profile: Option<Vec<ProfileDomain>>,
    /// 状態の取り直しが進行中か。エンジンを起こし直すたびに `poll_generation` を進め、古い取り直しを打ち切る
    refreshing: bool,
    poll_generation: u64,
    /// LM の読み込み中の次の取り直し（回数、世代）。`LM_POLL_TIMER` で流す
    pending_lm_poll: Option<(u32, u64)>,
    last_menu: Option<MenuModel>,
    /// 最後にメニューを作ったときの入力（設定の版、LM の状態、分野の版）。同じなら作り直さない（打鍵ごとの手間を減らす）
    menu_inputs: Option<(u64, Option<EngineLmState>, u64)>,
    /// 分野を受け取るたびに進める
    profile_version: u64,

    background: Background,
    background_poll_scheduled: bool,
    /// 設定アプリとの接続（L-E）
    bridge: BridgeLink,
    /// 次の `finish` で返す、設定アプリへの通知
    pending_bridge: Vec<ShellAction>,
}

impl Shell {
    /// 設定とキー割り当てを読み、エンジンへの口を作る（エンジンにはまだつながない）。
    pub fn new(
        config: ShellConfig,
        transport: Box<dyn EngineTransport>,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, ShellError> {
        let engine = Arc::new(EngineLink::new(transport, Arc::clone(&clock)));
        let settings = match &config.settings {
            SettingsSource::File(path) => SettingsBackend::file(path.clone()),
            SettingsSource::Inline(text) => SettingsBackend::inline(text.clone()),
        };
        let keymap = load_keymap(&config.resources_dir, settings.snapshot());
        let keymap_version = settings.version();
        let romaji = settings.romaji_source();
        let background = Background::new(BackgroundMode::Thread, config.shell_version.clone());
        Ok(Self {
            config,
            clock,
            engine,
            settings,
            keymap,
            keymap_version,
            romaji,
            client: None,
            last_session_id: 0,
            session_ids: Vec::new(),
            input_mode: InputMode::Japanese,
            english: EnglishMode::new(),
            key_switch_requested_at: None,
            mode_request: None,
            emoji: EmojiTableState::default(),
            lm_state: None,
            profile: None,
            refreshing: false,
            poll_generation: 0,
            pending_lm_poll: None,
            last_menu: None,
            menu_inputs: None,
            profile_version: 0,
            background,
            background_poll_scheduled: false,
            bridge: BridgeLink::default(),
            pending_bridge: Vec::new(),
        })
    }

    /// OS のイベントを 1 つ処理し、OS 側が順に実行する動作の列を返す。
    /// 周辺テキストと選択は、要るときだけ `text` から同期に取る。
    pub fn dispatch(&mut self, event: ShellEvent, text: &mut dyn TextAccess) -> Vec<ShellAction> {
        let mut actions = match event {
            ShellEvent::Key(input) => self.handle_key(&input, text),
            ShellEvent::Activate { bundle_id, secure } => self.activate(&bundle_id, secure),
            ShellEvent::Deactivate => self.deactivate(),
            ShellEvent::MenuCommand { command } => self.perform(command),
            ShellEvent::SettingsFileChanged => {
                self.reload_settings();
                Vec::new()
            }
            ShellEvent::Timer { id } => self.timer_fired(id),
            ShellEvent::BridgeFrame { conn, frame } => self.bridge_frame(conn, &frame),
            ShellEvent::InputModeChanged { mode_id, secure } => {
                if let Some(client) = &mut self.client {
                    client.machine.set_secure(secure);
                }
                self.change_input_mode(&mode_id)
            }
            ShellEvent::CommitComposition => self.commit_composition_now(),
            ShellEvent::PanelShown { side, source } => {
                self.panel_shown(side, source);
                Vec::new()
            }
            ShellEvent::PanelHidden => {
                self.panel_hidden();
                Vec::new()
            }
            ShellEvent::BridgeClosed { conn } => {
                self.bridge.closed(conn);
                Vec::new()
            }
        };
        self.finish(&mut actions);
        actions
    }

    /// エンジンへの口。落ちた通知（`EngineLink::notify_down`）を別のスレッドから呼ぶために、`Arc` を複製して持つ。
    pub fn engine(&self) -> &Arc<EngineLink> {
        &self.engine
    }

    // MARK: 契約の外の口（ADR 0041 §4.1 の `ShellEvent`・`ShellAction` に足す候補。L-E で決める）

    /// 主スレッドの外の問い合わせの流し方（本番は `Thread`。シナリオ、テスト、性能の計測は `Inline`）。
    pub fn set_background_mode(&mut self, mode: BackgroundMode) {
        self.background.set_mode(mode);
    }

    /// 今の入力モード。
    pub fn input_mode(&self) -> InputMode {
        self.input_mode
    }

    /// キー割り当て（`mode.*`）で頼んだモード。OS は `dispatch` の動作を実行したあとに取り出し、選んでもらう
    /// （macOS は `client.selectMode`）。実際に替わったかは `input_mode_changed` で知る。
    pub fn take_mode_request(&mut self) -> Option<InputMode> {
        self.mode_request.take()
    }

    /// OS からのモードの通知（macOS の `setValue(_:forTag: kTextServiceInputModePropertyTag)`。値はモードの ID）。
    /// 知らない ID、同じモードなら何もしない。
    pub fn input_mode_changed(&mut self, mode_id: &str) -> Vec<ShellAction> {
        let mut actions = self.change_input_mode(mode_id);
        self.finish(&mut actions);
        actions
    }

    /// IMK の `commitComposition`（クリックなどで外から確定させられた）。`ShellEvent::CommitComposition` と同じ。
    pub fn commit_composition(&mut self) -> Vec<ShellAction> {
        let mut actions = self.commit_composition_now();
        self.finish(&mut actions);
        actions
    }

    /// JIS の英数・かなキー（`EnglishMode::absorbs_key_code`）。OS がモードを切り替えたあと同じキーが届くので、
    /// どのモードでも受け止めるだけにする（その前にショートコードの表示があれば確定する。ADR 0026 §5.2）。
    /// `ShellEvent::Key` の `KeyKind::Eisu`・`Kana` と同じ。
    pub fn absorb_mode_key(&mut self) -> Vec<ShellAction> {
        let mut actions = self.absorb_mode_key_now();
        self.finish(&mut actions);
        actions
    }

    fn change_input_mode(&mut self, mode_id: &str) -> Vec<ShellAction> {
        let Some(mode) = InputMode::from_mode_id(mode_id) else {
            return Vec::new();
        };
        if mode == self.input_mode {
            return Vec::new();
        }
        let actions = self.leave_current_mode();
        let previous = self.input_mode;
        self.input_mode = mode;
        self.record_mode_switch(previous, mode);
        actions
    }

    fn commit_composition_now(&mut self) -> Vec<ShellAction> {
        let mut actions = self
            .client
            .as_mut()
            .map(|client| {
                client
                    .machine
                    .commit_composition(DiagEnd::CommitComposition)
            })
            .unwrap_or_default();
        actions.extend(self.flush_english_shortcode(DiagEnd::CommitComposition));
        actions
    }

    fn absorb_mode_key_now(&mut self) -> Vec<ShellAction> {
        consumed(self.flush_english_shortcode(DiagEnd::ModeSwitch))
    }

    /// 候補ウィンドウを出した（OS が位置を決めたあと）。上下と、アンカーがどこから取れたか（診断ログ）。
    pub fn panel_shown(&mut self, side: PanelSide, source: Option<AnchorSource>) {
        if let Some(client) = &mut self.client {
            client.machine.set_panel_side(side);
            if let Some(source) = source {
                client.machine.note_panel_shown(source);
            }
        }
        if let Some(source) = source {
            self.english.note_panel_shown(source);
        }
    }

    /// 候補ウィンドウを閉じた（`HideCandidates` の実行）。上下を既定に戻す。
    pub fn panel_hidden(&mut self) {
        if let Some(client) = &mut self.client {
            client.machine.set_panel_side(PanelSide::Below);
        }
    }

    /// 状態機械（テストと、L-E の移行の間の確認用）。
    pub fn machine(&self) -> Option<&InputStateMachine> {
        self.client.as_ref().map(|client| &client.machine)
    }

    /// 今の設定（テスト用）。
    pub fn settings(&self) -> &SettingsSnapshot {
        self.settings.snapshot()
    }

    // MARK: 入力先（今の `IkariInputController` の init・`activateServer`・`deactivateServer`）

    fn next_session_id(&mut self) -> u64 {
        self.last_session_id = self.last_session_id.wrapping_add(1);
        self.last_session_id
    }

    /// そのアプリのセッションの ID。覚えていれば同じ ID（エンジンの休止と再開で、30 分以内の履歴が続く。ADR 0046 A1）。
    /// Bundle ID が取れない（空）ときは、どのアプリか分からないので使い回さず、いつも新しい ID
    fn session_id_for(&mut self, bundle_id: &str) -> u64 {
        if bundle_id.is_empty() {
            return self.next_session_id();
        }
        let id = match self.session_ids.iter().position(|(b, _)| b == bundle_id) {
            Some(index) => self.session_ids.remove(index).1,
            None => self.next_session_id(),
        };
        self.session_ids.push((bundle_id.to_string(), id));
        if self.session_ids.len() > SESSION_ID_MEMORY_MAX {
            self.session_ids.remove(0);
        }
        id
    }

    fn activate(&mut self, bundle_id: &str, secure: bool) -> Vec<ShellAction> {
        // 手で編集した settings.tsv に気づく（stat 1 回）。idle のときに反映する
        self.reload_settings();
        let needs_client = self
            .client
            .as_ref()
            .is_none_or(|client| client.bundle_id != bundle_id);
        if needs_client {
            self.client = Some(self.make_client(bundle_id));
        }
        self.sync_settings();
        if let Some(client) = &mut self.client {
            client.machine.set_secure(secure);
            client.machine.activate();
        }
        // 絵文字の表と、エンジンの起動の前倒し・メニューの状態の取り直し。主スレッドは止めない
        self.refresh_emoji_table();
        self.refresh_engine_info();
        Vec::new()
    }

    /// 新しい入力先（IMK のコントローラを作るのと同じ）。セッションの ID は、同じアプリなら使い回す
    fn make_client(&mut self, bundle_id: &str) -> Client {
        self.refresh_keymap();
        let settings = self.settings.shared();
        let session = SessionInfo {
            session_id: self.session_id_for(bundle_id),
            client_bundle_id: bundle_id.to_string(),
            secure: false,
        };
        let engine: Arc<dyn MachineEngine> = self.engine.clone();
        let mut machine = InputStateMachine::new(
            engine,
            Composer::new(settings.input_method, self.romaji),
            Arc::clone(&self.keymap),
            settings,
            session,
            Arc::clone(&self.clock),
        );
        machine.set_shortcode_table(Arc::clone(&self.emoji.table));
        Client {
            machine,
            bundle_id: bundle_id.to_string(),
            applied_settings_version: self.settings.version(),
            applied_emoji_version: self.emoji.version,
        }
    }

    fn deactivate(&mut self) -> Vec<ShellAction> {
        let mut actions = self
            .client
            .as_mut()
            .map(|client| client.machine.commit_composition(DiagEnd::Deactivate))
            .unwrap_or_default();
        actions.extend(self.flush_english_shortcode(DiagEnd::Deactivate));
        actions.extend(self.hide_panel());
        // `end_session` の前に送る（終わったセッションには入らない）
        self.flush_english_pool();
        if let Some(client) = &mut self.client {
            client.machine.deactivate();
        }
        actions
    }

    fn hide_panel(&mut self) -> Vec<ShellAction> {
        self.panel_hidden();
        vec![ShellAction::HideCandidates]
    }

    // MARK: キー（今の `IkariInputController.handle`）

    fn handle_key(&mut self, input: &KeyInput, text: &mut dyn TextAccess) -> Vec<ShellAction> {
        if self.client.is_none() {
            // `activateServer` を経ずに打鍵が来た（殻が起動し直した直後）。入力先は分からない
            self.client = Some(self.make_client(""));
        }
        self.sync_settings();
        self.sync_emoji_table();
        // JIS の英数・かなキーは、どのモードでも受け止めるだけ（secure を見る前。今の `IkariInputController.handle` の順）
        if matches!(input.key, KeyKind::Eisu | KeyKind::Kana) {
            return self.absorb_mode_key_now();
        }
        let Some(client) = &mut self.client else {
            return vec![ShellAction::PassThrough];
        };
        // 打鍵のたびに secure を見る（NFR-18）。セッションを始めていなければ始める（#315）
        client.machine.set_secure(input.secure);
        client.machine.activate_if_needed();

        if self.input_mode == InputMode::English {
            return self.handle_english(input, text);
        }
        let input = &japanese_key(input, client.machine.settings().input_method);
        // キー割り当ての `mode.*` は、変換より先に見る（合成中なら今の表示のまま確定してから切り替える）
        if let Some(target) =
            EnglishMode::mode_switch_target(input, &self.keymap_for_client(), self.input_mode)
        {
            return consumed(self.select_mode(target));
        }
        match &mut self.client {
            Some(client) => client.machine.handle(input, text),
            None => vec![ShellAction::PassThrough],
        }
    }

    /// 状態機械に適用済みのキー割り当て（入力中は、設定が変わっても前のまま）
    fn keymap_for_client(&self) -> Arc<Keymap> {
        match &self.client {
            Some(client) => Arc::clone(client.machine.keymap()),
            None => Arc::clone(&self.keymap),
        }
    }

    // MARK: 英語のモード（ADR 0026 §5.2。エンジンは同期で呼ばない）

    /// 英語のモードの打鍵。順は、secure → `mode.*` → ショートコードの口 → 文脈のたまりに足して素通し。
    fn handle_english(&mut self, input: &KeyInput, text: &mut dyn TextAccess) -> Vec<ShellAction> {
        let input = english_key(input);
        let keymap = self.keymap_for_client();
        match EnglishMode::classify(&input, &keymap, self.input_mode) {
            EnglishKeyClass::PassThroughSecure => {
                // 表示があればそのまま確定する。たまりは送らずに捨てる（secure の前後を 1 件につなげない）
                let mut actions = self.flush_english_shortcode(DiagEnd::CommitComposition);
                self.english.discard();
                actions.push(ShellAction::PassThrough);
                actions
            }
            EnglishKeyClass::SwitchMode(target) => {
                let actions = if target != self.input_mode {
                    self.select_mode(target)
                } else {
                    Vec::new()
                };
                consumed(actions)
            }
            EnglishKeyClass::Text => {
                if let Some(actions) = self.english_clause_key(&input, text) {
                    return actions;
                }
                // 設定と入力先の ID は借りる（打鍵ごとに複製しない）。借用はこの塊の中で終わる
                let now = self.now();
                let outcome = {
                    let (settings, bundle_id) = match &self.client {
                        Some(client) => (client.machine.settings(), client.bundle_id.as_str()),
                        None => (self.settings.snapshot(), ""),
                    };
                    let environment = EnglishShortcodeEnvironment {
                        settings,
                        client_bundle_id: bundle_id,
                        secure: false,
                        is_repeat: input.is_repeat,
                    };
                    self.english.handle_text(&input, &environment, now)
                };
                let pass_key = outcome.pass_key;
                let lookup = outcome.lookup.clone();
                let mut actions = self.apply_english(outcome);
                if let Some(request) = lookup {
                    self.request_postal_lookup(request);
                }
                self.record_english_episode();
                if pass_key {
                    actions.push(ShellAction::PassThrough);
                }
                consumed(actions)
            }
        }
    }

    /// 英語のモードでの文節削除のキー（ADR 0053 §6.4）。当たれば状態機械に渡す（日本語のモードの確定後と同じ規則。
    /// 直前が英字ならエンジンを呼ばずに素通し）。当たらないキーは、取り消しの覚えを捨てて `None`。
    /// ショートコードと郵便番号の状態の中は、今の扱い
    fn english_clause_key(
        &mut self,
        input: &KeyInput,
        text: &mut dyn TextAccess,
    ) -> Option<Vec<ShellAction>> {
        let in_shortcode = self.english.is_shortcode_active() || self.english.is_postal_active();
        let handled = !in_shortcode
            && self
                .client
                .as_ref()
                .is_some_and(|client| client.machine.handles_clause_key(input));
        if !handled {
            if let Some(client) = &mut self.client {
                client.machine.forget_clause_undo();
            }
            return None;
        }
        // 消す前に、英語の文脈のたまりを確定履歴へ送る（削除で取り消さない。ADR 0050 D9）
        self.flush_english_pool();
        let client = self.client.as_mut()?;
        Some(client.machine.handle(input, text))
    }

    /// 郵便番号の住所を、主スレッドの外で 1 回だけ問い合わせる（ADR 0032 §9.1）。返事は `finish` で受ける
    fn request_postal_lookup(&mut self, request: PostalLookupRequest) {
        self.background.spawn(&self.engine, Job::Postal(request));
    }

    /// ショートコードの動作を返し、取り出したたまりを確定履歴に送る（今の `apply(EnglishTextOutcome)`）
    fn apply_english(&mut self, outcome: EnglishTextOutcome) -> Vec<ShellAction> {
        for pooled in &outcome.flushed {
            self.record_english_text(pooled);
        }
        outcome.actions
    }

    fn record_english_text(&self, text: &str) {
        if let Some(client) = &self.client {
            client.machine.record_english_text(text);
        }
    }

    fn record_english_episode(&mut self) {
        let finished = self.english.take_finished_diag();
        if let Some(client) = &self.client {
            client.machine.record_english_episode(finished);
        }
    }

    /// ショートコード（と郵便番号）の表示があれば、今の表示のまま確定する
    fn flush_english_shortcode(&mut self, end: DiagEnd) -> Vec<ShellAction> {
        let mut actions = Vec::new();
        // 郵便番号をためていれば、打った文字のまま確定する（ショートコードとは同時に状態に入らない）
        if let Some(outcome) = self.english.flush_postal() {
            actions.extend(self.apply_english(outcome));
        }
        let now = self.now();
        if let Some(outcome) = self.english.flush_shortcode(end, now) {
            actions.extend(self.apply_english(outcome));
            self.record_english_episode();
        }
        actions
    }

    fn flush_english_pool(&mut self) {
        if let Some(pooled) = self.english.flush() {
            self.record_english_text(&pooled);
        }
    }

    // MARK: モード（今の `selectMode`・`leaveCurrentMode`・`recordModeSwitch`）

    /// キー割り当てでモードを替える。今のモードの表示を確定してから、OS に選んでもらう（`take_mode_request`）
    fn select_mode(&mut self, target: InputMode) -> Vec<ShellAction> {
        self.key_switch_requested_at = Some(self.clock.now());
        let actions = self.leave_current_mode();
        self.mode_request = Some(target);
        actions
    }

    /// モードを替える前の後始末。合成中なら今の表示のまま確定し、候補ウィンドウを閉じ、英語の文脈のたまりを送る
    fn leave_current_mode(&mut self) -> Vec<ShellAction> {
        let mut actions = self
            .client
            .as_mut()
            .map(|client| client.machine.commit_composition(DiagEnd::ModeSwitch))
            .unwrap_or_default();
        actions.extend(self.flush_english_shortcode(DiagEnd::ModeSwitch));
        actions.extend(self.hide_panel());
        self.flush_english_pool();
        actions
    }

    fn record_mode_switch(&mut self, from: InputMode, to: InputMode) {
        let requested = self.key_switch_requested_at.take();
        let now = self.clock.now();
        let via_key = requested.is_some_and(|at| now.saturating_sub(at) < KEY_SWITCH_WINDOW);
        if let Some(client) = &self.client {
            client.machine.record_mode_switch(from, to, via_key);
        }
    }

    // MARK: 設定（今の `syncSettings`・`settingsDidChange`・`restartEngine`）

    /// キー割り当てを今の設定に合わせる（設定の版が進んだときだけ読み直す）
    fn refresh_keymap(&mut self) {
        let version = self.settings.version();
        if version != self.keymap_version {
            self.keymap = load_keymap(&self.config.resources_dir, self.settings.snapshot());
            self.keymap_version = version;
        }
    }

    /// 設定の版が進んでいたら、idle のときだけ状態機械に反映する（表示中の文字列を変えない）
    fn sync_settings(&mut self) {
        let latest = self.settings.version();
        if self
            .client
            .as_ref()
            .is_none_or(|client| client.applied_settings_version == latest)
        {
            return;
        }
        self.refresh_keymap();
        let settings = self.settings.shared();
        let keymap = Arc::clone(&self.keymap);
        let romaji = self.romaji;
        let Some(client) = &mut self.client else {
            return;
        };
        let composer = (settings.input_method != client.machine.settings().input_method)
            .then(|| Composer::new(settings.input_method, romaji));
        if client.machine.reconfigure(settings, keymap, composer) {
            client.applied_settings_version = latest;
        }
    }

    fn reload_settings(&mut self) {
        if let Some(change) = self.settings.reload_if_changed() {
            self.settings_did_change(&change);
        }
    }

    fn settings_did_change(&mut self, change: &SettingsChange) {
        if engine_settings_differ(&change.old, &change.new) {
            self.restart_engine(change.new.engine_lm);
        }
        // つながっている設定アプリは取り直す（メニュー、設定アプリ、手の編集で変わったとき）
        self.pending_bridge.extend(self.bridge.settings_changed());
    }

    /// `engine.*` が変わったとき: エンジンに保存して終了してもらい（`SETTINGS_CHANGED`）、続けて `Hello` で起こし直す
    fn restart_engine(&mut self, lm_enabled: bool) {
        self.lm_state = Some(if lm_enabled {
            EngineLmState::Loading
        } else {
            EngineLmState::Disabled
        });
        self.poll_generation += 1;
        self.refreshing = true;
        self.background.spawn(
            &self.engine,
            Job::EngineInfo {
                attempt: 0,
                generation: self.poll_generation,
                shutdown_first: true,
            },
        );
    }

    fn write_settings(&mut self, key: &str, value: &str) {
        let entry = SettingsEntry {
            key: key.to_string(),
            value: value.to_string(),
        };
        // 失敗（書けない、検査で断られた）は今の設定のまま。ログは OS 側（種類だけ）
        if let Ok(Some(change)) = self.settings.update(vec![entry], Vec::new()) {
            self.settings_did_change(&change);
        }
    }

    // MARK: 設定アプリとの接続（今の `BridgeServer` の判断）

    fn bridge_frame(&mut self, conn: ConnId, frame: &[u8]) -> Vec<ShellAction> {
        let (mut actions, work) = self.bridge.receive(
            conn,
            frame,
            &self.config.shell_version,
            self.settings.snapshot(),
        );
        match work {
            None => {}
            Some(BridgeWork::Relay { conn, job }) => {
                self.background
                    .spawn(&self.engine, Job::Relay { conn, job });
            }
            Some(BridgeWork::Update {
                conn,
                request_id,
                update,
            }) => actions.push(self.update_from_bridge(conn, request_id, update)),
        }
        actions
    }

    /// 設定アプリからの書き換え（今の `BridgeServer.apply(UpdateSettings)`）。通知（`SettingsChanged`）は `finish` で送る
    fn update_from_bridge(
        &mut self,
        conn: ConnId,
        request_id: u64,
        update: crate::bridge::proto::UpdateSettings,
    ) -> ShellAction {
        if let Some(key) = bridge_link::invalid_key(&update) {
            return bridge_link::update_reply(conn, request_id, Err(Some(&key)));
        }
        let set = update
            .set
            .into_iter()
            .map(|entry| SettingsEntry {
                key: entry.key,
                value: entry.value,
            })
            .collect();
        match self.settings.update(set, update.reset) {
            Ok(change) => {
                if let Some(change) = change {
                    self.settings_did_change(&change);
                }
                bridge_link::update_reply(conn, request_id, Ok(self.settings.snapshot()))
            }
            // 書いたあとの警告が増える（今の Swift と同じく、キー名は入れない）
            Err(UpdateFailed::Rejected) => {
                bridge_link::update_reply(conn, request_id, Err(Some("")))
            }
            Err(UpdateFailed::WriteFailed) => {
                bridge_link::update_reply(conn, request_id, Err(None))
            }
        }
    }

    // MARK: 入力メニュー（今の `AppContext.perform`・`menuState`）

    fn perform(&mut self, command: MenuCommand) -> Vec<ShellAction> {
        match command {
            MenuCommand::SelectKeymapPreset { preset } => {
                self.write_settings(crate::settings::KEY_KEYMAP_PRESET, preset_name(preset));
            }
            MenuCommand::SelectInputMethod { method } => {
                self.write_settings(crate::settings::KEY_INPUT_METHOD, input_method_name(method));
            }
            MenuCommand::SetLm { enabled } => {
                let value = if enabled {
                    crate::settings::VALUE_ON
                } else {
                    crate::settings::VALUE_OFF
                };
                self.write_settings(crate::settings::KEY_ENGINE_LM, value);
            }
            MenuCommand::SetProfileDomain { id, included } => {
                // エンジンを呼ぶので主スレッドの外で。最新の状態を取り直してから 1 つだけ変える
                self.background
                    .spawn(&self.engine, Job::SetProfileDomain { id, included });
            }
            MenuCommand::OpenSettings { tab } => {
                // すでに動いていてつながっているなら、開くタブをブリッジでも伝える（起動の引数は動作中のアプリに届かない）
                let mut actions: Vec<ShellAction> = tab
                    .and_then(|tab| self.bridge.open_tab(tab))
                    .into_iter()
                    .collect();
                actions.push(ShellAction::OpenSettings { tab });
                return actions;
            }
        }
        Vec::new()
    }

    fn menu_model(&self) -> MenuModel {
        let settings = self.settings.snapshot();
        let mut state = InputMenuState::new(
            settings.keymap_preset,
            settings.input_method,
            settings.engine_lm,
        );
        state.lm_state = self.lm_state;
        state.profile = self.profile.clone();
        crate::menu::build(&state)
    }

    // MARK: エンジンの状態と絵文字の表（今の `refreshEngineInfo`・`refreshEmojiTable`）

    fn refresh_emoji_table(&mut self) {
        if self.emoji.fetching {
            return;
        }
        self.emoji.fetching = true;
        self.background.spawn(&self.engine, Job::EmojiTable);
    }

    fn refresh_engine_info(&mut self) {
        if self.refreshing {
            return;
        }
        self.refreshing = true;
        self.background.spawn(
            &self.engine,
            Job::EngineInfo {
                attempt: 0,
                generation: self.poll_generation,
                shutdown_first: false,
            },
        );
    }

    /// 絵文字の表の版が進んでいたら、状態機械と英語のモードに渡す（エンジンは呼ばない）
    fn sync_emoji_table(&mut self) {
        let version = self.emoji.version;
        let table = Arc::clone(&self.emoji.table);
        let Some(client) = &mut self.client else {
            return;
        };
        if client.applied_emoji_version == version {
            return;
        }
        client.applied_emoji_version = version;
        client.machine.set_shortcode_table(Arc::clone(&table));
        self.english.set_shortcode_table(table);
    }

    fn timer_fired(&mut self, id: TimerId) -> Vec<ShellAction> {
        match id {
            LM_POLL_TIMER => {
                // 起こし直したあとの古い取り直しは捨てる（`generation`）
                if let Some((attempt, generation)) = self.pending_lm_poll.take()
                    && generation == self.poll_generation
                {
                    self.background.spawn(
                        &self.engine,
                        Job::EngineInfo {
                            attempt,
                            generation,
                            shutdown_first: false,
                        },
                    );
                }
            }
            BACKGROUND_POLL_TIMER => self.background_poll_scheduled = false,
            _ => {}
        }
        Vec::new()
    }

    // MARK: 毎回の終わり

    /// 主スレッドの外の結果を受け取り、メニューの中身が変わっていれば知らせ、見回りのタイマーを頼む
    fn finish(&mut self, actions: &mut Vec<ShellAction>) {
        for result in self.background.drain() {
            actions.extend(self.accept(result));
        }
        // 候補ウィンドウを閉じたら上下を既定に戻す（今の `hidePanel` の `setPanelSide(.below)`。閉じるのは OS 側）
        if actions.contains(&ShellAction::HideCandidates) {
            self.panel_hidden();
        }
        actions.append(&mut self.pending_bridge);
        let inputs = (self.settings.version(), self.lm_state, self.profile_version);
        if self.menu_inputs != Some(inputs) {
            self.menu_inputs = Some(inputs);
            let menu = self.menu_model();
            if self.last_menu.as_ref() != Some(&menu) {
                self.last_menu = Some(menu.clone());
                actions.push(ShellAction::ShowMenu(menu));
            }
        }
        if self.background.has_outstanding() && !self.background_poll_scheduled {
            self.background_poll_scheduled = true;
            actions.push(ShellAction::ScheduleTimer {
                id: BACKGROUND_POLL_TIMER,
                after_ms: BACKGROUND_POLL_INTERVAL_MS,
            });
        }
    }

    fn accept(&mut self, result: JobResult) -> Vec<ShellAction> {
        match result {
            JobResult::EmojiTable(reply) => {
                self.emoji.fetching = false;
                // 取れなかったときは今の表のまま（変換を止めない）
                if let Some(reply) = reply
                    && (reply.data_label != self.emoji.data_label || self.emoji.table.is_empty())
                {
                    let entries = reply
                        .entries
                        .into_iter()
                        .map(|entry| ShortcodeEntry::new(entry.name, entry.emoji))
                        .collect();
                    self.emoji.table = Arc::new(ShortcodeTable::new(entries));
                    self.emoji.data_label = reply.data_label;
                    self.emoji.version += 1;
                }
                Vec::new()
            }
            JobResult::EngineInfo {
                attempt,
                generation,
                lm,
                profile,
            } => {
                if generation != self.poll_generation {
                    return Vec::new();
                }
                if let Some(lm) = lm {
                    self.lm_state = Some(lm);
                    if let Some(profile) = profile {
                        self.profile = Some(profile);
                        self.profile_version += 1;
                    }
                    if lm == EngineLmState::Loading && attempt < LM_POLL_LIMIT {
                        self.pending_lm_poll = Some((attempt + 1, generation));
                        return vec![ShellAction::ScheduleTimer {
                            id: LM_POLL_TIMER,
                            after_ms: LM_POLL_INTERVAL_MS,
                        }];
                    }
                }
                self.refreshing = false;
                Vec::new()
            }
            JobResult::Postal { request, addresses } => {
                // パスワード欄に移っていたら何もしない
                let secure = self
                    .client
                    .as_ref()
                    .is_some_and(|client| client.machine.session().secure);
                if secure {
                    return Vec::new();
                }
                match self.english.postal_lookup_arrived(&request, &addresses) {
                    Some(outcome) => self.apply_english(outcome),
                    None => Vec::new(),
                }
            }
            JobResult::Profile(domains) => {
                if let Some(domains) = domains {
                    self.profile = Some(domains);
                    self.profile_version += 1;
                }
                Vec::new()
            }
            JobResult::Relay { conn, frame } => {
                self.bridge.relay_done(conn, &frame).into_iter().collect()
            }
        }
    }

    fn now(&self) -> u64 {
        u64::try_from(self.clock.now().as_nanos()).unwrap_or(u64::MAX)
    }
}

/// キーを受け止めたことを表す列（動作がなければ `Consume`）。
fn consumed(mut actions: Vec<ShellAction>) -> Vec<ShellAction> {
    if actions.is_empty() {
        actions.push(ShellAction::Consume);
    }
    actions
}

fn input_method_name(method: InputMethodKind) -> &'static str {
    match method {
        InputMethodKind::Romaji => "romaji",
        InputMethodKind::Kana => "kana",
    }
}

/// JIS の番号で届いたキーを、OS の配列の文字のキーにする（`layout_text`。空なら `Other`）。
fn layout_key(input: &KeyInput, text: &str) -> KeyInput {
    KeyInput {
        key: if text.is_empty() {
            KeyKind::Other
        } else {
            KeyKind::Character {
                text: text.to_string(),
            }
        },
        layout_text: None,
        ..input.clone()
    }
}

/// 日本語のモードのキー（今の `KeyCodes.keyInput`）。かな入力で、Ctrl / Option / Command がないときだけ JIS のキーにし、
/// ほかは OS の配列の文字で読む（`layout_text` があるとき。ないとき（シナリオ）は届いたまま）。
fn japanese_key(input: &KeyInput, method: InputMethodKind) -> KeyInput {
    let (KeyKind::Jis { .. }, Some(text)) = (&input.key, &input.layout_text) else {
        return input.clone();
    };
    let modifiers = input.modifiers;
    let chord = modifiers.control || modifiers.option || modifiers.command;
    if method == InputMethodKind::Kana && !chord {
        return input.clone();
    }
    layout_key(input, text)
}

/// 英語のモードは、かな入力の設定でも JIS のキーにしない（文字で読む。今の `KeyCodes.keyInput(…, inputMethod: .romaji)`）。
///
/// OS の配列の文字（`layout_text`）があればそれを使う。ないとき（シナリオ）は刻印の ASCII に直す
/// （Shift 付きの英字は大文字。刻印のないキーは `Other`）。
fn english_key(input: &KeyInput) -> KeyInput {
    let KeyKind::Jis { code } = input.key else {
        return input.clone();
    };
    if let Some(text) = &input.layout_text {
        return layout_key(input, text);
    }
    let shift = input.modifiers.shift;
    let character = jis_ascii::character_for_jis_key(code, shift).or_else(|| {
        shift
            .then(|| jis_ascii::character_for_jis_key(code, false))
            .flatten()
            .filter(char::is_ascii_lowercase)
            .map(|c| c.to_ascii_uppercase())
    });
    KeyInput {
        key: character.map_or(KeyKind::Other, |c| KeyKind::Character {
            text: c.to_string(),
        }),
        ..input.clone()
    }
}
