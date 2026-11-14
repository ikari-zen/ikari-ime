//! 英語のモードの絵文字のショートコード（ADR 0027 §3.2、§3.3、§3.4 の英語の列。B48。今の `ShortcodeEnglish.swift`）。
//! `ShortcodeMachine`（英語のモード）への委譲と、文脈のたまりへの反映をここにまとめる。
//! コントローラは返り値の `actions` を実行し、`pass_key` ならキーをアプリに返すだけにする。
//! 英語の経路でエンジンを同期で呼ばない（表は殻のメモリのもの。ADR 0026 §9）。

use std::sync::Arc;

use super::{EnglishMode, marked_text};
use crate::diag::{
    DiagCommitInfo, DiagEnd, DiagEpisode, DiagFinishedEpisode, DiagMode::ShortcodeEn, DiagPrev,
    DiagStart,
};
use crate::panel::AnchorSource;
use crate::postal::PostalLookupRequest;
use crate::shortcode::{
    CANDIDATE_PAGE_SIZE, ShortcodeAfterCommit, ShortcodeChoosing, ShortcodeOutcome, ShortcodePhase,
    ShortcodeTable,
};
use crate::types::{
    CandidateItem, CandidateKind, CandidatePanelModel, KeyInput, KeyKind, SettingsSnapshot,
    ShellAction,
};

/// ショートコードの入りの判定に要る、打鍵のときの状況。
#[derive(Debug, Clone, Copy)]
pub struct EnglishShortcodeEnvironment<'a> {
    pub settings: &'a SettingsSnapshot,
    pub client_bundle_id: &'a str,
    pub secure: bool,
    pub is_repeat: bool,
}

/// 英語のモードの打鍵（`Text`）を処理した結果。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EnglishTextOutcome {
    /// 殻が実行する動作（変換中の文字列、確定、候補ウィンドウ）。`PassThrough` は含めない
    pub actions: Vec<ShellAction>,
    /// 今のキーをアプリに渡す（`handle` が false）。false ならキーを消費する
    pub pass_key: bool,
    /// 区切りで取り出した文脈のたまり。殻が `CommitText`（読みは空）としてエンジンに送る
    pub flushed: Vec<String>,
    /// 郵便番号の住所を問い合わせる（ADR 0032 §9.1）。殻が主スレッドの外で 1 回だけ頼む
    pub lookup: Option<PostalLookupRequest>,
}

impl EnglishMode {
    /// ショートコードの状態に入っている（変換中の文字列か候補を出している）。
    pub fn is_shortcode_active(&self) -> bool {
        self.shortcode.is_active()
    }

    /// 殻が受け取った絵文字の表を渡す。
    pub fn set_shortcode_table(&mut self, table: impl Into<Arc<ShortcodeTable>>) {
        self.shortcode.table = table.into();
    }

    /// 語の頭か（ADR 0027 §3.2）。直前の文字が、ない（区切りのあと。直前が分からない）、空白・改行、`(`、`[`、`{` のどれか。
    pub fn is_word_start(after: Option<char>) -> bool {
        match after {
            None => true,
            Some(last) => last.is_whitespace() || matches!(last, '(' | '[' | '{'),
        }
    }

    /// 入ってよいか（§3.2 の共通の条件と、英語の語の頭）。修飾キーと表が空かどうかは `ShortcodeMachine` が見る。
    fn can_enter_shortcode(
        environment: &EnglishShortcodeEnvironment<'_>,
        last: Option<char>,
    ) -> bool {
        environment.settings.emoji_shortcode
            && !environment.secure
            && !environment.is_repeat
            && !environment
                .settings
                .emoji_shortcode_exclude
                .iter()
                .any(|id| id == environment.client_bundle_id)
            && Self::is_word_start(last)
    }

