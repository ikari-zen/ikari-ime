//! 英語のモードの郵便番号（ADR 0032 §9.1。issue #27）と Unicode 表記（issue #28）の状態機械。
//! 今の `EnglishPostal.swift` の `PostalMachine`（`EnglishMode` への取り付けは `english::postal`）。レーン L-A1。
//!
//! 語の頭の `te` + 数字 7 桁、`u+` + 16 進 4〜6 桁をためて、最後の桁を打った瞬間に候補ウィンドウを出す
//! （`u+` は 4 桁目から出て、5 桁、6 桁と増えるたびに更新する）。最後の桁より前は候補も下線も出さない。
//! 候補を出している間も下線は付けない。選ぶのは ↑/↓、確定は Enter、Esc は打った文字のまま確定して閉じる。
//! 住所はエンジンに 1 回だけ非同期に問い合わせ（`PostalLookupRequest`）、返事が来たとき状態が同じなら候補に足す。
//! この型は問い合わせをしない（呼ぶ側が `timeouts::POSTAL_LOOKUP` を上限にして行う）。

use crate::english::marked_text;
use crate::types::{
    CandidateItem, CandidateKind, CandidatePanelModel, KeyInput, KeyKind, ShellAction,
};

/// 郵便番号の桁数。
pub const POSTAL_DIGITS: usize = 7;
/// `u+` の 16 進の桁数の下限と上限。
pub const HEX_MIN: usize = 4;
pub const HEX_MAX: usize = 6;
/// 候補ウィンドウの 1 ページの行数。
pub const PAGE_SIZE: usize = 9;
const TE_PREFIX: &str = "te";
const UNICODE_PREFIX: &str = "u+";

/// 住所の問い合わせ。`serial` は候補を出すたびに増え、返事が古いかを見分ける（同じ番号を打ち直した場合も区別する）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostalLookupRequest {
    pub code: String,
    pub serial: u64,
}

/// ためる語の種類。`Te` = `te` + 郵便番号（`postal.english`）、`Unicode` = `u+` + 16 進（`unicode.notation`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PostalKind {
    Te,
    Unicode,
}

/// 今オンの種類（設定）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PostalKinds {
    pub te: bool,
    pub unicode: bool,
}

impl PostalKinds {
    pub const TE_ONLY: Self = Self {
        te: true,
        unicode: false,
    };

    pub fn contains(self, kind: PostalKind) -> bool {
        match kind {
            PostalKind::Te => self.te,
            PostalKind::Unicode => self.unicode,
        }
    }

    pub fn is_empty(self) -> bool {
        !self.te && !self.unicode
    }
}

impl Default for PostalKinds {
    fn default() -> Self {
        Self::TE_ONLY
    }
}

