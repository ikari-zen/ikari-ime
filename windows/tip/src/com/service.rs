//! テキストサービス本体（スレッドごとに 1 つ。TSF が `IClassFactory::CreateInstance` で作り、`ActivateEx` で始める）。
//!
//! OS の通知を `ShellEvent` にして `Shell::dispatch` に渡し、返った動作を `ops` で `TsfOp` にして `edit::Exec` で
//! 実行する。判断は持たない（ADR 0040 §2.8）。
//!
//! # キー（`ITfKeyEventSink`）
//! - TSF は、キーを食べるかを `OnTestKeyDown` で先に聞き、食べると言ったら `OnKeyDown` を呼ぶ。`shell-core` には
//!   「試すだけ」の口がない（`dispatch` は状態を進める）ので、最初に来た方（ふつうは `OnTestKeyDown`）で
//!   同期の編集セッションを取り、`dispatch` と `TsfOp` の実行をまとめて行う。食べたときは、そのキー（VK）を覚え、続く `OnKeyDown` は実行せずに「食べた」を返す。食べなかったキーは覚えない（`OnKeyDown` は来ない）。
//!   覚えたキーは、ほかのキー、キーを離したとき、フォーカスの移動で忘れる。
//! - 同期の編集セッションが取れないとき（読み取り専用の欄など）は、`dispatch` を呼ばずにキーをアプリに渡す。
//!
//! # 入力先（`ShellEvent::Activate` / `Deactivate`）
//! - 文書のフォーカス（`ITfThreadMgrEventSink::OnSetFocus`）が移ったら、`Deactivate` と `Activate` を送る。
//!   前の文書に合成が残っていれば、その文字列のまま終える（非同期の編集セッション）。`shell-core` の確定の動作は、
//!   文書のない `dispatch` なので実行しない（表示中の文字列がそのまま残る。macOS の確定と同じ文字列になる見込み。実機で確かめる）。
//! - アプリが合成を終わらせたら（`OnCompositionTerminated`）、`CommitComposition` を文書なしで送る（文字列は文書に
//!   残っているので、もう一度入れない）。
//!
//! # 寿命
//! `Shell` は `ActivateEx` で作り、`Deactivate` で捨てる（エンジンへの接続も閉じる）。`dispatch` の中で panic したら
//! `Shell` を捨て、次のキーで作り直す（この回のキーはアプリへ。macOS の殻と同じ）。

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use ikari_shell_core::english::InputMode;
use ikari_shell_core::{SelectedText, Shell, ShellEvent, SurroundingText, TextAccess, TimerId};
use windows::Win32::Foundation::{E_UNEXPECTED, LPARAM, WPARAM};
use windows::Win32::UI::TextServices::{
    ITfCompartmentEventSink, ITfCompartmentEventSink_Impl, ITfComposition, ITfCompositionSink,
    ITfCompositionSink_Impl, ITfContext, ITfDocumentMgr, ITfKeyEventSink, ITfKeyEventSink_Impl,
    ITfKeystrokeMgr, ITfSource, ITfTextInputProcessor_Impl, ITfTextInputProcessorEx,
    ITfTextInputProcessorEx_Impl, ITfThreadMgr, ITfThreadMgrEventSink, ITfThreadMgrEventSink_Impl,
    TF_ES_ASYNCDONTCARE, TF_ES_READWRITE, TF_ES_SYNC,
};
use windows_core::{BOOL, ComObject, GUID, IUnknownImpl, Interface, Ref, implement};

use super::dll::DllRef;
use super::edit::{self, Doc, Exec, ModeSink, Timers, TsfText};
use super::guard::guard;
use super::indicator::Indicator;
use super::{field, host, keyboard, log, timer};
use crate::keys;
use crate::latency::Kind;
use crate::mode::{self, ModeTracker};
use crate::ops::OpsTranslator;

/// `ActivateEx` の旗（使わない）
const NO_ACTIVATE_FLAGS: u32 = 0;