    /// 英語のモードで、`mode.*` でも secure でもない打鍵。ショートコードの口を通し、関係なければ文脈のたまりに足して素通しにする
    /// （ADR 0026 §5.2 の 5-3）。確定した文字列（`:eyes:` でも絵文字でも）はたまりに足す。
    /// `now` は単調時計のナノ秒（診断ログ用）。
    pub fn handle_text(
        &mut self,
        input: &KeyInput,
        environment: &EnglishShortcodeEnvironment<'_>,
        now: u64,
    ) -> EnglishTextOutcome {
        if let Some(postal) = self.handle_postal(input, environment) {
            return postal;
        }
        let before_page = match self.shortcode.phase() {
            ShortcodePhase::Choosing(choosing) => Some(choosing.page()),
            _ => None,
        };
        let was_choice = before_page.is_some();
        let can_enter = Self::can_enter_shortcode(environment, self.last_character());
        self.shortcode.small_letters = environment.settings.emoji_small_letters;
        let result = self.shortcode.handle(input, can_enter);
        self.note_diag(input, &result.outcome, was_choice, now);

        let mut actions: Vec<ShellAction> = Vec::new();
        let mut committed: Option<String> = None;
        let mut pass_key = false;
        match result.outcome {
            ShortcodeOutcome::NotHandled => {
                return EnglishTextOutcome {
                    pass_key: true,
                    flushed: self.record(input).into_iter().collect(),
                    ..EnglishTextOutcome::default()
                };
            }
            ShortcodeOutcome::Typing {
                marked,
                hides_candidates,
            } => {
                actions.push(marked_text(&marked, false));
                if hides_candidates {
                    actions.push(ShellAction::HideCandidates);
                }
            }
            ShortcodeOutcome::Cancelled => {
                // `:` だけで BackSpace。見えていた `:` を消し、BackSpace は消費する
                actions.push(marked_text("", true));
            }
            ShortcodeOutcome::ShowChoices(choosing) => {
                actions.push(marked_text(choosing.marked(), true));
                actions.push(ShellAction::ShowCandidates(panel_model(&choosing)));
            }
            ShortcodeOutcome::MoveChoice(choosing) => {
                actions.push(marked_text(choosing.marked(), true));
                let model = panel_model(&choosing);
                actions.push(if before_page == Some(choosing.page()) {
                    ShellAction::MoveSelection(model)
                } else {
                    ShellAction::ShowCandidates(model)
                });
            }
            ShortcodeOutcome::Commit(commit) => {
                committed = Some(commit.text.clone());
                actions.push(ShellAction::Commit { text: commit.text });
                if was_choice {
                    actions.push(ShellAction::HideCandidates);
                }
                // 英語のモードの `after` は `Consume` と `PassKey` だけ（`ContinueWith` は日本語のモード用）
                pass_key = !matches!(commit.after, ShortcodeAfterCommit::Consume);
            }
            ShortcodeOutcome::Retype { literal, .. } => {
                // 過去のキーは渡し直せないので、見えていた文字列を確定し、今のキーだけアプリに渡す（§3.3）
                committed = Some(literal.clone());
                actions.push(ShellAction::Commit { text: literal });
                if was_choice {
                    actions.push(ShellAction::HideCandidates);
                }
                pass_key = true;
            }
        }
        let mut flushed: Vec<String> = Vec::new();
        if let Some(text) = committed
            && let Some(overflow) = self.append_committed(&text)
        {
            flushed.push(overflow);
        }
        if pass_key && let Some(key) = self.record(input) {
            flushed.push(key);
        }
        EnglishTextOutcome {
            actions,
            pass_key,
            flushed,
            lookup: None,
        }
    }

    /// モードの切り替え、secure、`deactivateServer`、クリック、`inputControllerWillClose` のとき、今の表示のまま確定する。
    /// 状態に入っていなければ `None`。たまりは取り出さない（呼ぶ側が続けて `flush` する）。
    /// `end` は診断ログの終わった理由（呼び出しの理由に合わせる）。
    pub fn flush_shortcode(&mut self, end: DiagEnd, now: u64) -> Option<EnglishTextOutcome> {
        let was_choice = matches!(self.shortcode.phase(), ShortcodePhase::Choosing(_));
        let commit = self.shortcode.flush()?;
        let prev = if was_choice {
            DiagPrev::ShortcodeChoice
        } else {
            DiagPrev::Shortcode
        };
        self.finish_diag(end, Some(prev), None, now);
        let mut actions = vec![ShellAction::Commit {
            text: commit.text.clone(),
        }];
        if was_choice {
            actions.push(ShellAction::HideCandidates);
        }
        Some(EnglishTextOutcome {
            actions,
            pass_key: false,
            flushed: self.append_committed(&commit.text).into_iter().collect(),
            lookup: None,
        })
    }

    // MARK: 診断ログ（ADR 0034 §4.2）。数えるのは整数の加算だけ。文字は持たない

    /// 候補ウィンドウを出した（コントローラが、位置を取ったあとに呼ぶ）。合成の最初の 1 回だけ覚える。
    pub fn note_panel_shown(&mut self, source: AnchorSource) {
        if let Some(episode) = &mut self.diag_episode {
            episode.note_panel(source);
        }
    }

    /// 終わった合成を取り出す（なければ `None`）。secure のときは呼ぶ側（`record_english_episode`）が捨てる。
    pub fn take_finished_diag(&mut self) -> Option<DiagFinishedEpisode> {
        self.finished_diag.take()
    }

    /// 数えている途中の合成（テスト用）。
    pub fn diag_episode(&self) -> Option<&DiagEpisode> {
        self.diag_episode.as_ref()
    }

    fn note_diag(
        &mut self,
        input: &KeyInput,
        outcome: &ShortcodeOutcome,
        was_choice: bool,
        now: u64,
    ) {
        match outcome {
            ShortcodeOutcome::NotHandled => return,
            ShortcodeOutcome::Retype { .. } => {
                // 打ち直しは 1 回の合成として数えない（日本語のモードと同じ）
                self.diag_episode = None;
                return;
            }
            _ => {}
        }
        let episode = self
            .diag_episode
            .get_or_insert_with(|| DiagEpisode::new(ShortcodeEn, DiagStart::Typing, now));
        episode.keys += 1;
        if input.key == KeyKind::Backspace {
            episode.bs += 1;
        }
        match outcome {
            ShortcodeOutcome::ShowChoices(_) => {
                if episode.converted_at.is_none() {
                    episode.converted_at = Some(now);
                    episode.keys_to_conv = Some(episode.keys);
                }
            }
            ShortcodeOutcome::MoveChoice(_) => episode.cand_moves += 1,
            ShortcodeOutcome::Cancelled => {
                self.finish_diag(DiagEnd::BsEmpty, Some(DiagPrev::Shortcode), None, now);
            }
            ShortcodeOutcome::Commit(_) => {
                let (end, commit) = if was_choice {
                    (
                        DiagEnd::ShortcodePick,
                        Some(DiagCommitInfo {
                            pick_kind: Some(self.shortcode.last_pick_kind()),
                            ..DiagCommitInfo::default()
                        }),
                    )
                } else {
                    (DiagEnd::ShortcodeLiteral, None)
                };
                self.finish_diag(end, None, commit, now);
            }
            ShortcodeOutcome::Typing { .. }
            | ShortcodeOutcome::NotHandled
            | ShortcodeOutcome::Retype { .. } => {}
        }
    }