/// 1 打鍵の結果。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PostalStep {
    pub actions: Vec<ShellAction>,
    pub pass_key: bool,
    /// 確定した文字列（文脈のたまりに足す）
    pub committed: Option<String>,
    /// 非同期に問い合わせる番号
    pub lookup: Option<PostalLookupRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostalChoosing {
    pub kind: PostalKind,
    pub buffer: String,
    pub rows: Vec<String>,
    pub selected: usize,
    /// 返事を待っている問い合わせ。返事が来たら `None`
    pub pending: Option<PostalLookupRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum PostalPhase {
    #[default]
    Inactive,
    Typing(PostalKind, String),
    Choosing(PostalChoosing),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PostalMachine {
    phase: PostalPhase,
    serial: u64,
}

impl PostalMachine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn phase(&self) -> &PostalPhase {
        &self.phase
    }

    pub fn is_active(&self) -> bool {
        self.phase != PostalPhase::Inactive
    }

    /// 今ためている語の種類（ためていなければ `None`）。
    pub fn active_kind(&self) -> Option<PostalKind> {
        match &self.phase {
            PostalPhase::Inactive => None,
            PostalPhase::Typing(kind, _) => Some(*kind),
            PostalPhase::Choosing(choosing) => Some(choosing.kind),
        }
    }

    // MARK: キー

    /// 関係ないキー（ためていなくて、入りの条件も満たさない）は `None`。`kinds` は今オンの種類（設定）。
    pub fn handle(
        &mut self,
        input: &KeyInput,
        can_enter: bool,
        kinds: PostalKinds,
    ) -> Option<PostalStep> {
        match self.phase.clone() {
            PostalPhase::Inactive => self.enter(input, can_enter, kinds),
            PostalPhase::Typing(kind, buffer) => Some(self.typing(kind, &buffer, input)),
            PostalPhase::Choosing(choosing) => Some(self.choose(choosing, input)),
        }
    }

    fn enter(
        &mut self,
        input: &KeyInput,
        can_enter: bool,
        kinds: PostalKinds,
    ) -> Option<PostalStep> {
        if !can_enter || !is_plain(input) {
            return None;
        }
        let KeyKind::Character { text } = &input.key else {
            return None;
        };
        let kind = match text.as_str() {
            "t" if kinds.te => PostalKind::Te,
            "u" if kinds.unicode => PostalKind::Unicode,
            _ => return None,
        };
        self.phase = PostalPhase::Typing(kind, text.clone());
        Some(typing_step(text))
    }

    fn typing(&mut self, kind: PostalKind, buffer: &str, input: &KeyInput) -> PostalStep {
        if !is_plain(input) {
            return self.retype(buffer, false);
        }
        match &input.key {
            KeyKind::Character { text } => {
                if let Some(next) = append(text, buffer, kind) {
                    return self.settle(kind, next);
                }
            }
            KeyKind::Backspace => {
                let mut shorter = buffer.to_string();
                shorter.pop();
                if shorter.is_empty() {
                    self.phase = PostalPhase::Inactive;
                    return PostalStep {
                        actions: vec![mark("")],
                        ..PostalStep::default()
                    };
                }
                self.phase = PostalPhase::Typing(kind, shorter.clone());
                return typing_step(&shorter);
            }
            _ => {}
        }
        self.retype(buffer, false)
    }

    fn choose(&mut self, choosing: PostalChoosing, input: &KeyInput) -> PostalStep {
        if !is_plain(input) {
            return self.retype(&choosing.buffer, true);
        }
        match &input.key {
            KeyKind::Down => self.move_selection(choosing, 1),
            KeyKind::Up => self.move_selection(choosing, -1),
            KeyKind::Enter => self.commit(&choosing),
            KeyKind::Escape => {
                // 打った文字のまま確定して閉じる。Esc は消費する
                let mut step = self.retype(&choosing.buffer, true);
                step.pass_key = false;
                step
            }
            KeyKind::Backspace => {
                // `u+` は 5 桁、6 桁なら 1 桁戻っても候補が残る（更新）。足りなくなったら閉じる
                let mut shorter = choosing.buffer.clone();
                shorter.pop();
                let mut step = self.settle(choosing.kind, shorter);
                if matches!(self.phase, PostalPhase::Typing(..)) {
                    step.actions.push(ShellAction::HideCandidates);
                }
                step
            }
            KeyKind::Character { text } => {
                // `u+` は桁が増えるたびに候補を更新する。ほかは打ち直し
                if let Some(next) = append(text, &choosing.buffer, choosing.kind)
                    && base_rows(choosing.kind, &next).is_some()
                {
                    return self.settle(choosing.kind, next);
                }
                self.retype(&choosing.buffer, true)
            }
            // Space、Tab、数字、ほかの文字、桁あふれ、←/→ など
            _ => self.retype(&choosing.buffer, true),
        }
    }

    // MARK: 遷移の部品

    /// 最後の桁がそろったら候補を出し、そろっていなければ「ためている状態」にする。
    fn settle(&mut self, kind: PostalKind, buffer: String) -> PostalStep {
        let Some(rows) = base_rows(kind, &buffer) else {
            self.phase = PostalPhase::Typing(kind, buffer.clone());
            return typing_step(&buffer);
        };
        // 住所を問い合わせるのは郵便番号だけ。`u+` は殻の中で文字が決まる
        let request = postal_code(kind, &buffer).map(|code| {
            self.serial += 1;
            PostalLookupRequest {
                code,
                serial: self.serial,
            }
        });
        let choosing = PostalChoosing {
            kind,
            buffer,
            rows,
            selected: 0,
            pending: request.clone(),
        };
        let actions = vec![
            mark(&choosing.rows[0]),
            ShellAction::ShowCandidates(model(&choosing)),
        ];
        self.phase = PostalPhase::Choosing(choosing);
        PostalStep {
            actions,
            lookup: request,
            ..PostalStep::default()
        }
    }

    fn move_selection(&mut self, choosing: PostalChoosing, delta: i64) -> PostalStep {
        let mut next = choosing.clone();
        let count = i64::try_from(choosing.rows.len())
            .unwrap_or(i64::MAX)
            .max(1);
        let selected = i64::try_from(choosing.selected).unwrap_or(0);
        next.selected = usize::try_from((selected + delta).rem_euclid(count)).unwrap_or(0);
        let same_page = next.selected / PAGE_SIZE == choosing.selected / PAGE_SIZE;
        let panel = model(&next);
        let row = next.rows.get(next.selected).cloned().unwrap_or_default();
        self.phase = PostalPhase::Choosing(next);
        PostalStep {
            actions: vec![
                mark(&row),
                if same_page {
                    ShellAction::MoveSelection(panel)
                } else {
                    ShellAction::ShowCandidates(panel)
                },
            ],
            ..PostalStep::default()
        }
    }

    fn commit(&mut self, choosing: &PostalChoosing) -> PostalStep {
        let text = choosing
            .rows
            .get(choosing.selected)
            .cloned()
            .unwrap_or_else(|| choosing.buffer.clone());
        self.phase = PostalPhase::Inactive;
        PostalStep {
            actions: vec![
                ShellAction::Commit { text: text.clone() },
                ShellAction::HideCandidates,
            ],
            committed: Some(text),
            ..PostalStep::default()
        }
    }

    /// 形から外れた。ためた文字をそのまま確定し、今のキーだけアプリに渡す
    /// （過去のキーは渡し直せない。ADR 0027 §3.3 と同じ）。
    fn retype(&mut self, buffer: &str, hides_candidates: bool) -> PostalStep {
        self.phase = PostalPhase::Inactive;
        let mut actions = vec![ShellAction::Commit {
            text: buffer.to_string(),
        }];
        if hides_candidates {
            actions.push(ShellAction::HideCandidates);
        }
        PostalStep {
            actions,
            pass_key: true,
            committed: Some(buffer.to_string()),
            lookup: None,
        }
    }

    /// 状態を捨てる（モードの切り替え、secure、`deactivateServer`、設定のオフなど）。ためた文字を確定し、キーは渡さない。
    pub fn flush(&mut self) -> Option<PostalStep> {
        let mut step = match self.phase.clone() {
            PostalPhase::Inactive => return None,
            PostalPhase::Typing(_, buffer) => self.retype(&buffer, false),
            PostalPhase::Choosing(choosing) => self.retype(&choosing.buffer, true),
        };
        step.pass_key = false;
        Some(step)
    }

    // MARK: 非同期の返事

    /// 返事を受ける。候補を出している間で、同じ問い合わせ（番号と通し番号）のときだけ候補に足す。違えば捨てる（`None`）。
    /// 住所が空なら足さない。
    pub fn lookup_arrived(
        &mut self,
        request: &PostalLookupRequest,
        addresses: &[String],
    ) -> Option<PostalStep> {
        let PostalPhase::Choosing(choosing) = &mut self.phase else {
            return None;
        };
        if choosing.pending.as_ref() != Some(request) {
            return None;
        }
        choosing.pending = None;
        let mut added = false;
        for address in addresses {
            if !address.is_empty() && !choosing.rows.contains(address) {
                choosing.rows.push(address.clone());
                added = true;
            }
        }
        added.then(|| PostalStep {
            actions: vec![ShellAction::ShowCandidates(model(choosing))],
            ..PostalStep::default()
        })
    }
}

// MARK: 純関数

fn is_plain(input: &KeyInput) -> bool {
    !(input.modifiers.control || input.modifiers.option || input.modifiers.command)
}

/// 次の 1 文字を足した文字列。形に合わなければ `None`。
pub fn append(text: &str, buffer: &str, kind: PostalKind) -> Option<String> {
    let mut chars = text.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else {
        return None;
    };
    let length = buffer.chars().count();
    match kind {
        PostalKind::Te => {
            if buffer == "t" {
                return (text == "e").then(|| TE_PREFIX.to_string());
            }
            (c.is_ascii_digit() && length.saturating_sub(TE_PREFIX.len()) < POSTAL_DIGITS)
                .then(|| format!("{buffer}{text}"))
        }
        PostalKind::Unicode => {
            if buffer == "u" {
                return (text == "+").then(|| UNICODE_PREFIX.to_string());
            }
            (c.is_ascii_hexdigit() && length.saturating_sub(UNICODE_PREFIX.len()) < HEX_MAX)
                .then(|| format!("{buffer}{text}"))
        }
    }
}

/// 最後の桁がそろった形なら、問い合わせ前の候補（0 番は「そのままの入力」）。そろっていなければ `None`。
pub fn base_rows(kind: PostalKind, buffer: &str) -> Option<Vec<String>> {
    match kind {
        PostalKind::Te => {
            let code = postal_code(kind, buffer)?;
            Some(vec![
                buffer.to_string(),
                format!("〒{}-{}", &code[..3], &code[3..]),
                format!("〒{code}"),
            ])
        }
        PostalKind::Unicode => {
            let character = unicode_character(buffer)?;
            Some(vec![buffer.to_string(), character])
        }
    }
}

/// 問い合わせる番号（最後の桁がそろったときだけ）。
pub fn postal_code(kind: PostalKind, buffer: &str) -> Option<String> {
    match kind {
        PostalKind::Te => {
            let code = buffer.strip_prefix(TE_PREFIX)?;
            (code.chars().count() == POSTAL_DIGITS && code.is_ascii()).then(|| code.to_string())
        }
        PostalKind::Unicode => None,
    }
}

/// `u+` + 16 進（4〜6 桁）が表す 1 文字。10FFFF を超える並び、サロゲート、制御文字、未割り当ては `None`（出さない）。
pub fn unicode_character(buffer: &str) -> Option<String> {
    let digits = buffer.strip_prefix(UNICODE_PREFIX)?;
    if !(HEX_MIN..=HEX_MAX).contains(&digits.chars().count()) {
        return None;
    }
    let value = u32::from_str_radix(digits, 16).ok()?;
    let character = char::from_u32(value)?;
    if character.is_control() || crate::english::is_noncharacter(character) {
        return None;
    }
    Some(character.to_string())
}

fn model(choosing: &PostalChoosing) -> CandidatePanelModel {
    let start = choosing.selected / PAGE_SIZE * PAGE_SIZE;
    let end = (start + PAGE_SIZE).min(choosing.rows.len());
    let rows = &choosing.rows[start..end];
    CandidatePanelModel {
        candidates: rows
            .iter()
            .enumerate()
            .map(|(offset, row)| CandidateItem {
                surface: row.clone(),
                annotation: String::new(),
                kind: if start + offset == 0 {
                    CandidateKind::Literal
                } else {
                    CandidateKind::Unspecified
                },
                label: String::new(),
            })
            .collect(),
        selected: u32::try_from(choosing.selected - start).unwrap_or(0),
        annotation: String::new(),
        reserved_rows: u32::try_from(choosing.rows.len().min(PAGE_SIZE).max(rows.len()))
            .unwrap_or(u32::MAX),
    }
}

fn typing_step(buffer: &str) -> PostalStep {
    PostalStep {
        actions: vec![mark(buffer)],
        ..PostalStep::default()
    }
}

/// 下線なし（ADR 0032 §9.1）。
fn mark(text: &str) -> ShellAction {
    marked_text(text, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Modifiers;

    fn plain(kind: KeyKind) -> KeyInput {
        KeyInput {
            key: kind,
            modifiers: Modifiers::NONE,
            is_repeat: false,
            secure: false,
        }
    }

    fn ch(text: &str) -> KeyInput {
        plain(KeyKind::Character { text: text.into() })
    }

    fn chars(text: &str) -> Vec<KeyInput> {
        text.chars().map(|c| ch(&c.to_string())).collect()
    }

    fn feed_with(
        machine: &mut PostalMachine,
        keys: &[KeyInput],
        kinds: PostalKinds,
    ) -> Option<PostalStep> {
        let mut last = None;
        for key in keys {
            last = machine.handle(key, true, kinds);
        }
        last
    }

    fn feed(machine: &mut PostalMachine, keys: &[KeyInput]) -> Option<PostalStep> {
        feed_with(machine, keys, PostalKinds::TE_ONLY)
    }

    fn feed_unicode(machine: &mut PostalMachine, keys: &[KeyInput]) -> Option<PostalStep> {
        let kinds = PostalKinds {
            te: false,
            unicode: true,
        };
        feed_with(machine, keys, kinds)
    }

    fn underlined(step: &Option<PostalStep>) -> bool {
        step.iter()
            .flat_map(|s| s.actions.iter())
            .find_map(|a| match a {
                ShellAction::SetMarkedText(marked) => Some(marked.underlined),
                _ => None,
            })
            .unwrap_or(false)
    }

    fn rows(machine: &PostalMachine) -> Option<Vec<String>> {
        match machine.phase() {
            PostalPhase::Choosing(choosing) => Some(choosing.rows.clone()),
            _ => None,
        }
    }

    fn shows_candidates(step: &Option<PostalStep>) -> bool {
        step.iter()
            .flat_map(|s| s.actions.iter())
            .any(|a| matches!(a, ShellAction::ShowCandidates(_)))
    }

    fn hides_candidates(step: &Option<PostalStep>) -> bool {
        step.iter()
            .flat_map(|s| s.actions.iter())
            .any(|a| *a == ShellAction::HideCandidates)
    }

    const CODE: &str = "1000001";
    const CHIYODA: &str = "東京都千代田区千代田";

    fn committed(step: Option<PostalStep>) -> Option<String> {
        step.and_then(|s| s.committed)
    }

    #[test]
    fn plain_digits_and_other_letters_are_not_taken() {
        let mut machine = PostalMachine::new();
        assert_eq!(machine.handle(&ch("1"), true, PostalKinds::TE_ONLY), None);
        assert_eq!(machine.handle(&ch("u"), true, PostalKinds::TE_ONLY), None);
        assert!(!machine.is_active());
    }

    #[test]
    fn not_word_start_does_not_enter() {
        let mut machine = PostalMachine::new();
        assert_eq!(machine.handle(&ch("t"), false, PostalKinds::TE_ONLY), None);
    }

    #[test]
    fn opens_on_the_seventh_digit_without_underline() {
        let mut machine = PostalMachine::new();
        let before = feed(&mut machine, &chars("te100000"));
        assert!(!underlined(&before));
        assert_eq!(rows(&machine), None);
        assert!(!shows_candidates(&before));
        let step = feed(&mut machine, &[ch("1")]);
        assert!(!underlined(&step));
        assert!(shows_candidates(&step));
        assert_eq!(step.unwrap().lookup.unwrap().code, CODE);
        assert_eq!(
            rows(&machine),
            Some(vec![
                "te1000001".into(),
                "〒100-0001".into(),
                "〒1000001".into()
            ])
        );
    }

    #[test]
    fn down_and_up_move_and_enter_commits() {
        let mut machine = PostalMachine::new();
        feed(&mut machine, &chars("te1000001"));
        feed(&mut machine, &[plain(KeyKind::Down)]);
        let step = feed(&mut machine, &[plain(KeyKind::Enter)]).unwrap();
        assert_eq!(step.committed.as_deref(), Some("〒100-0001"));
        assert!(!step.pass_key);
        assert!(!machine.is_active());
        feed(&mut machine, &chars("te1000001"));
        feed(&mut machine, &[plain(KeyKind::Up)]);
        assert_eq!(
            committed(feed(&mut machine, &[plain(KeyKind::Enter)])).as_deref(),
            Some("〒1000001")
        );
    }

    #[test]
    fn enter_on_zero_commits_the_typed_text() {
        let mut machine = PostalMachine::new();
        feed(&mut machine, &chars("te1000001"));
        assert_eq!(
            committed(feed(&mut machine, &[plain(KeyKind::Enter)])).as_deref(),
            Some("te1000001")
        );
    }

    #[test]
    fn space_tab_digit_and_letters_commit_as_typed_and_pass_the_key() {
        for key in [
            plain(KeyKind::Space),
            ch("\t"),
            ch("2"),
            ch("a"),
            plain(KeyKind::Left),
        ] {
            let mut machine = PostalMachine::new();
            feed(&mut machine, &chars("te1000001"));
            feed(&mut machine, &[plain(KeyKind::Down)]);
            let step = feed(&mut machine, &[key]);
            assert!(hides_candidates(&step));
            let step = step.unwrap();
            assert_eq!(step.committed.as_deref(), Some("te1000001"));
            assert!(step.pass_key);
            assert!(!machine.is_active());
        }
    }

    #[test]
    fn escape_commits_the_typed_text_and_consumes_the_key() {
        let mut machine = PostalMachine::new();
        feed(&mut machine, &chars("te1000001"));
        let step = feed(&mut machine, &[plain(KeyKind::Escape)]).unwrap();
        assert_eq!(step.committed.as_deref(), Some("te1000001"));
        assert!(!step.pass_key);
        assert!(!machine.is_active());
    }

    #[test]
    fn command_modifiers_retype() {
        let mut machine = PostalMachine::new();
        feed(&mut machine, &chars("te10"));
        let cmd_v = KeyInput {
            modifiers: Modifiers {
                command: true,
                ..Modifiers::NONE
            },
            ..ch("v")
        };
        let step = machine.handle(&cmd_v, true, PostalKinds::TE_ONLY).unwrap();
        assert_eq!(step.committed.as_deref(), Some("te10"));
        assert!(step.pass_key);
    }

    #[test]
    fn backspace_shortens_and_closes_the_candidates() {
        let mut machine = PostalMachine::new();
        feed(&mut machine, &chars("te1000001"));
        let step = feed(&mut machine, &[plain(KeyKind::Backspace)]);
        assert!(hides_candidates(&step));
        assert!(!step.unwrap().pass_key);
        assert_eq!(
            machine.phase(),
            &PostalPhase::Typing(PostalKind::Te, "te100000".into())
        );
        // もう一度 7 桁目を打てば開き直す
        let reopened = feed(&mut machine, &[ch("1")]).unwrap();
        assert_eq!(reopened.lookup.unwrap().code, CODE);
    }

    #[test]
    fn backspace_empties_and_cancels() {
        let mut machine = PostalMachine::new();
        let step = feed(&mut machine, &[ch("t"), plain(KeyKind::Backspace)]).unwrap();
        assert!(!step.pass_key);
        assert!(!machine.is_active());
    }

    #[test]
    fn non_matching_key_retypes() {
        for text in ["to", "tex", "te1a"] {
            let mut machine = PostalMachine::new();
            let step = feed(&mut machine, &chars(text)).unwrap();
            let expected: String = text.chars().take(text.chars().count() - 1).collect();
            assert_eq!(step.committed.as_deref(), Some(expected.as_str()));
            assert!(step.pass_key);
            assert!(!machine.is_active());
        }
    }

    #[test]
    fn lookup_adds_addresses_when_the_state_is_the_same() {
        let mut machine = PostalMachine::new();
        let request = feed(&mut machine, &chars("te1000001"))
            .unwrap()
            .lookup
            .unwrap();
        let step = machine.lookup_arrived(&request, &[CHIYODA.to_string()]);
        assert_eq!(
            rows(&machine).unwrap().last().map(String::as_str),
            Some(CHIYODA)
        );
        assert!(shows_candidates(&step));
        // 同じ返事がもう一度来ても足さない
        assert_eq!(
            machine.lookup_arrived(&request, &[CHIYODA.to_string()]),
            None
        );
        assert_eq!(rows(&machine).unwrap().len(), 4);
    }

    #[test]
    fn empty_addresses_add_nothing() {
        let mut machine = PostalMachine::new();
        let request = feed(&mut machine, &chars("te1000001"))
            .unwrap()
            .lookup
            .unwrap();
        assert_eq!(machine.lookup_arrived(&request, &[]), None);
        assert_eq!(rows(&machine).unwrap().len(), 3);
    }

    #[test]
    fn stale_lookup_is_dropped() {
        let mut machine = PostalMachine::new();
        let first = feed(&mut machine, &chars("te1000001"))
            .unwrap()
            .lookup
            .unwrap();
        feed(&mut machine, &[plain(KeyKind::Escape)]);
        assert_eq!(machine.lookup_arrived(&first, &["x".to_string()]), None);
        // 同じ番号を打ち直しても、前の問い合わせの返事は捨てる
        let second = feed(&mut machine, &chars("te1000001"))
            .unwrap()
            .lookup
            .unwrap();
        assert!(second.code == first.code && second.serial != first.serial);
        assert_eq!(machine.lookup_arrived(&first, &["x".to_string()]), None);
        assert_eq!(rows(&machine).unwrap().len(), 3);
        assert!(
            machine
                .lookup_arrived(&second, &["y".to_string()])
                .is_some()
        );
    }

    #[test]
    fn addresses_can_be_selected_and_committed() {
        let mut machine = PostalMachine::new();
        let request = feed(&mut machine, &chars("te1000001"))
            .unwrap()
            .lookup
            .unwrap();
        machine.lookup_arrived(&request, &[CHIYODA.to_string()]);
        feed(&mut machine, &[plain(KeyKind::Up)]);
        assert_eq!(
            committed(feed(&mut machine, &[plain(KeyKind::Enter)])).as_deref(),
            Some(CHIYODA)
        );
    }

    #[test]
    fn flush_commits_the_typed_text_without_passing_the_key() {
        let mut machine = PostalMachine::new();
        assert_eq!(machine.flush(), None);
        feed(&mut machine, &chars("te10"));
        let typing = machine.flush().unwrap();
        assert_eq!(typing.committed.as_deref(), Some("te10"));
        assert!(!typing.pass_key);
        feed(&mut machine, &chars("te1000001"));
        let choosing = machine.flush().unwrap();
        assert_eq!(choosing.committed.as_deref(), Some("te1000001"));
        assert!(choosing.actions.contains(&ShellAction::HideCandidates));
    }

    #[test]
    fn candidate_pages_follow_the_selection() {
        let mut machine = PostalMachine::new();
        let request = feed(&mut machine, &chars("te1000001"))
            .unwrap()
            .lookup
            .unwrap();
        let addresses: Vec<String> = (0..10).map(|i| format!("住所{i}")).collect();
        machine.lookup_arrived(&request, &addresses);
        // 0..=12 の 13 行。8 → 9 でページが替わる
        for _ in 0..8 {
            feed(&mut machine, &[plain(KeyKind::Down)]);
        }
        let step = feed(&mut machine, &[plain(KeyKind::Down)]).unwrap();
        let ShellAction::ShowCandidates(panel) = &step.actions[1] else {
            panic!("ページが替わらない: {:?}", step.actions);
        };
        assert_eq!(panel.selected, 0);
        assert_eq!(panel.candidates.len(), 4);
        assert_eq!(panel.reserved_rows, 9);
    }

    // Unicode 表記（ADR 0032 §9.1。issue #28）。`u+` + 16 進 4〜6 桁

    #[test]
    fn not_taken_when_the_kind_is_off() {
        let mut machine = PostalMachine::new();
        assert_eq!(machine.handle(&ch("u"), true, PostalKinds::TE_ONLY), None);
        assert!(!machine.is_active());
    }

    #[test]
    fn plus_is_required_and_ordinary_words_pass() {
        // 2 文字目で形から外れて打ち直しになる（語の頭だけで入るので、その先は呼ぶ側が can_enter = false にする）
        for word in ["use", "unit", "ubuntu", "u3042"] {
            let mut machine = PostalMachine::new();
            let keys: Vec<KeyInput> = chars(word).into_iter().take(2).collect();
            let step = feed_unicode(&mut machine, &keys).unwrap();
            assert!(step.pass_key, "{word}");
            assert_eq!(step.committed.as_deref(), Some("u"), "{word}");
            assert!(!machine.is_active(), "{word}");
        }
    }

    #[test]
    fn opens_on_the_fourth_digit_without_underline_and_asks_no_lookup() {
        let mut machine = PostalMachine::new();
        let before = feed_unicode(&mut machine, &chars("u+304"));
        assert!(!shows_candidates(&before));
        assert!(!underlined(&before));
        let step = feed_unicode(&mut machine, &[ch("2")]);
        assert!(shows_candidates(&step));
        assert!(!underlined(&step));
        assert_eq!(step.unwrap().lookup, None);
        assert_eq!(rows(&machine), Some(vec!["u+3042".into(), "あ".into()]));
    }

    #[test]
    fn updates_on_the_fifth_and_sixth_digits_then_retypes() {
        let mut machine = PostalMachine::new();
        let five = feed_unicode(&mut machine, &chars("u+01F60"));
        assert!(shows_candidates(&five));
        assert_eq!(
            rows(&machine),
            Some(vec!["u+01F60".into(), "\u{01F60}".into()])
        );
        let six = feed_unicode(&mut machine, &[ch("0")]);
        assert!(shows_candidates(&six));
        assert_eq!(rows(&machine), Some(vec!["u+01F600".into(), "😀".into()]));
        let seventh = feed_unicode(&mut machine, &[ch("0")]).unwrap();
        assert!(seventh.pass_key);
        assert_eq!(seventh.committed.as_deref(), Some("u+01F600"));
    }

    #[test]
    fn above_10ffff_surrogates_controls_and_noncharacters_are_not_offered() {
        for typed in ["u+110000", "u+D800", "u+000A", "u+FFFF", "u+FDD0"] {
            let mut machine = PostalMachine::new();
            let step = feed_unicode(&mut machine, &chars(typed));
            assert!(!shows_candidates(&step), "{typed}");
            assert_eq!(rows(&machine), None, "{typed}");
        }
    }

    #[test]
    fn upper_and_lower_case_hex_both_work() {
        let mut machine = PostalMachine::new();
        feed_unicode(&mut machine, &chars("u+30aB"));
        assert_eq!(
            rows(&machine).unwrap().last().map(String::as_str),
            Some("\u{30AB}")
        );
    }

    #[test]
    fn only_down_up_and_enter_select() {
        let mut machine = PostalMachine::new();
        feed_unicode(&mut machine, &chars("u+3042"));
        let enter = feed_unicode(&mut machine, &[plain(KeyKind::Enter)]);
        assert_eq!(committed(enter).as_deref(), Some("u+3042"));
        let mut other = PostalMachine::new();
        feed_unicode(&mut other, &chars("u+3042"));
        feed_unicode(&mut other, &[plain(KeyKind::Down)]);
        let pick = feed_unicode(&mut other, &[plain(KeyKind::Enter)]);
        assert_eq!(committed(pick).as_deref(), Some("あ"));
    }

    #[test]
    fn space_tab_and_non_hex_commit_the_typed_text_and_pass_the_key() {
        for key in [plain(KeyKind::Space), ch("\t"), ch("z"), ch("2")] {
            let mut machine = PostalMachine::new();
            feed_unicode(&mut machine, &chars("u+01F600"));
            let step = feed_unicode(&mut machine, &[key]).unwrap();
            assert!(step.pass_key);
            assert_eq!(step.committed.as_deref(), Some("u+01F600"));
        }
    }

    #[test]
    fn unicode_backspace_steps_back_and_closes_below_four_digits() {
        let mut machine = PostalMachine::new();
        feed_unicode(&mut machine, &chars("u+30420"));
        let five = feed_unicode(&mut machine, &[plain(KeyKind::Backspace)]);
        assert!(shows_candidates(&five));
        assert_eq!(rows(&machine), Some(vec!["u+3042".into(), "あ".into()]));
        let three = feed_unicode(&mut machine, &[plain(KeyKind::Backspace)]);
        assert!(hides_candidates(&three));
        assert_eq!(rows(&machine), None);
    }
}