/// 食べたと答えたキー（`OnTestKeyDown` のあとの `OnKeyDown` を照合する）。
///
/// VK（`wParam`）だけで照合する。`lParam` は、アプリが `ITfKeystrokeMgr` を直接呼ぶときに `OnTestKeyDown` と
/// `OnKeyDown` で違う値を渡すことがありうる（実機で確かめる）。同じ VK を押し直す前には必ず離す通知
/// （`OnTestKeyUp`/`OnKeyUp`）が来て、そこで忘れるので、前の押下と取り違えない。
#[derive(Clone, Copy, PartialEq, Eq)]
struct KeyId {
    wparam: usize,
}

#[derive(Default)]
struct State {
    thread_mgr: Option<ITfThreadMgr>,
    client_id: u32,
    /// `ITfSource::AdviseSink`（スレッドの通知）の番号
    thread_mgr_cookie: Option<u32>,
    key_sink_advised: bool,
    shell: Option<Shell>,
    /// `Shell` を作れなかった（資源が読めない、など）。キーのたびに作り直さない（次の `ActivateEx` でもう一度試す）
    shell_unavailable: bool,
    translator: OpsTranslator,
    /// OS に表示したモード（変わったときだけ表示する。`Shell` を作り直したら捨てる）
    modes: ModeTracker,
    /// この TIP の合成
    composition: Option<ITfComposition>,
    /// 食べたと答えたキー
    eaten_key: Option<KeyId>,
    /// `Activate` を送ってある（`Deactivate` を送るまで）
    active: bool,
    /// 入力先のアプリの名前（実行ファイルの名前）
    client_name: String,
}

#[implement(
    ITfTextInputProcessorEx,
    ITfKeyEventSink,
    ITfCompositionSink,
    ITfThreadMgrEventSink,
    ITfCompartmentEventSink
)]
pub struct TextService {
    state: RefCell<State>,
    /// 入力モードの表示（`ActivateEx` で作り、`Deactivate` で外す）。`state` とは別にして、`dispatch` の最中
    /// （`state` を借りている間）の表示と、compartment の通知（書き込みの最中に来る）が、借用でぶつからないようにする
    indicator: RefCell<Option<Indicator>>,
    /// 自分の compartment の書き込みの間だけ立てる（そのときの通知を読み捨てる）
    writing_mode: Cell<bool>,
    _dll: DllRef,
}

impl TextService {
    pub fn new() -> Self {
        Self {
            state: RefCell::new(State::default()),
            indicator: RefCell::new(None),
            writing_mode: Cell::new(false),
            _dll: DllRef::new(),
        }
    }

    /// タイマーが来た（`timer::on_timer` から）。文書があればその編集セッションで、なければ文書なしで送る
    pub fn timer_fired(this: &ComObject<TextService>, id: TimerId) {
        let context = focused_context(this);
        run_event(
            this,
            context.as_ref(),
            ShellEvent::Timer { id },
            Mode::Async,
        );
    }
}

/// 文書のない `dispatch` に渡す読み手（何も取れない）
struct NoText;

impl TextAccess for NoText {
    fn surrounding(&mut self) -> Option<SurroundingText> {
        None
    }

    fn selection(&mut self) -> Option<SelectedText> {
        None
    }
}

/// `TsfOp` のタイマーを、このテキストサービス宛てにする
struct ServiceTimers<'a>(&'a ComObject<TextService>);

impl Timers for ServiceTimers<'_> {
    fn schedule(&self, id: TimerId, after_ms: u32) {
        timer::schedule(self.0, id, after_ms);
    }

    fn cancel(&self, id: TimerId) {
        timer::cancel(self.0, id);
    }
}

/// `TsfOp::ShowInputMode` を、このテキストサービスの表示（`indicator`）に渡す
struct ServiceModes<'a>(&'a ComObject<TextService>);