    fn finish_diag(
        &mut self,
        end: DiagEnd,
        prev: Option<DiagPrev>,
        commit: Option<DiagCommitInfo>,
        now: u64,
    ) {
        let Some(episode) = self.diag_episode.take() else {
            return;
        };
        self.finished_diag = Some(DiagFinishedEpisode {
            episode,
            end,
            prev: if end.is_external() { prev } else { None },
            commit,
            ended_at: now,
        });
    }
}

/// 候補ウィンドウ。絵文字の行の右に名前（`label`）を薄く出し、「そのままの入力」の行は `Literal`。
/// 1 ページは 9 件（数字キーはそのページの番号）。
pub fn panel_model(choosing: &ShortcodeChoosing) -> CandidatePanelModel {
    let start = choosing.page() * CANDIDATE_PAGE_SIZE;
    let end = (start + CANDIDATE_PAGE_SIZE).min(choosing.candidates.len());
    let rows = &choosing.candidates[start..end];
    CandidatePanelModel {
        developer_note: String::new(),
        candidates: rows
            .iter()
            .map(|candidate| CandidateItem {
                surface: candidate.surface.clone(),
                annotation: String::new(),
                kind: if candidate.label.is_none() {
                    CandidateKind::Literal
                } else {
                    CandidateKind::Unspecified
                },
                label: candidate.label.clone().unwrap_or_default(),
            })
            .collect(),
        selected: u32::try_from(choosing.selected - start).unwrap_or(0),
        annotation: String::new(),
        reserved_rows: u32::try_from(
            choosing
                .candidates
                .len()
                .min(CANDIDATE_PAGE_SIZE)
                .max(rows.len()),
        )
        .unwrap_or(u32::MAX),
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::*;
    use crate::shortcode::{MAX_NAME_LENGTH, ShortcodeEntry};
    use crate::types::Modifiers;

    fn table() -> ShortcodeTable {
        ShortcodeTable::new(
            [
                ("eye", "👁"),
                ("eyes", "👀"),
                ("smile", "😄"),
                ("tada", "🎉"),
            ]
            .into_iter()
            .map(|(n, e)| ShortcodeEntry::new(n, e))
            .collect(),
        )
    }

    /// アプリの代わり。確定したテキスト、変換中の文字列、候補ウィンドウ、アプリに届いたキー
    struct App {
        mode: EnglishMode,
        settings: SettingsSnapshot,
        bundle_id: String,
        secure: bool,
        clock: u64,
        /// アプリのテキスト（素通しのキーと確定）
        text: String,
        marked: String,
        marked_underlined: bool,
        panel_rows: Vec<String>,
        panel_visible: bool,
        /// 区切りで送られた確定履歴（`CommitText` の surface。読みは空）
        sent: Vec<String>,
        /// Enter など、アプリに渡したが文字にならないキー
        passed_keys: Vec<KeyKind>,
    }

    const STEP: u64 = 5_000_000;

    impl App {
        fn new() -> Self {
            Self::with_table(table())
        }

        fn with_table(table: ShortcodeTable) -> Self {
            let mut mode = EnglishMode::new();
            mode.set_shortcode_table(table);
            let mut settings = default_settings();
            settings.emoji_shortcode = true;
            Self {
                mode,
                settings,
                bundle_id: "com.tinyspeck.slackmacgap".into(),
                secure: false,
                clock: 0,
                text: String::new(),
                marked: String::new(),
                marked_underlined: true,
                panel_rows: Vec::new(),
                panel_visible: false,
                sent: Vec::new(),
                passed_keys: Vec::new(),
            }
        }

        fn type_text(&mut self, string: &str) {
            for character in string.chars() {
                let kind = if character == ' ' {
                    KeyKind::Space
                } else {
                    KeyKind::Character {
                        text: character.to_string(),
                    }
                };
                self.press(kind);
            }
        }

        fn press(&mut self, kind: KeyKind) {
            self.press_with(kind, Modifiers::NONE, false);
        }

        fn press_with(&mut self, kind: KeyKind, modifiers: Modifiers, is_repeat: bool) {
            let input = key_with(kind.clone(), modifiers);
            self.clock += STEP;
            let environment = EnglishShortcodeEnvironment {
                settings: &self.settings,
                client_bundle_id: &self.bundle_id,
                secure: self.secure,
                is_repeat,
            };
            let outcome = self.mode.handle_text(&input, &environment, self.clock);
            self.apply(&outcome);
            if !outcome.pass_key {
                return;
            }
            match kind {
                KeyKind::Character { text }
                    if modifiers.is_empty() || modifiers == mods(true, false, false, false) =>
                {
                    self.text.push_str(&text);
                }
                KeyKind::Space => self.text.push(' '),
                other => self.passed_keys.push(other),
            }
        }

        fn apply(&mut self, outcome: &EnglishTextOutcome) {
            for action in &outcome.actions {
                match action {
                    ShellAction::SetMarkedText(mark) => {
                        self.marked = mark.text.clone();
                        self.marked_underlined = mark.underlined;
                    }
                    ShellAction::Commit { text } => {
                        self.text.push_str(text);
                        self.marked.clear();
                    }
                    ShellAction::ShowCandidates(model) | ShellAction::MoveSelection(model) => {
                        self.panel_visible = true;
                        self.panel_rows =
                            model.candidates.iter().map(|c| c.surface.clone()).collect();
                    }
                    ShellAction::HideCandidates => {
                        self.panel_visible = false;
                        self.panel_rows.clear();
                    }
                    _ => {}
                }
            }
            self.sent.extend(outcome.flushed.iter().cloned());
        }

        /// モードの切り替え、クリックなど
        fn flush(&mut self) {
            self.clock += STEP;
            if let Some(outcome) = self
                .mode
                .flush_shortcode(DiagEnd::CommitComposition, self.clock)
            {
                self.apply(&outcome);
            }
            if let Some(pooled) = self.mode.flush() {
                self.sent.push(pooled);
            }
        }

        fn rows(&self) -> Vec<&str> {
            self.panel_rows.iter().map(String::as_str).collect()
        }
    }

    fn ch_kind(text: &str) -> KeyKind {
        KeyKind::Character { text: text.into() }
    }

    // 語の頭

    #[test]
    fn enters_after_word_boundaries() {
        for last in [
            None,
            Some(' '),
            Some('\n'),
            Some('\t'),
            Some('('),
            Some('['),
            Some('{'),
            Some('\u{3000}'),
        ] {
            assert!(EnglishMode::is_word_start(last), "{last:?}");
        }
        for last in ['0', 'a', 'Z', '/', ':', '"', '\'', ')', '-', '.'] {
            assert!(!EnglishMode::is_word_start(Some(last)), "{last}");
        }
    }

    // §3.6 の英語の行

    #[test]
    fn time_is_untouched() {
        let mut app = App::new();
        app.type_text("10:30");
        assert_eq!(app.text, "10:30");
        assert!(app.marked.is_empty() && !app.panel_visible);
    }

    #[test]
    fn url_is_untouched() {
        let mut app = App::new();
        app.type_text("https://x");
        assert_eq!(app.text, "https://x");
    }

    #[test]
    fn label_colon_is_untouched() {
        let mut app = App::new();
        app.type_text("Note: hi");
        assert_eq!(app.text, "Note: hi");
    }

    #[test]
    fn smiley_retypes_at_close_paren() {
        let mut app = App::new();
        app.type_text(" :");
        assert_eq!(app.marked, ":");
        assert_eq!(app.text, " ");
        app.type_text(")");
        assert_eq!(app.text, " :)");
        assert!(app.marked.is_empty());
    }

    #[test]
    fn typing_ignoring_the_shortcode_gives_the_same_text() {
        let mut app = App::new();
        app.type_text(" :eyes:");
        assert_eq!(app.marked, ":eyes:");
        assert_eq!(app.rows(), [":eyes:", "👀"]);
        app.type_text(" ");
        assert_eq!(app.text, " :eyes: ");
        assert!(app.marked.is_empty() && !app.panel_visible);
    }

    #[test]
    fn backspace_on_close_colon_then_colon_again_shows_candidates_again() {
        let mut app = App::new();
        app.type_text(" :eyes:");
        assert!(app.panel_visible);
        app.press(KeyKind::Backspace);
        assert!(!app.panel_visible);
        assert_eq!(app.marked, ":eyes");
        app.type_text(":");
        assert!(app.panel_visible);
        assert_eq!(app.rows(), [":eyes:", "👀"]);
    }

    #[test]
    fn small_letters_backspace_then_colon_again() {
        let mut app = App::new();
        app.settings.emoji_small_letters = true;
        app.type_text(" :m2:");
        assert_eq!(app.rows(), [":m2:", "m²", "㎡"]);
        app.press(KeyKind::Backspace);
        assert_eq!(app.marked, ":m2");
        app.type_text(":");
        assert_eq!(app.rows(), [":m2:", "m²", "㎡"]);
    }

    #[test]
    fn small_letters_off_by_default_leaves_the_text_alone() {
        let mut app = App::new();
        app.type_text(":m2:");
        assert_eq!(app.text, ":m2:");
        assert!(app.panel_rows.is_empty());
        assert!(!default_settings().emoji_small_letters);
    }

    #[test]
    fn small_letters_on_shows_the_typed_text_then_the_small_letters() {
        let mut app = App::new();
        app.settings.emoji_small_letters = true;
        app.type_text(":m2:");
        assert_eq!(app.rows(), [":m2:", "m²", "㎡"]);
        assert_eq!(app.marked, ":m2:");
    }

    #[test]
    fn emoji_table_wins_over_the_small_letters_rule() {
        let mut app = App::new();
        app.settings.emoji_small_letters = true;
        app.type_text(":eyes:");
        assert_eq!(app.rows(), [":eyes:", "👀"]);
    }

    #[test]
    fn enter_on_first_candidate_passes_enter() {
        let mut app = App::new();
        app.type_text(" :eyes:");
        app.press(KeyKind::Enter);
        assert_eq!(app.text, " :eyes:");
        assert_eq!(app.passed_keys, [KeyKind::Enter]);
        assert!(!app.panel_visible);
    }

    #[test]
    fn enter_on_emoji_commits_and_consumes_enter() {
        let mut app = App::new();
        app.type_text(" :eyes:");
        app.press(KeyKind::Down);
        assert_eq!(app.marked, "👀");
        app.press(KeyKind::Enter);
        assert_eq!(app.text, " 👀");
        assert!(app.passed_keys.is_empty());
        assert!(!app.panel_visible);
    }

    #[test]
    fn ruby_symbol_retypes() {
        let mut app = App::new();
        app.type_text(" :foo,");
        assert_eq!(app.text, " :foo,");
        assert!(app.marked.is_empty());
    }

    #[test]
    fn markdown_table_retypes() {
        let mut app = App::new();
        app.type_text(" :---");
        // B52: `-` は名前の文字なので状態を保つ。次の非名前文字（表の ` |`）かフラッシュで確定される
        assert_eq!(app.marked, ":---");
        app.type_text(" |");
        assert_eq!(app.text, " :--- |");
        assert!(app.marked.is_empty());
    }

    #[test]
    fn custom_emoji_retypes_when_prefix_stops_matching() {
        let mut app = App::new();
        app.type_text(" :p");
        // B52: 前方一致が切れても名前の文字が続く間は状態を保つ（Backspace で直せる）
        assert_eq!(app.text, " ");
        assert_eq!(app.marked, ":p");
        assert!(!app.marked_underlined);
        // 閉じの `:` が完全一致しないので打ち直し。以後は Slack 自前の補完。ikari は何もしない
        app.type_text("arty_parrot:");
        assert_eq!(app.text, " :party_parrot:");
        assert!(app.marked.is_empty());
        assert!(!app.panel_visible);
    }

    // B52: 途中のタイポを直せる

    #[test]
    fn typo_then_backspace_then_finish_shows_candidates() {
        let mut app = App::new();
        app.type_text(" :eyx");
        assert_eq!(app.marked, ":eyx");
        assert_eq!(app.text, " ");
        app.press(KeyKind::Backspace);
        assert_eq!(app.marked, ":ey");
        app.type_text("es:");
        assert_eq!(app.rows(), [":eyes:", "👀"]);
        assert_eq!(app.marked, ":eyes:");
    }

    #[test]
    fn typo_closed_with_colon_retypes_the_whole_string() {
        let mut app = App::new();
        app.type_text(" :eyxs:");
        assert_eq!(app.text, " :eyxs:");
        assert!(app.marked.is_empty() && !app.panel_visible);
    }

    #[test]
    fn tab_and_down_retype_without_prefix_match() {
        let mut tab = App::new();
        tab.type_text(" :eyx");
        tab.press(ch_kind("\t"));
        assert_eq!(tab.text, " :eyx\t");
        assert!(tab.marked.is_empty());
        assert!(!tab.panel_visible);
        let mut down = App::new();
        down.type_text(" :eyx");
        down.press(KeyKind::Down);
        assert_eq!(down.text, " :eyx");
        assert_eq!(down.passed_keys, [KeyKind::Down]);
        assert!(!down.panel_visible);
        let mut matched = App::new();
        matched.type_text(" :ey");
        matched.press(ch_kind("\t"));
        assert_eq!(matched.rows(), [":ey", "👁", "👀"]);
    }

    #[test]
    fn non_name_characters_still_retype_immediately() {
        for typed in [
            " :foo,",
            " :D",
            " :P",
            " :)",
            " :3 ",
            " :eyx!",
            " :eyxA",
            " :ey\u{3042}",
        ] {
            let mut app = App::new();
            app.type_text(typed);
            assert_eq!(app.text, typed);
            assert!(app.marked.is_empty() && !app.panel_visible, "{typed:?}");
        }
    }

    #[test]
    fn hyphen_run_is_committed_by_flush() {
        let mut app = App::new();
        app.type_text(" :---");
        app.flush();
        assert_eq!(app.text, " :---");
        assert!(app.marked.is_empty());
    }

    #[test]
    fn name_length_limit_retypes() {
        let mut app = App::new();
        let name = "x".repeat(MAX_NAME_LENGTH);
        app.type_text(&format!(" :{name}"));
        assert_eq!(app.marked, format!(":{name}"));
        app.type_text("x");
        assert!(app.marked.is_empty());
        assert_eq!(app.text, format!(" :{name}x"));
    }

    #[test]
    fn backspace_to_colon_then_backspace_cancels() {
        let mut app = App::new();
        app.type_text(" :zz");
        app.press(KeyKind::Backspace);
        app.press(KeyKind::Backspace);
        assert_eq!(app.marked, ":");
        app.press(KeyKind::Backspace);
        assert!(app.marked.is_empty());
        assert_eq!(app.text, " ");
    }

    // 英語の列のキー

    #[test]
    fn prefix_candidates_with_down_then_select() {
        let mut app = App::new();
        app.type_text(" :ey");
        assert_eq!(app.marked, ":ey");
        app.press(KeyKind::Down);
        assert_eq!(app.rows(), [":ey", "👁", "👀"]);
        app.press(KeyKind::Down);
        app.press(KeyKind::Down);
        assert_eq!(app.marked, "👀");
        app.press(KeyKind::Up);
        assert_eq!(app.marked, "👁");
        app.type_text(" ");
        assert_eq!(app.text, " 👁 ");
    }

    #[test]
    fn tab_and_shift_tab_move_selection() {
        let mut app = App::new();
        app.type_text(":ey");
        app.press(ch_kind("\t"));
        app.press(ch_kind("\t"));
        assert_eq!(app.marked, "👁");
        app.press_with(ch_kind("\t"), mods(true, false, false, false), false);
        assert_eq!(app.marked, ":ey");
        app.press(ch_kind("\u{19}"));
        assert_eq!(app.marked, "👀");
    }

    #[test]
    fn space_in_typing_retypes_with_space() {
        let mut app = App::new();
        app.type_text(":ey ");
        assert_eq!(app.text, ":ey ");
        assert!(app.marked.is_empty());
    }

    #[test]
    fn digit_commits_selected_and_passes_digit() {
        let mut app = App::new();
        app.type_text(":eyes:");
        app.press(KeyKind::Down);
        app.press(ch_kind("1"));
        assert_eq!(app.text, "👀1");
    }

    #[test]
    fn letter_commits_and_passes_letter() {
        let mut app = App::new();
        app.type_text(":eyes:");
        app.type_text("x");
        assert_eq!(app.text, ":eyes:x");
    }

    #[test]
    fn backspace_cancels_bare_colon() {
        let mut app = App::new();
        app.type_text(":");
        app.press(KeyKind::Backspace);
        assert!(app.text.is_empty() && app.marked.is_empty());
        assert!(app.passed_keys.is_empty());
    }

    #[test]
    fn backspace_shortens_name() {
        let mut app = App::new();
        app.type_text(":eye");
        app.press(KeyKind::Backspace);
        assert_eq!(app.marked, ":ey");
    }

    #[test]
    fn backspace_from_choices_returns_to_typing() {
        let mut app = App::new();
        app.type_text(":ey");
        app.press(KeyKind::Down);
        app.press(KeyKind::Backspace);
        assert_eq!(app.marked, ":ey");
        assert!(!app.panel_visible);
    }

    #[test]
    fn escape_in_choices_commits_literal_and_consumes() {
        let mut app = App::new();
        app.type_text(":eyes:");
        app.press(KeyKind::Down);
        app.press(KeyKind::Escape);
        assert_eq!(app.text, ":eyes:");
        assert!(app.passed_keys.is_empty());
    }

    #[test]
    fn escape_in_typing_commits_and_passes_escape() {
        let mut app = App::new();
        app.type_text(":ey");
        app.press(KeyKind::Escape);
        assert_eq!(app.text, ":ey");
        assert_eq!(app.passed_keys, [KeyKind::Escape]);
    }

    #[test]
    fn arrow_and_modifier_keys_commit_and_pass() {
        let mut app = App::new();
        app.type_text(":eyes:");
        app.press(KeyKind::Left);
        assert_eq!(app.text, ":eyes:");
        assert_eq!(app.passed_keys, [KeyKind::Left]);

        app.type_text(" :ey");
        app.press_with(ch_kind("c"), mods(false, false, false, true), false);
        assert_eq!(app.text, ":eyes: :ey");
        assert_eq!(app.passed_keys, [KeyKind::Left, ch_kind("c")]);
    }

    #[test]
    fn committed_text_goes_to_the_pool() {
        let mut app = App::new();
        app.type_text("a :eyes:");
        app.press(KeyKind::Down);
        app.press(KeyKind::Enter);
        // Enter は消費。たまりはまだ区切られていない
        assert!(app.sent.is_empty());
        app.type_text(" b");
        app.press(KeyKind::Enter);
        assert_eq!(app.sent, ["a 👀 b"]);
    }

    #[test]
    fn passed_enter_after_literal_flushes_pool_with_shortcode_text() {
        let mut app = App::new();
        app.type_text(":eyes:");
        app.press(KeyKind::Enter);
        assert_eq!(app.sent, [":eyes:"]);
    }

    #[test]
    fn repeated_colon_does_not_enter() {
        let mut app = App::new();
        app.press_with(ch_kind(":"), Modifiers::NONE, true);
        assert_eq!(app.text, ":");
        assert!(app.marked.is_empty());
    }

    // 入りの条件

    #[test]
    fn last_pool_character_decides_entry() {
        let mut app = App::new();
        app.type_text("(:");
        assert_eq!(app.marked, ":");
        app.press(KeyKind::Escape);
        app.type_text("a:");
        assert!(app.marked.is_empty());
    }

    #[test]
    fn empty_pool_enters_after_separator() {
        let mut app = App::new();
        app.type_text("ab");
        app.press(KeyKind::Enter);
        app.type_text(":e");
        assert_eq!(app.marked, ":e");
    }

    #[test]
    fn setting_off_does_not_enter() {
        let mut app = App::new();
        app.settings.emoji_shortcode = false;
        app.type_text(" :eyes:");
        assert_eq!(app.text, " :eyes:");
        assert!(app.marked.is_empty());
    }

    #[test]
    fn excluded_app_does_not_enter() {
        let mut app = App::new();
        app.bundle_id = "com.apple.Terminal".into();
        app.type_text(" :eyes:");
        assert_eq!(app.text, " :eyes:");
        assert!(app.marked.is_empty());
    }

    #[test]
    fn secure_does_not_enter() {
        let mut app = App::new();
        app.secure = true;
        app.type_text(" :eyes:");
        assert_eq!(app.text, " :eyes:");
        assert!(app.marked.is_empty());
    }

    #[test]
    fn empty_table_does_not_enter() {
        let mut app = App::with_table(ShortcodeTable::empty());
        app.type_text(" :eyes:");
        assert_eq!(app.text, " :eyes:");
        assert!(app.marked.is_empty());
    }

    #[test]
    fn modifier_colon_does_not_enter() {
        let mut app = App::new();
        app.press_with(ch_kind(":"), mods(false, false, false, true), false);
        assert!(app.marked.is_empty());
        assert_eq!(app.passed_keys, [ch_kind(":")]);
    }

    // フラッシュ

    #[test]
    fn flush_commits_typing_as_is() {
        let mut app = App::new();
        app.type_text("x :ey");
        app.flush();
        assert_eq!(app.text, "x :ey");
        assert!(!app.mode.is_shortcode_active());
        assert_eq!(app.sent, ["x :ey"]);
    }

    #[test]
    fn flush_commits_selected_candidate_and_closes_panel() {
        let mut app = App::new();
        app.type_text(":ey");
        app.press(KeyKind::Down);
        app.press(KeyKind::Down);
        assert!(app.panel_visible);
        app.flush();
        assert_eq!(app.text, "👁");
        assert!(!app.panel_visible);
        assert_eq!(app.sent, ["👁"]);
    }

    #[test]
    fn flush_without_shortcode_does_nothing() {
        let mut app = App::new();
        app.type_text("hi");
        assert_eq!(
            app.mode.flush_shortcode(DiagEnd::CommitComposition, 0),
            None
        );
    }

    // 下線

    #[test]
    fn typing_has_no_underline() {
        let mut app = App::new();
        app.type_text(" :ey");
        assert_eq!(app.marked, ":ey");
        assert!(!app.marked_underlined);
    }

    #[test]
    fn choosing_has_an_underline() {
        let mut app = App::new();
        app.type_text(" :eyes:");
        assert!(app.marked_underlined);
    }

    #[test]
    fn down_choosing_has_an_underline() {
        let mut app = App::new();
        app.type_text(" :ey");
        app.press(KeyKind::Down);
        assert!(app.panel_visible);
        assert!(app.marked_underlined);
    }

    // 候補ウィンドウの形

    #[test]
    fn panel_labels_the_emoji_rows_and_marks_the_typed_row_literal() {
        let mut app = App::new();
        app.type_text(":eyes:");
        let outcome = {
            let environment = EnglishShortcodeEnvironment {
                settings: &app.settings,
                client_bundle_id: &app.bundle_id,
                secure: false,
                is_repeat: false,
            };
            app.mode.handle_text(&key(KeyKind::Down), &environment, 0)
        };
        let ShellAction::MoveSelection(model) = &outcome.actions[1] else {
            panic!("{:?}", outcome.actions);
        };
        assert_eq!(model.selected, 1);
        assert_eq!(model.candidates[0].kind, CandidateKind::Literal);
        assert_eq!(model.candidates[0].label, "");
        assert_eq!(model.candidates[1].kind, CandidateKind::Unspecified);
        assert_eq!(model.candidates[1].label, ":eyes:");
        assert_eq!(model.reserved_rows, 2);
    }

    // 診断ログ（ADR 0034 §4.2）

    fn finish(app: &mut App) -> Option<crate::diag::DiagEvent> {
        app.mode.take_finished_diag().map(|f| f.event(42))
    }

    /// キーを 1 つ流して、終わった合成があれば集める
    fn drain(app: &mut App, events: &mut Vec<crate::diag::DiagEvent>) {
        if let Some(event) = finish(app) {
            events.push(event);
        }
    }

    fn run_keys(app: &mut App, text: &str, events: &mut Vec<crate::diag::DiagEvent>) {
        for character in text.chars() {
            app.type_text(&character.to_string());
            drain(app, events);
        }
    }

    fn press_and_drain(app: &mut App, kind: KeyKind, events: &mut Vec<crate::diag::DiagEvent>) {
        app.press(kind);
        drain(app, events);
    }

    #[test]
    fn diag_picking_an_emoji_is_one_episode() {
        let mut app = App::new();
        let mut events = Vec::new();
        run_keys(&mut app, ":eyes:", &mut events);
        assert!(events.is_empty()); // 打鍵ごとには送らない
        app.mode.note_panel_shown(AnchorSource::Client);
        press_and_drain(&mut app, KeyKind::Down, &mut events);
        press_and_drain(&mut app, KeyKind::Enter, &mut events);
        assert_eq!(events.len(), 1);
        let event = &events[0];
        assert_eq!(event.kind, "sh_episode");
        assert_eq!(event.session_id, 42);
        assert_eq!(event.tags["mode"], "shortcode_en");
        assert_eq!(event.tags["end"], "shortcode_pick");
        assert_eq!(event.tags["pick_kind"], "emoji");
        assert_eq!(event.tags["panel"], "client");
        assert!(!event.tags.contains_key("prev"));
        assert_eq!(event.nums["keys"], 8);
        assert_eq!(event.nums["keys_to_conv"], 6);
        assert_eq!(event.nums["cand_moves"], 1);
        assert!(event.nums["dur_ms"] > 0);
        assert!(event.nums["pick_ms"] > 0);
        assert!(!event.nums.contains_key("wait_us"));
    }

    #[test]
    fn diag_small_letter_pick() {
        let mut app = App::new();
        app.settings.emoji_small_letters = true;
        let mut events = Vec::new();
        run_keys(&mut app, ":m2:", &mut events);
        press_and_drain(&mut app, KeyKind::Down, &mut events);
        press_and_drain(&mut app, KeyKind::Enter, &mut events);
        assert_eq!(events[0].tags["pick_kind"], "small_letter");
    }

    #[test]
    fn diag_a_flush_carries_the_reason_and_the_previous_state() {
        let mut app = App::new();
        let mut events = Vec::new();
        run_keys(&mut app, ":ey", &mut events);
        app.clock += STEP;
        let _ = app.mode.flush_shortcode(DiagEnd::ModeSwitch, app.clock);
        drain(&mut app, &mut events);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].tags["end"], "mode_switch");
        assert_eq!(events[0].tags["prev"], "shortcode");
        assert_eq!(events[0].tags["panel"], "none");

        let _ = app.mode.flush(); // 区切り（直前が語の頭になる）
        run_keys(&mut app, ":eyes:", &mut events);
        app.clock += STEP;
        let _ = app.mode.flush_shortcode(DiagEnd::Deactivate, app.clock);
        drain(&mut app, &mut events);
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].tags["end"], "deactivate");
        assert_eq!(events[1].tags["prev"], "shortcode_choice");
    }

    #[test]
    fn diag_retype_is_not_counted() {
        let mut app = App::new();
        let mut events = Vec::new();
        run_keys(&mut app, ":ey", &mut events);
        press_and_drain(&mut app, ch_kind("!"), &mut events); // 名前の文字ではない: 打ち直し
        assert!(events.is_empty());
        assert_eq!(app.mode.diag_episode(), None);
    }

    #[test]
    fn diag_backspace_on_the_bare_colon_is_bs_empty() {
        let mut app = App::new();
        let mut events = Vec::new();
        run_keys(&mut app, ":", &mut events);
        press_and_drain(&mut app, KeyKind::Backspace, &mut events);
        assert_eq!(events[0].tags["end"], "bs_empty");
        assert_eq!(events[0].nums["bs"], 1);
    }

    #[test]
    fn diag_ordinary_typing_makes_no_episode() {
        let mut app = App::new();
        let mut events = Vec::new();
        run_keys(&mut app, "hello world", &mut events);
        press_and_drain(&mut app, KeyKind::Enter, &mut events);
        assert!(events.is_empty());
    }

    #[test]
    fn diag_literal_commit_is_shortcode_literal_without_pick_kind() {
        let mut app = App::new();
        let mut events = Vec::new();
        // 第 1 候補（そのままの入力）のまま Enter: 候補は出していたので pick になり、種類は literal
        run_keys(&mut app, ":eyes:", &mut events);
        press_and_drain(&mut app, KeyKind::Enter, &mut events);
        assert_eq!(events[0].tags["end"], "shortcode_pick");
        assert_eq!(events[0].tags["pick_kind"], "literal");
    }

    #[test]
    fn diag_all_words_are_in_the_allow_list() {
        let mut app = App::new();
        let mut events = Vec::new();
        run_keys(&mut app, ":eyes:", &mut events);
        press_and_drain(&mut app, KeyKind::Down, &mut events);
        press_and_drain(&mut app, KeyKind::Enter, &mut events);
        let pick_kinds = [
            "dict",
            "hiragana",
            "katakana",
            "rewriter",
            "literal",
            "fallback",
            "emoji",
            "small_letter",
        ];
        let panels = [
            "client",
            "accessibility",
            "last_known",
            "mouse_screen",
            "none",
        ];
        assert!(pick_kinds.contains(&events[0].tags["pick_kind"].as_str()));
        assert!(panels.contains(&events[0].tags["panel"].as_str()));
    }
}