impl ModeSink for ServiceModes<'_> {
    fn show(&self, mode: InputMode) {
        let service = self.0;
        let Ok(indicator) = service.indicator.try_borrow() else {
            log::event("mode display re-entered");
            return;
        };
        if let Some(indicator) = indicator.as_ref() {
            // 書き込みで来る自分の通知を、`OnChange` が読み捨てる
            service.writing_mode.set(true);
            indicator.show(mode);
            service.writing_mode.set(false);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// 同期（キーの通知の中だけ）
    Sync,
    /// 非同期でよい（TSF が取れるときに呼ぶ。すぐ呼ばれることもある）
    Async,
}

/// フォーカスのある文書の、いちばん上の文脈
fn focused_context(this: &ComObject<TextService>) -> Option<ITfContext> {
    let thread_mgr = this.state.try_borrow().ok()?.thread_mgr.clone()?;
    // SAFETY: スレッドの管理者は生きている（`state` が持つ）
    let document = unsafe { thread_mgr.GetFocus() }.ok()?;
    // SAFETY: 文書は生きている
    unsafe { document.GetTop() }.ok()
}

/// 文書の中で（または文書なしで）`dispatch` と実行をする。戻り値はキーを食べたか。
/// `None` は実行しなかった（`Shell` がない、呼び直し、セッションが取れない、非同期でまだ、panic）。
fn run_event(
    this: &ComObject<TextService>,
    context: Option<&ITfContext>,
    event: ShellEvent,
    mode: Mode,
) -> Option<bool> {
    let Some(context) = context else {
        return dispatch_and_run(this, None, event);
    };
    let client_id = this.state.try_borrow().ok()?.client_id;
    let outcome = Rc::new(Cell::new(None));
    let body = {
        let this = this.clone();
        let context = context.clone();
        let outcome = Rc::clone(&outcome);
        move |ec: u32| outcome.set(dispatch_and_run(&this, Some((ec, &context)), event))
    };
    let flags = match mode {
        Mode::Sync => TF_ES_SYNC | TF_ES_READWRITE,
        Mode::Async => TF_ES_ASYNCDONTCARE | TF_ES_READWRITE,
    };
    match edit::request(context, client_id, flags, body) {
        Ok(_) => outcome.get(),
        Err(e) => {
            log::error("RequestEditSession failed", &e);
            None
        }
    }
}

/// `dispatch` して、返った動作を実行する。`doc` は編集セッションの `ec` と文脈
fn dispatch_and_run(
    this: &ComObject<TextService>,
    doc: Option<(u32, &ITfContext)>,
    event: ShellEvent,
) -> Option<bool> {
    let result = guard("panic in dispatch", None, || {
        Some(dispatch_and_run_inner(this, doc, event))
    });
    match result {
        Some(eaten) => eaten,
        None => {
            // panic のあとの `Shell` は信用しない。捨てて、次のキーで作り直す
            poison(this);
            None
        }
    }
}

fn dispatch_and_run_inner(
    this: &ComObject<TextService>,
    doc: Option<(u32, &ITfContext)>,
    event: ShellEvent,
) -> Option<bool> {
    // 呼び直し（`dispatch` の中から TSF が通知を返した、など）は、何もしない
    let Ok(mut state) = this.state.try_borrow_mut() else {
        log::event("re-entered while dispatching");
        return None;
    };
    let State {
        shell,
        translator,
        modes,
        composition,
        ..
    } = &mut *state;
    let shell = shell.as_mut()?;
    // 英数・かなキーは、OS の役として TIP がモードを替える（`mode`）
    let requested_mode = mode::requested_by_event(&event);
    // secure な欄（パスワード、PIN）かは、打鍵のたびに読んで `Key` に入れる（`shell-core` も打鍵のたびに見る。
    // 判断は `shell-core`: secure なら素通し、合成しない、周辺テキストを読まない、学習と診断に回さない）。
    // `shell-core` が周辺テキストを読むのは `dispatch` の中なので、読む前に伝える。読めなければ通常（`secure` の doc）
    let mut event = event;
    if let (ShellEvent::Key(input), Some((ec, context))) = (&mut event, doc) {
        input.secure = input.secure || field::is_secure(ec, context);
    }
    let mut actions = match doc {
        Some((ec, context)) => {
            let mut text = TsfText {
                ec,
                context,
                composition: composition.as_ref(),
            };
            shell.dispatch(event, &mut text)
        }
        None => shell.dispatch(event, &mut NoText),
    };
    // `shell-core` が頼んだモード（キー割り当て）と、英数・かなキーの行き先を、OS からの通知として返す
    actions.extend(mode::settle(shell, requested_mode));
    let mut ops = translator.translate_all(&actions);
    // モードが変わっていれば（または最初なら）表示する
    ops.extend(modes.sync(shell.input_mode()));
    let sink: ITfCompositionSink = this.to_interface();
    let timers = ServiceTimers(this);
    let mode_sink = ServiceModes(this);
    let mut exec = Exec {
        doc: doc.map(|(ec, context)| Doc {
            ec,
            context,
            sink: &sink,
        }),
        composition,
        timers: &timers,
        modes: &mode_sink,
    };
    Some(exec.run(&ops))
}

fn poison(this: &ComObject<TextService>) {
    let shell = match this.state.try_borrow_mut() {
        Ok(mut state) => {
            state.translator.reset();
            // 作り直した `Shell` は日本語から始まる。次の表示で OS の値を合わせる
            state.modes.reset();
            state.active = false;
            state.eaten_key = None;
            state.shell.take()
        }
        Err(_) => None,
    };
    timer::cancel_all(this);
    // 錠の外で落とす（エンジンへの接続を閉じる）
    drop(shell);
}

/// `Shell` がなければ作り、入力先を知らせていなければ `Activate` を送る
fn ensure_active(this: &ComObject<TextService>) -> bool {
    let (needs_shell, active, name) = match this.state.try_borrow() {
        Ok(state) => (
            state.shell.is_none() && state.thread_mgr.is_some() && !state.shell_unavailable,
            state.active,
            state.client_name.clone(),
        ),
        Err(_) => return false,
    };
    if needs_shell {
        let shell = host::create_shell();
        match this.state.try_borrow_mut() {
            Ok(mut state) => {
                state.shell_unavailable = shell.is_none();
                state.shell = shell;
                // 新しい `Shell` は日本語から始まる。次の表示で OS の値を合わせる
                state.modes.reset();
            }
            Err(_) => return false,
        }
    }
    if !active {
        let activated = dispatch_and_run(
            this,
            None,
            ShellEvent::Activate {
                bundle_id: name,
                // 編集セッションがなく、入力スコープは読めない。secure は打鍵のたびに `Key` で伝える
                // （`shell-core` は打鍵のたびに見る。`Activate` の時点では周辺テキストも読まない）
                secure: false,
            },
        );
        if activated.is_none() {
            return false;
        }
        if let Ok(mut state) = this.state.try_borrow_mut() {
            state.active = true;
        }
    }
    this.state
        .try_borrow()
        .map(|state| state.shell.is_some())
        .unwrap_or(false)
}

/// 入力先を離れる（`Deactivate` を送る）。合成が残っていれば、その文字列のまま終える
fn leave_client(this: &ComObject<TextService>) {
    let (composition, active) = match this.state.try_borrow_mut() {
        Ok(mut state) => {
            state.eaten_key = None;
            state.translator.reset();
            let active = std::mem::take(&mut state.active);
            (state.composition.take(), active)
        }
        Err(_) => return,
    };
    if let Some(composition) = composition {
        end_composition_async(this, composition);
    }
    if active {
        dispatch_and_run(this, None, ShellEvent::Deactivate);
    }
}

/// 合成を、いまの文字列のまま終える（文書の編集セッションで。取れるときに TSF が呼ぶ）
fn end_composition_async(this: &ComObject<TextService>, composition: ITfComposition) {
    let context = (|| {
        // SAFETY: 合成は生きている
        let range = unsafe { composition.GetRange() }.ok()?;
        // SAFETY: 範囲は生きている
        unsafe { range.GetContext() }.ok()
    })();
    let (Some(context), Ok(client_id)) = (
        context,
        this.state.try_borrow().map(|state| state.client_id),
    ) else {
        return;
    };
    let body = move |ec: u32| {
        // SAFETY: `ec` はこのセッションのもの。合成は生きている（`body` が持つ）
        if let Err(e) = unsafe { composition.EndComposition(ec) } {
            log::error("EndComposition (focus change) failed", &e);
        }
    };
    if let Err(e) = edit::request(
        &context,
        client_id,
        TF_ES_ASYNCDONTCARE | TF_ES_READWRITE,
        body,
    ) {
        log::error("RequestEditSession (focus change) failed", &e);
    }
}

impl TextService_Impl {
    fn this(&self) -> ComObject<TextService> {
        self.to_object()
    }

    fn activate(&self, thread_mgr: &ITfThreadMgr, client_id: u32) -> windows_core::Result<()> {
        {
            let mut state = self.state.try_borrow_mut().map_err(|_| E_UNEXPECTED)?;
            state.thread_mgr = Some(thread_mgr.clone());
            state.client_id = client_id;
            state.client_name = host::client_name();
        }
        let source: ITfSource = thread_mgr.cast()?;
        let sink: ITfThreadMgrEventSink = self.to_interface();
        // SAFETY: `sink` は生きていて、TSF が AddRef して持つ
        let cookie = unsafe { source.AdviseSink(&ITfThreadMgrEventSink::IID, &sink) }?;
        self.state
            .try_borrow_mut()
            .map_err(|_| E_UNEXPECTED)?
            .thread_mgr_cookie = Some(cookie);

        let keystroke: ITfKeystrokeMgr = thread_mgr.cast()?;
        let key_sink: ITfKeyEventSink = self.to_interface();
        // SAFETY: 同上。前景の TIP としてキーを受ける
        unsafe { keystroke.AdviseKeyEventSink(client_id, &key_sink, true) }?;
        self.state
            .try_borrow_mut()
            .map_err(|_| E_UNEXPECTED)?
            .key_sink_advised = true;

        // 入力モードの表示（compartment と言語バーのボタン）。なくても入力はできるので、失敗しても続ける
        let compartment_sink: ITfCompartmentEventSink = self.to_interface();
        match Indicator::install(thread_mgr, client_id, &compartment_sink) {
            Ok(indicator) => {
                *self.indicator.try_borrow_mut().map_err(|_| E_UNEXPECTED)? = Some(indicator)
            }
            Err(e) => log::error("mode indicator install failed", &e),
        }

        let shell = host::create_shell();
        {
            let mut state = self.state.try_borrow_mut().map_err(|_| E_UNEXPECTED)?;
            state.shell_unavailable = shell.is_none();
            state.shell = shell;
            // 新しい `Shell` は日本語から始まる。次の表示で OS の値を合わせる
            state.modes.reset();
        }
        // 文書にフォーカスがあれば、入力先を知らせる
        if focused_context(&self.this()).is_some() {
            ensure_active(&self.this());
        }
        Ok(())
    }

    fn deactivate(&self) {
        // 測定の残りを書き出す（オフなら何もしない）
        host::meter().flush_all();
        let this = self.this();
        leave_client(&this);
        timer::cancel_all(&this);
        // 通知の登録とボタンを外す（錠の外で。外す間に来る通知は、`indicator` がないので読み捨てられる）
        let indicator = self
            .indicator
            .try_borrow_mut()
            .ok()
            .and_then(|mut slot| slot.take());
        if let Some(mut indicator) = indicator {
            indicator.remove();
        }
        let (thread_mgr, cookie, key_sink, client_id, shell) = match self.state.try_borrow_mut() {
            Ok(mut state) => (
                state.thread_mgr.take(),
                state.thread_mgr_cookie.take(),
                std::mem::take(&mut state.key_sink_advised),
                state.client_id,
                state.shell.take(),
            ),
            Err(_) => return,
        };
        if let Some(thread_mgr) = thread_mgr {
            if let (Some(cookie), Ok(source)) = (cookie, thread_mgr.cast::<ITfSource>()) {
                // SAFETY: `AdviseSink` が返した番号
                if let Err(e) = unsafe { source.UnadviseSink(cookie) } {
                    log::error("UnadviseSink failed", &e);
                }
            }
            if key_sink && let Ok(keystroke) = thread_mgr.cast::<ITfKeystrokeMgr>() {
                // SAFETY: `ActivateEx` で受けた番号
                if let Err(e) = unsafe { keystroke.UnadviseKeyEventSink(client_id) } {
                    log::error("UnadviseKeyEventSink failed", &e);
                }
            }
        }
        // 錠の外で落とす（エンジンへの接続を閉じる）
        drop(shell);
    }

    fn forget_eaten_key(&self) {
        if let Ok(mut state) = self.state.try_borrow_mut() {
            state.eaten_key = None;
        }
    }

    /// キーの押下。`test` は `OnTestKeyDown`。戻り値はキーを食べたか
    fn key_down(
        &self,
        context: Option<&ITfContext>,
        wparam: WPARAM,
        lparam: LPARAM,
        test: bool,
    ) -> bool {
        let id = KeyId { wparam: wparam.0 };
        {
            let Ok(mut state) = self.state.try_borrow_mut() else {
                return false;
            };
            match state.eaten_key {
                // `OnTestKeyDown` で処理済み。`OnKeyDown` で忘れ、`OnTestKeyDown` の繰り返しでは覚えたまま
                Some(eaten) if eaten == id => {
                    if !test {
                        state.eaten_key = None;
                    }
                    return true;
                }
                _ => state.eaten_key = None,
            }
        }
        let Some(context) = context else {
            return false;
        };
        let this = self.this();
        if !ensure_active(&this) {
            return false;
        }
        // 往復時間の測定（S1）。キーを受けてから、返った操作を実行し終わるまで。オフなら分岐 1 つ
        let meter = host::meter();
        let stamp = meter.start();
        let Some(press) = keyboard::read(wparam, lparam) else {
            return false;
        };
        // 修飾キーだけの押下は送らない（アプリへ）
        let Some(event) = keys::key_event(&press.raw()) else {
            return false;
        };
        let outcome = run_event(&this, Some(context), event, Mode::Sync);
        meter.finish(Kind::Key, stamp, outcome.is_some());
        let eaten = outcome.unwrap_or(false);
        if eaten
            && test
            && let Ok(mut state) = self.state.try_borrow_mut()
        {
            state.eaten_key = Some(id);
        }
        // 書き出しは、キーの処理が終わったあと（測った時間に入れない）
        meter.flush_if_due();
        eaten
    }
}

impl ITfTextInputProcessor_Impl for TextService_Impl {
    fn Activate(&self, thread_mgr: Ref<ITfThreadMgr>, client_id: u32) -> windows_core::Result<()> {
        ITfTextInputProcessorEx_Impl::ActivateEx(self, thread_mgr, client_id, NO_ACTIVATE_FLAGS)
    }

    fn Deactivate(&self) -> windows_core::Result<()> {
        guard("panic in Deactivate", Ok(()), || {
            self.deactivate();
            Ok(())
        })
    }
}

impl ITfTextInputProcessorEx_Impl for TextService_Impl {
    fn ActivateEx(
        &self,
        thread_mgr: Ref<ITfThreadMgr>,
        client_id: u32,
        _flags: u32,
    ) -> windows_core::Result<()> {
        guard("panic in ActivateEx", Err(E_UNEXPECTED.into()), || {
            let thread_mgr = thread_mgr.ok()?;
            let result = self.activate(thread_mgr, client_id);
            if let Err(e) = &result {
                log::error("ActivateEx failed", e);
                self.deactivate();
            }
            result
        })
    }
}

impl ITfKeyEventSink_Impl for TextService_Impl {
    fn OnSetFocus(&self, _foreground: BOOL) -> windows_core::Result<()> {
        guard("panic in OnSetFocus (key)", Ok(()), || {
            self.forget_eaten_key();
            Ok(())
        })
    }

    fn OnTestKeyDown(
        &self,
        context: Ref<ITfContext>,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> windows_core::Result<BOOL> {
        guard("panic in OnTestKeyDown", Ok(BOOL::from(false)), || {
            Ok(BOOL::from(self.key_down(
                context.as_ref(),
                wparam,
                lparam,
                true,
            )))
        })
    }

    fn OnTestKeyUp(
        &self,
        _context: Ref<ITfContext>,
        _wparam: WPARAM,
        _lparam: LPARAM,
    ) -> windows_core::Result<BOOL> {
        guard("panic in OnTestKeyUp", Ok(BOOL::from(false)), || {
            self.forget_eaten_key();
            Ok(BOOL::from(false))
        })
    }

    fn OnKeyDown(
        &self,
        context: Ref<ITfContext>,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> windows_core::Result<BOOL> {
        guard("panic in OnKeyDown", Ok(BOOL::from(false)), || {
            Ok(BOOL::from(self.key_down(
                context.as_ref(),
                wparam,
                lparam,
                false,
            )))
        })
    }

    fn OnKeyUp(
        &self,
        _context: Ref<ITfContext>,
        _wparam: WPARAM,
        _lparam: LPARAM,
    ) -> windows_core::Result<BOOL> {
        guard("panic in OnKeyUp", Ok(BOOL::from(false)), || {
            self.forget_eaten_key();
            Ok(BOOL::from(false))
        })
    }

    fn OnPreservedKey(
        &self,
        _context: Ref<ITfContext>,
        _guid: *const GUID,
    ) -> windows_core::Result<BOOL> {
        // 予約キー（`ITfKeystrokeMgr::PreserveKey`）は登録していない
        guard("panic in OnPreservedKey", Ok(BOOL::from(false)), || {
            Ok(BOOL::from(false))
        })
    }
}

impl ITfCompositionSink_Impl for TextService_Impl {
    fn OnCompositionTerminated(
        &self,
        _ec: u32,
        _composition: Ref<ITfComposition>,
    ) -> windows_core::Result<()> {
        guard("panic in OnCompositionTerminated", Ok(()), || {
            // この TIP の合成は 1 つだけ。アプリ（または TSF）が終えたので、文字列は文書に残っている
            match self.state.try_borrow_mut() {
                Ok(mut state) => {
                    state.composition = None;
                    state.translator.reset();
                    state.eaten_key = None;
                }
                Err(_) => return Ok(()),
            }
            // 状態機械を確定の後の状態にする（文書には書かない。書くと同じ文字列が 2 回入る）
            dispatch_and_run(&self.this(), None, ShellEvent::CommitComposition);
            Ok(())
        })
    }
}

impl ITfThreadMgrEventSink_Impl for TextService_Impl {
    fn OnInitDocumentMgr(&self, _document: Ref<ITfDocumentMgr>) -> windows_core::Result<()> {
        Ok(())
    }

    fn OnUninitDocumentMgr(&self, _document: Ref<ITfDocumentMgr>) -> windows_core::Result<()> {
        Ok(())
    }

    fn OnSetFocus(
        &self,
        focus: Ref<ITfDocumentMgr>,
        _previous: Ref<ITfDocumentMgr>,
    ) -> windows_core::Result<()> {
        guard("panic in OnSetFocus (document)", Ok(()), || {
            let this = self.this();
            leave_client(&this);
            if focus.is_some() {
                ensure_active(&this);
            }
            Ok(())
        })
    }

    fn OnPushContext(&self, _context: Ref<ITfContext>) -> windows_core::Result<()> {
        Ok(())
    }

    fn OnPopContext(&self, _context: Ref<ITfContext>) -> windows_core::Result<()> {
        Ok(())
    }
}

impl ITfCompartmentEventSink_Impl for TextService_Impl {
    /// 入力モードの compartment が変わった（言語バーのクリック、IME のオン・オフ、ほかのアプリや OS の操作）。
    /// 値を `mode` の表で読み、`shell-core` に通知する。自分の書き込みの通知は読み捨てる
    fn OnChange(&self, guid: *const GUID) -> windows_core::Result<()> {
        guard("panic in OnChange (compartment)", Ok(()), || {
            if guid.is_null() || self.writing_mode.get() {
                return Ok(());
            }
            // SAFETY: null でない（上で確かめた）。TSF が渡した GUID への参照
            let guid = unsafe { *guid };
            let mode = match self.indicator.try_borrow() {
                Ok(indicator) => indicator
                    .as_ref()
                    .and_then(|indicator| indicator.read(&guid)),
                Err(_) => None,
            };
            let Some(mode) = mode else {
                return Ok(());
            };
            let this = self.this();
            let context = focused_context(&this);
            run_event(
                &this,
                context.as_ref(),
                ShellEvent::InputModeChanged {
                    mode_id: mode.mode_id().to_owned(),
                    // TODO(S8、W4): secure な入力欄
                    secure: false,
                },
                Mode::Async,
            );
            Ok(())
        })
    }
}

/// `ITfTextInputProcessorEx` を実装していることの確認（`IClassFactory::CreateInstance` が問い合わせる）
const _: fn(&ComObject<TextService>) -> ITfTextInputProcessorEx = |this| this.to_interface();
