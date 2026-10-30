//! ショートコードの状態機械（ADR 0027 §3.3、§3.4、§9.1。今の `ShortcodeMachine.swift`、`ShortcodeTable.swift`、
//! `JisKeyAscii.swift`。レーン L-A1）。純粋（OS にもエンジンにも依存しない）。
//!
//! 責任の分け方（§9.1）: `ShortcodeMachine` は「今の状態と、キーを受けたときの結果」だけを持つ。
//! 打ち直し（`retype`）と確定後のキーの扱い（`ShortcodeAfterCommit`）の実行は、呼ぶ側
//! （日本語のモードは `machine`、英語のモードは `english`）が持つ。

use std::sync::Arc;

use crate::diag::DiagPickKind;
use crate::english::is_printable;
use crate::small_letters;
use crate::types::{KeyInput, KeyKind};

pub mod jis_ascii;
pub mod table;

pub use table::{ShortcodeEntry, ShortcodeTable};

/// 候補ウィンドウの 1 ページの行数（数字キー 1〜9。ADR 0014）。選択が動くとページも動く。
pub const CANDIDATE_PAGE_SIZE: usize = 9;
/// Tab（`.character` で届くときの文字）。
const TAB: &str = "\t";
/// Shift+Tab（`NSBackTabCharacter`）が `.character` で届くときの文字。
const BACK_TAB: &str = "\u{19}";
/// 英語のモードで、前方一致が切れても保つ名前の長さの上限（表の名前の最大長。ADR 0027 §3.3、B52）。
pub const MAX_NAME_LENGTH: usize = 64;
const COLON: char = ':';

/// モードごとの差（§3.1 の表）。名前の文字の読み方と、無視したときのキーの扱いを決める。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShortcodeMode {
    /// 英語のモード（ADR 0026）。届いた文字を読む。打ち直しと無視は素通し
    English,
    /// 日本語のモードのローマ字入力。届いた文字を読む。打ち直しは idle からの処理に流す
    Romaji,
    /// 日本語のモードのかな入力（JIS）。`Jis` のキーを `jis_ascii` の刻印で読む。届いたのが文字なら文字
    KanaJis,
}

/// 候補の 1 行。`label` は絵文字の行の右に薄く出す名前（`:eyes:`）。「そのままの入力」の行は `None`（§3.4）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortcodeCandidate {
    pub surface: String,
    pub label: Option<String>,
}

impl ShortcodeCandidate {
    pub fn new(surface: impl Into<String>, label: Option<String>) -> Self {
        Self {
            surface: surface.into(),
            label,
        }
    }
}

/// `shortcode(name)`: 打っている途中（変換中の文字列は `:` + name。§3.3）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ShortcodeTyping {
    /// `:` を除いた、ここまでの名前
    pub name: String,
    /// 入ってから受けたキー（入りの `:` を含む。打ち直しで流し直す。§3.3）
    pub keys: Vec<KeyInput>,
}

impl ShortcodeTyping {
    /// 変換中の文字列（`:` + name）。
    pub fn marked(&self) -> String {
        format!("{COLON}{}", self.name)
    }
}

/// `shortcodeChoice(candidates, selected)`: 候補を出している（変換中の文字列は選んでいる候補。§3.4）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortcodeChoosing {
    pub name: String,
    /// 見えていた半角の入力（`:eyes:`、前方一致のときは `:ey`）。候補の 0 番の `surface`（Q16）
    pub typed: String,
    /// 0 番は「そのままの入力」（`typed`）。§3.5 の並び
    pub candidates: Vec<ShortcodeCandidate>,
    pub selected: usize,
    /// 入ってから受けたキー（BackSpace・Esc で `shortcode(name)` に戻るため）
    pub keys: Vec<KeyInput>,
}

impl ShortcodeChoosing {
    /// 選んでいる候補（変換中の文字列）。範囲外なら `typed`。
    pub fn marked(&self) -> &str {
        self.candidates
            .get(self.selected)
            .map_or(self.typed.as_str(), |c| c.surface.as_str())
    }

    /// 選んでいる候補のページ（1 ページは 9 件。数字キーはそのページの番号）。
    pub fn page(&self) -> usize {
        self.selected / CANDIDATE_PAGE_SIZE
    }
}

/// 今のショートコードの状態。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ShortcodePhase {
    #[default]
    Inactive,
    Typing(ShortcodeTyping),
    Choosing(ShortcodeChoosing),
}

/// 確定したあとの、今のキーの扱い（§3.4 の表の「キーをアプリへ」「新しい合成」）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShortcodeAfterCommit {
    /// キーを消費する（絵文字を選んだ Enter、Esc、日本語のモードの数字キーでの確定など）
    Consume,
    /// キーをアプリに渡す（英語のモードの Space・数字・文字、第 1 候補のままの Enter。矢印・修飾キー付き）
    PassKey,
    /// そのキーで新しい合成を始める（日本語のモードの印字できる文字。`continueAfterCommit`）
    ContinueWith(KeyInput),
}

/// 確定の指示。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortcodeCommit {
    /// 確定する文字列（絵文字、またはそのままの入力）
    pub text: String,
    /// `CommitText` の読み。`:` + 名前（§3.7）
    pub reading: String,
    pub after: ShortcodeAfterCommit,
}

/// キーを受けた結果。呼ぶ側はこれを `ShellAction` に直す（`machine`）か、英語の経路の操作に直す（`english`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShortcodeOutcome {
    /// ショートコードに関係ないキー。呼ぶ側が今までどおり処理する（`phase` は `Inactive` のまま）
    NotHandled,
    /// 入った、または名前が変わった。変換中の文字列を `marked` にする。`hides_candidates` は候補を閉じて戻ったとき
    Typing {
        marked: String,
        hides_candidates: bool,
    },
    /// `:` だけで BackSpace。変換中の文字列を消して元の状態に戻る（キーは消費）
    Cancelled,
    /// 候補を出した（初めて）。変換中の文字列は `ShortcodeChoosing::marked`
    ShowChoices(ShortcodeChoosing),
    /// 候補の選択が動いた
    MoveChoice(ShortcodeChoosing),
    /// 確定する
    Commit(ShortcodeCommit),
    /// 打ち直す（§3.3）。`keys` は入ってから受けたキー（入りの `:` から今のキーまで。最後が今のキー。
    /// 日本語のモードの Esc は「合成に戻る」ので、今のキーを含めない）。
    /// `literal` は今のキーを除く表示（`:ey`）。英語のモードは `literal` を確定し、今のキーだけをアプリに渡す。
    /// 日本語のモードは `keys` を、ショートコードを切った idle からの処理に順に流す
    Retype {
        keys: Vec<KeyInput>,
        literal: String,
    },
}

/// キーを受けた結果と、そのあとの状態。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortcodeResult {
    pub outcome: ShortcodeOutcome,
    pub phase: ShortcodePhase,
}

/// ショートコードの状態機械。状態は `phase` が全部。表とモードは作るときに渡す。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortcodeMachine {
    pub table: Arc<ShortcodeTable>,
    pub mode: ShortcodeMode,
    /// 上付き・下付きの小さい文字の候補（`:m2:` → m²。`emoji.small_letters`。#264）。英語のモードの呼び出しが、キーごとに設定を写す
    pub small_letters: bool,
    phase: ShortcodePhase,
    /// 直近に確定した候補の種類（診断ログの `pick_kind`。ADR 0034）。確定のたびに更新する
    last_pick_kind: DiagPickKind,
}

impl ShortcodeMachine {
    pub fn new(table: Arc<ShortcodeTable>, mode: ShortcodeMode) -> Self {
        Self {
            table,
            mode,
            small_letters: false,
            phase: ShortcodePhase::Inactive,
            last_pick_kind: DiagPickKind::Literal,
        }
    }

    pub fn phase(&self) -> &ShortcodePhase {
        &self.phase
    }

    pub fn last_pick_kind(&self) -> DiagPickKind {
        self.last_pick_kind
    }

    /// 状態に入っている（`Inactive` でない）。
    pub fn is_active(&self) -> bool {
        self.phase != ShortcodePhase::Inactive
    }

    /// キーを 1 つ受ける。
    /// - `can_enter`: 入ってよいか。呼ぶ側が §3.2 の共通の条件（secure でない、`emoji.shortcode` が on、
    ///   前面のアプリが除外にない、Ctrl・Option・Command なし、キーリピートでない）と、モードごとの条件
    ///   （英語は語の頭、日本語は idle）を判定して渡す。表が空のときは `can_enter` にかかわらず入らない（機械が見る）。
    ///   `is_active` の間は使わない
    /// - 返り値の `phase` は `self.phase` にも反映される
    pub fn handle(&mut self, input: &KeyInput, can_enter: bool) -> ShortcodeResult {
        let outcome = match self.phase.clone() {
            ShortcodePhase::Inactive => self.enter(input, can_enter),
            ShortcodePhase::Typing(typing) => self.handle_typing(input, typing),
            ShortcodePhase::Choosing(choosing) => self.handle_choosing(input, choosing),
        };
        ShortcodeResult {
            outcome,
            phase: self.phase.clone(),
        }
    }

    /// モードの切り替え、secure になる、`deactivateServer`、クリック（`commitComposition`）のとき。
    /// 今の表示のまま確定する指示を返し、`Inactive` に戻る。`Inactive` なら `None`（§3.3、§3.4 の最後の行）。
    pub fn flush(&mut self) -> Option<ShortcodeCommit> {
        let phase = std::mem::take(&mut self.phase);
        match phase {
            ShortcodePhase::Inactive => None,
            ShortcodePhase::Typing(typing) => {
                let marked = typing.marked();
                Some(ShortcodeCommit {
                    text: marked.clone(),
                    reading: marked,
                    after: ShortcodeAfterCommit::Consume,
                })
            }
            ShortcodePhase::Choosing(choosing) => {
                let index = choosing.selected;
                Some(self.commit_value(&choosing, index, ShortcodeAfterCommit::Consume))
            }
        }
    }

    /// 何も確定せずに状態を捨てる（呼ぶ側が表示を消したあとなど）。
    pub fn reset(&mut self) {
        self.phase = ShortcodePhase::Inactive;
    }

    // MARK: 入り

    fn enter(&mut self, input: &KeyInput, can_enter: bool) -> ShortcodeOutcome {
        if !can_enter
            || self.table.is_empty()
            || has_command_modifiers(input)
            || !self.is_entry_key(input)
        {
            return ShortcodeOutcome::NotHandled;
        }
        let typing = ShortcodeTyping {
            name: String::new(),
            keys: vec![input.clone()],
        };
        let marked = typing.marked();
        self.phase = ShortcodePhase::Typing(typing);
        ShortcodeOutcome::Typing {
            marked,
            hides_candidates: false,
        }
    }

    /// 入りのキー。かな入力は Shift+けのキー（Shift なしは「け」のまま）。どのモードでも `:` の文字。
    fn is_entry_key(&self, input: &KeyInput) -> bool {
        match &input.key {
            KeyKind::Character { text } => text.chars().eq(std::iter::once(COLON)),
            KeyKind::Jis { code } => {
                self.mode == ShortcodeMode::KanaJis
                    && *code == jis_ascii::COLON_KEY
                    && input.modifiers.shift
            }
            _ => false,
        }
    }

    // MARK: shortcode(name)（§3.3）

    fn handle_typing(&mut self, input: &KeyInput, typing: ShortcodeTyping) -> ShortcodeOutcome {
        if has_command_modifiers(input) {
            return self.retype(&typing, input, true);
        }
        let english = self.mode == ShortcodeMode::English;
        match &input.key {
            KeyKind::Backspace => {
                if typing.name.is_empty() {
                    self.phase = ShortcodePhase::Inactive;
                    return ShortcodeOutcome::Cancelled;
                }
                let mut shortened = typing;
                shortened.name.pop();
                shortened.keys.pop();
                let marked = shortened.marked();
                self.phase = ShortcodePhase::Typing(shortened);
                return ShortcodeOutcome::Typing {
                    marked,
                    hides_candidates: false,
                };
            }
            // 日本語のモードは「合成に戻る」ので Esc を流さない。英語のモードは Esc をアプリへ渡す
            KeyKind::Escape => return self.retype(&typing, input, english),
            KeyKind::Space => {
                return if english {
                    self.retype(&typing, input, true)
                } else {
                    self.show_prefix(typing, input)
                };
            }
            KeyKind::Down => return self.show_prefix(typing, input),
            KeyKind::Character { text } if text == TAB => return self.show_prefix(typing, input),
            _ => {}
        }
        let Some(character) = self.ascii_character(input) else {
            return self.retype(&typing, input, true);
        };
        if character == COLON {
            return self.close_colon(typing, input);
        }
        if !is_name_character(character) {
            return self.retype(&typing, input, true);
        }
        let mut extended = typing.clone();
        extended.name.push(character);
        extended.keys.push(input.clone());
        if english {
            // 英語のモード（B52）: 前方一致が切れても、名前の文字が続く間は状態を保つ（Backspace で直せる）。長さの上限で打ち直す
            if extended.name.chars().count() > MAX_NAME_LENGTH {
                return self.retype(&typing, input, true);
            }
        } else if self.table.prefix(&extended.name).is_empty() {
            return self.retype(&typing, input, true);
        }
        let marked = extended.marked();
        self.phase = ShortcodePhase::Typing(extended);
        ShortcodeOutcome::Typing {
            marked,
            hides_candidates: false,
        }
    }

    fn retype(
        &mut self,
        typing: &ShortcodeTyping,
        input: &KeyInput,
        including_key: bool,
    ) -> ShortcodeOutcome {
        self.phase = ShortcodePhase::Inactive;
        let mut keys = typing.keys.clone();
        if including_key {
            keys.push(input.clone());
        }
        ShortcodeOutcome::Retype {
            keys,
            literal: typing.marked(),
        }
    }

    /// 閉じの `:`。名前が完全に一致したときだけ候補を出す。
    fn close_colon(&mut self, typing: ShortcodeTyping, input: &KeyInput) -> ShortcodeOutcome {
        let found: Vec<String> = match self.table.exact(&typing.name) {
            Some(emoji) => vec![emoji.to_string()],
            None if self.small_letters => small_letters::candidates(&typing.name),
            None => Vec::new(),
        };
        if found.is_empty() {
            return self.retype(&typing, input, true);
        }
        let typed = format!("{}{COLON}", typing.marked());
        let mut candidates = vec![ShortcodeCandidate::new(typed.clone(), None)];
        candidates.extend(
            found
                .into_iter()
                .map(|surface| ShortcodeCandidate::new(surface, Some(typed.clone()))),
        );
        let mut keys = typing.keys;
        keys.push(input.clone());
        let choosing = ShortcodeChoosing {
            name: typing.name,
            typed,
            candidates,
            selected: 0,
            keys,
        };
        self.phase = ShortcodePhase::Choosing(choosing.clone());
        ShortcodeOutcome::ShowChoices(choosing)
    }

    /// Tab、↓（日本語のモードは Space も）。前方一致の候補（§3.5）。名前が空なら打ち直す。
    fn show_prefix(&mut self, typing: ShortcodeTyping, input: &KeyInput) -> ShortcodeOutcome {
        let matched = self.table.prefix(&typing.name);
        if matched.is_empty() {
            return self.retype(&typing, input, true);
        }
        let mut candidates = vec![ShortcodeCandidate::new(typing.marked(), None)];
        candidates.extend(matched.into_iter().map(|entry| {
            ShortcodeCandidate::new(
                entry.emoji.clone(),
                Some(format!("{COLON}{}{COLON}", entry.name)),
            )
        }));
        let choosing = ShortcodeChoosing {
            typed: typing.marked(),
            name: typing.name,
            candidates,
            selected: 0,
            keys: typing.keys,
        };
        self.phase = ShortcodePhase::Choosing(choosing.clone());
        ShortcodeOutcome::ShowChoices(choosing)
    }

    // MARK: shortcodeChoice（§3.4）

    fn handle_choosing(
        &mut self,
        input: &KeyInput,
        choosing: ShortcodeChoosing,
    ) -> ShortcodeOutcome {
        use ShortcodeAfterCommit::{Consume, ContinueWith, PassKey};
        let selected = choosing.selected;
        if has_command_modifiers(input) {
            return self.commit(&choosing, selected, PassKey);
        }
        let english = self.mode == ShortcodeMode::English;
        let shift = input.modifiers.shift;
        match &input.key {
            KeyKind::Down => return self.move_choice(choosing, 1),
            KeyKind::Up => return self.move_choice(choosing, -1),
            KeyKind::Space => {
                if english {
                    return self.commit(&choosing, selected, PassKey);
                }
                return self.move_choice(choosing, if shift { -1 } else { 1 });
            }
            KeyKind::Enter => {
                if english {
                    // 第 1 候補のままの Enter はアプリへ渡す（Slack ならそのまま送信）。絵文字を選んでいたら消費（Q14）
                    let after = if selected == 0 { PassKey } else { Consume };
                    return self.commit(&choosing, selected, after);
                }
                return self.commit(&choosing, selected, Consume);
            }
            KeyKind::Backspace => return self.back_to_typing(choosing),
            KeyKind::Escape => {
                return if english {
                    self.commit(&choosing, 0, Consume)
                } else {
                    self.back_to_typing(choosing)
                };
            }
            KeyKind::Left | KeyKind::Right | KeyKind::Function { .. } => {
                return self.commit(&choosing, selected, PassKey);
            }
            KeyKind::Other => {
                return if english {
                    self.commit(&choosing, selected, PassKey)
                } else {
                    self.move_choice(choosing, 0)
                };
            }
            KeyKind::Character { text } if text == TAB => {
                return self.move_choice(choosing, if shift { -1 } else { 1 });
            }
            KeyKind::Character { text } if text == BACK_TAB => {
                return self.move_choice(choosing, -1);
            }
            KeyKind::Character { .. } | KeyKind::Jis { .. } => {}
        }
        if let Some(number) = self
            .ascii_character(input)
            .and_then(|c| c.to_digit(10))
            .filter(|n| (1..=9).contains(n))
        {
            if english {
                return self.commit(&choosing, selected, PassKey);
            }
            let page_start = selected / CANDIDATE_PAGE_SIZE * CANDIDATE_PAGE_SIZE;
            let index = page_start + number as usize - 1;
            return if index < choosing.candidates.len() {
                self.commit(&choosing, index, Consume)
            } else {
                self.move_choice(choosing, 0)
            };
        }
        if let KeyKind::Character { text } = &input.key
            && !is_printable(text)
        {
            return if english {
                self.commit(&choosing, selected, PassKey)
            } else {
                self.move_choice(choosing, 0)
            };
        }
        // そのほかの印字できる文字。英語のモードはキーをアプリへ、日本語のモードはそのキーで新しい合成
        let after = if english {
            PassKey
        } else {
            ContinueWith(input.clone())
        };
        self.commit(&choosing, selected, after)
    }

    /// 選択を動かす。端では折り返す。`delta` が 0 なら何も動かさない（受け止めるだけ）。
    fn move_choice(&mut self, choosing: ShortcodeChoosing, delta: i64) -> ShortcodeOutcome {
        let mut moved = choosing;
        let count = i64::try_from(moved.candidates.len())
            .unwrap_or(i64::MAX)
            .max(1);
        let selected = i64::try_from(moved.selected).unwrap_or(0);
        moved.selected = usize::try_from((selected + delta).rem_euclid(count)).unwrap_or(0);
        self.phase = ShortcodePhase::Choosing(moved.clone());
        ShortcodeOutcome::MoveChoice(moved)
    }

    /// 「そのままの入力」の行は `Literal`。ほかは、表にある名前の絵文字なら `Emoji`、閉じの `:` で表にない名前から作った
    /// 上付き・下付きの文字は `SmallLetter`（前方一致の候補は閉じの `:` がなく、必ず表の絵文字）。
    fn pick_kind(&self, choosing: &ShortcodeChoosing, index: usize) -> DiagPickKind {
        let has_label = choosing
            .candidates
            .get(index)
            .is_some_and(|c| c.label.is_some());
        if !has_label {
            return DiagPickKind::Literal;
        }
        let closed = choosing.typed.ends_with(COLON);
        if closed && self.table.exact(&choosing.name).is_none() {
            DiagPickKind::SmallLetter
        } else {
            DiagPickKind::Emoji
        }
    }

    fn commit_value(
        &mut self,
        choosing: &ShortcodeChoosing,
        index: usize,
        after: ShortcodeAfterCommit,
    ) -> ShortcodeCommit {
        self.last_pick_kind = self.pick_kind(choosing, index);
        let text = choosing
            .candidates
            .get(index)
            .map_or_else(|| choosing.typed.clone(), |c| c.surface.clone());
        ShortcodeCommit {
            text,
            reading: format!("{COLON}{}", choosing.name),
            after,
        }
    }

    fn commit(
        &mut self,
        choosing: &ShortcodeChoosing,
        index: usize,
        after: ShortcodeAfterCommit,
    ) -> ShortcodeOutcome {
        self.phase = ShortcodePhase::Inactive;
        ShortcodeOutcome::Commit(self.commit_value(choosing, index, after))
    }

    /// 候補を閉じて `shortcode(name)` に戻る。閉じの `:` で出した候補なら、その `:` を消す。
    fn back_to_typing(&mut self, choosing: ShortcodeChoosing) -> ShortcodeOutcome {
        let mut keys = choosing.keys;
        if choosing.typed.ends_with(COLON) {
            keys.pop();
        }
        let typing = ShortcodeTyping {
            name: choosing.name,
            keys,
        };
        let marked = typing.marked();
        self.phase = ShortcodePhase::Typing(typing);
        ShortcodeOutcome::Typing {
            marked,
            hides_candidates: true,
        }
    }

    /// 名前を読むための ASCII 1 文字。届いた文字（1 文字の ASCII）か、かな入力のキーの刻印。
    fn ascii_character(&self, input: &KeyInput) -> Option<char> {
        match &input.key {
            KeyKind::Character { text } => {
                let mut chars = text.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) if c.is_ascii() => Some(c),
                    _ => None,
                }
            }
            KeyKind::Jis { code } if self.mode == ShortcodeMode::KanaJis => {
                jis_ascii::character_for_jis_key(*code, input.modifiers.shift)
            }
            _ => None,
        }
    }
}

fn has_command_modifiers(input: &KeyInput) -> bool {
    input.modifiers.control || input.modifiers.option || input.modifiers.command
}

/// 名前の文字は `a-z 0-9 _ + -`（§3.3）。
pub fn is_name_character(character: char) -> bool {
    character.is_ascii_lowercase()
        || character.is_ascii_digit()
        || matches!(character, '_' | '+' | '-')
}

#[cfg(test)]
mod tests {
    use super::table::tests::sample_table;
    use super::*;
    use crate::types::Modifiers;

    fn machine(mode: ShortcodeMode) -> ShortcodeMachine {
        ShortcodeMachine::new(Arc::new(sample_table()), mode)
    }

    fn key(kind: KeyKind, modifiers: Modifiers) -> KeyInput {
        KeyInput {
            key: kind,
            modifiers,
            is_repeat: false,
            secure: false,
        }
    }

    fn plain(kind: KeyKind) -> KeyInput {
        key(kind, Modifiers::NONE)
    }

    fn ch(text: &str) -> KeyKind {
        KeyKind::Character { text: text.into() }
    }

    fn with(shift: bool, control: bool, command: bool) -> Modifiers {
        Modifiers {
            shift,
            control,
            option: false,
            command,
        }
    }

    /// 打鍵を順に流して、最後の結果を返す
    fn feed(machine: &mut ShortcodeMachine, text: &str) -> ShortcodeResult {
        let mut last = ShortcodeResult {
            outcome: ShortcodeOutcome::NotHandled,
            phase: machine.phase().clone(),
        };
        for (index, character) in text.chars().enumerate() {
            last = machine.handle(&plain(ch(&character.to_string())), index == 0);
        }
        last
    }

    fn colon_key() -> KeyInput {
        plain(ch(":"))
    }

    #[test]
    fn enters_only_when_allowed() {
        let mut denied = machine(ShortcodeMode::English);
        assert_eq!(
            denied.handle(&colon_key(), false).outcome,
            ShortcodeOutcome::NotHandled
        );
        assert!(!denied.is_active());
        let mut empty =
            ShortcodeMachine::new(Arc::new(ShortcodeTable::empty()), ShortcodeMode::English);
        assert_eq!(
            empty.handle(&colon_key(), true).outcome,
            ShortcodeOutcome::NotHandled
        );
        let mut other = machine(ShortcodeMode::English);
        assert_eq!(
            other.handle(&plain(ch("a")), true).outcome,
            ShortcodeOutcome::NotHandled
        );
        let mut allowed = machine(ShortcodeMode::English);
        let result = allowed.handle(&key(ch(":"), with(true, false, false)), true);
        assert_eq!(
            result.outcome,
            ShortcodeOutcome::Typing {
                marked: ":".into(),
                hides_candidates: false
            }
        );
        assert!(allowed.is_active());
    }

    #[test]
    fn kana_enters_on_shift_colon_key_only() {
        let mut m = machine(ShortcodeMode::KanaJis);
        let bare = m.handle(
            &plain(KeyKind::Jis {
                code: jis_ascii::COLON_KEY,
            }),
            true,
        );
        assert_eq!(bare.outcome, ShortcodeOutcome::NotHandled);
        let shifted = m.handle(
            &key(
                KeyKind::Jis {
                    code: jis_ascii::COLON_KEY,
                },
                with(true, false, false),
            ),
            true,
        );
        assert!(matches!(shifted.outcome, ShortcodeOutcome::Typing { .. }));
        // 名前は刻印で読む（e = 26, y = 44）
        let e = m.handle(&plain(KeyKind::Jis { code: 22 + 4 }), false);
        assert!(matches!(e.outcome, ShortcodeOutcome::Typing { ref marked, .. } if marked == ":e"));
        // ローマ字のモードは Jis のキーを読まない
        let mut romaji = machine(ShortcodeMode::Romaji);
        assert_eq!(
            romaji
                .handle(
                    &key(
                        KeyKind::Jis {
                            code: jis_ascii::COLON_KEY
                        },
                        with(true, false, false)
                    ),
                    true
                )
                .outcome,
            ShortcodeOutcome::NotHandled
        );
    }

    #[test]
    fn english_space_and_escape_retype_including_the_key() {
        let mut space = machine(ShortcodeMode::English);
        feed(&mut space, ":ey");
        let result = space.handle(&plain(KeyKind::Space), false);
        let ShortcodeOutcome::Retype { keys, literal } = result.outcome else {
            panic!("retype でない");
        };
        assert_eq!(literal, ":ey");
        assert_eq!(keys.last(), Some(&plain(KeyKind::Space)));
        assert_eq!(keys.len(), 4);
        assert!(!space.is_active());

        let mut escape = machine(ShortcodeMode::English);
        feed(&mut escape, ":ey");
        let ShortcodeOutcome::Retype { keys, .. } =
            escape.handle(&plain(KeyKind::Escape), false).outcome
        else {
            panic!("retype でない");
        };
        assert_eq!(keys.last(), Some(&plain(KeyKind::Escape)));
    }

    #[test]
    fn japanese_escape_retypes_without_the_key() {
        let mut romaji = machine(ShortcodeMode::Romaji);
        feed(&mut romaji, ":ey");
        let ShortcodeOutcome::Retype { keys, .. } =
            romaji.handle(&plain(KeyKind::Escape), false).outcome
        else {
            panic!("retype でない");
        };
        assert_eq!(keys.len(), 3);
        assert!(!keys.contains(&plain(KeyKind::Escape)));
    }

    #[test]
    fn english_tab_shows_prefix_candidates() {
        let mut english = machine(ShortcodeMode::English);
        feed(&mut english, ":ey");
        let result = english.handle(&plain(ch("\t")), false);
        let ShortcodeOutcome::ShowChoices(choosing) = result.outcome else {
            panic!("候補が出ていない");
        };
        assert_eq!(choosing.candidates.len(), 4);
    }

    #[test]
    fn english_choosing_space_digit_and_letter_commit_and_pass_the_key() {
        for input in [
            plain(KeyKind::Space),
            plain(ch("1")),
            plain(ch("x")),
            plain(KeyKind::Left),
        ] {
            let mut english = machine(ShortcodeMode::English);
            feed(&mut english, ":eyes:");
            english.handle(&plain(KeyKind::Down), false);
            let result = english.handle(&input, false);
            assert_eq!(
                result.outcome,
                ShortcodeOutcome::Commit(ShortcodeCommit {
                    text: "👀".into(),
                    reading: ":eyes".into(),
                    after: ShortcodeAfterCommit::PassKey
                })
            );
            assert!(!english.is_active());
        }
    }

    fn commit(text: &str, after: ShortcodeAfterCommit) -> ShortcodeOutcome {
        ShortcodeOutcome::Commit(ShortcodeCommit {
            text: text.into(),
            reading: ":eyes".into(),
            after,
        })
    }

    #[test]
    fn english_enter_passes_only_when_the_literal_is_selected() {
        let mut literal = machine(ShortcodeMode::English);
        feed(&mut literal, ":eyes:");
        assert_eq!(
            literal.handle(&plain(KeyKind::Enter), false).outcome,
            commit(":eyes:", ShortcodeAfterCommit::PassKey)
        );
        let mut emoji = machine(ShortcodeMode::English);
        feed(&mut emoji, ":eyes:");
        emoji.handle(&plain(KeyKind::Down), false);
        assert_eq!(
            emoji.handle(&plain(KeyKind::Enter), false).outcome,
            commit("👀", ShortcodeAfterCommit::Consume)
        );
    }

    #[test]
    fn english_escape_commits_the_literal_and_consumes() {
        let mut english = machine(ShortcodeMode::English);
        feed(&mut english, ":eyes:");
        english.handle(&plain(KeyKind::Down), false);
        assert_eq!(
            english.handle(&plain(KeyKind::Escape), false).outcome,
            commit(":eyes:", ShortcodeAfterCommit::Consume)
        );
    }

    #[test]
    fn japanese_keys_continue_or_consume() {
        let mut letter = machine(ShortcodeMode::Romaji);
        feed(&mut letter, ":eyes:");
        let k = plain(ch("k"));
        assert_eq!(
            letter.handle(&k, false).outcome,
            commit(":eyes:", ShortcodeAfterCommit::ContinueWith(k))
        );
        let mut enter = machine(ShortcodeMode::Romaji);
        feed(&mut enter, ":eyes:");
        assert_eq!(
            enter.handle(&plain(KeyKind::Enter), false).outcome,
            commit(":eyes:", ShortcodeAfterCommit::Consume)
        );
    }

    #[test]
    fn japanese_digit_picks_within_the_page_and_ignores_out_of_range() {
        let mut m = machine(ShortcodeMode::Romaji);
        feed(&mut m, ":eyes:");
        // 候補は 0 番（そのままの入力）と 👀 の 2 件。2 で 👀、3 は範囲外で動かさない
        let out_of_range = m.handle(&plain(ch("3")), false);
        assert!(
            matches!(out_of_range.outcome, ShortcodeOutcome::MoveChoice(ref c) if c.selected == 0)
        );
        assert_eq!(
            m.handle(&plain(ch("2")), false).outcome,
            commit("👀", ShortcodeAfterCommit::Consume)
        );
    }

    #[test]
    fn japanese_choosing_moves_with_space_tab_and_wraps() {
        let mut m = machine(ShortcodeMode::Romaji);
        feed(&mut m, ":eyes:");
        let selected = |outcome: ShortcodeOutcome| match outcome {
            ShortcodeOutcome::MoveChoice(c) => c.selected,
            other => panic!("{other:?}"),
        };
        assert_eq!(selected(m.handle(&plain(KeyKind::Space), false).outcome), 1);
        assert_eq!(selected(m.handle(&plain(KeyKind::Space), false).outcome), 0);
        let shift = with(true, false, false);
        assert_eq!(
            selected(m.handle(&key(KeyKind::Space, shift), false).outcome),
            1
        );
        assert_eq!(selected(m.handle(&key(ch("\t"), shift), false).outcome), 0);
        assert_eq!(selected(m.handle(&plain(ch("\u{19}")), false).outcome), 1);
        // 非印字の文字は受け止めるだけ
        assert_eq!(selected(m.handle(&plain(ch("\u{f700}")), false).outcome), 1);
    }

    #[test]
    fn backspace_in_choosing_goes_back_to_typing_and_drops_the_closing_colon() {
        let mut m = machine(ShortcodeMode::Romaji);
        feed(&mut m, ":eyes:");
        let result = m.handle(&plain(KeyKind::Backspace), false);
        assert_eq!(
            result.outcome,
            ShortcodeOutcome::Typing {
                marked: ":eyes".into(),
                hides_candidates: true
            }
        );
        let ShortcodePhase::Typing(typing) = result.phase else {
            panic!("typing でない");
        };
        assert_eq!(typing.keys.len(), 5);
        // 前方一致の候補（閉じの `:` なし）から戻るときはキーを減らさない
        let mut prefix = machine(ShortcodeMode::Romaji);
        feed(&mut prefix, ":ey");
        prefix.handle(&plain(KeyKind::Down), false);
        let ShortcodePhase::Typing(typing) = prefix.handle(&plain(KeyKind::Backspace), false).phase
        else {
            panic!("typing でない");
        };
        assert_eq!(typing.keys.len(), 3);
    }

    #[test]
    fn closing_colon_with_unknown_name_retypes_and_small_letters_add_candidates() {
        let mut off = machine(ShortcodeMode::English);
        let result = feed(&mut off, ":m2:");
        assert!(matches!(result.outcome, ShortcodeOutcome::Retype { .. }));

        let mut on = machine(ShortcodeMode::English);
        on.small_letters = true;
        let ShortcodeOutcome::ShowChoices(choosing) = feed(&mut on, ":m2:").outcome else {
            panic!("候補が出ていない");
        };
        let surfaces: Vec<&str> = choosing
            .candidates
            .iter()
            .map(|c| c.surface.as_str())
            .collect();
        assert_eq!(surfaces, [":m2:", "m²", "㎡"]);
        // 表にある名前はそちらが先で、小さい文字は足さない
        let mut table_wins = machine(ShortcodeMode::English);
        table_wins.small_letters = true;
        let ShortcodeOutcome::ShowChoices(choosing) = feed(&mut table_wins, ":eyes:").outcome
        else {
            panic!("候補が出ていない");
        };
        assert_eq!(choosing.candidates.len(), 2);
    }

    #[test]
    fn pick_kind_follows_the_chosen_row() {
        let mut m = machine(ShortcodeMode::English);
        m.small_letters = true;
        feed(&mut m, ":m2:");
        m.handle(&plain(KeyKind::Down), false);
        m.handle(&plain(KeyKind::Enter), false);
        assert_eq!(m.last_pick_kind(), DiagPickKind::SmallLetter);
        feed(&mut m, ":eyes:");
        m.handle(&plain(KeyKind::Down), false);
        m.handle(&plain(KeyKind::Enter), false);
        assert_eq!(m.last_pick_kind(), DiagPickKind::Emoji);
        feed(&mut m, ":eyes:");
        m.handle(&plain(KeyKind::Enter), false);
        assert_eq!(m.last_pick_kind(), DiagPickKind::Literal);
    }

    #[test]
    fn english_keeps_typing_past_a_dead_prefix_and_japanese_retypes() {
        let mut english = machine(ShortcodeMode::English);
        let result = feed(&mut english, ":zzz");
        assert!(
            matches!(result.outcome, ShortcodeOutcome::Typing { ref marked, .. } if marked == ":zzz")
        );
        let mut japanese = machine(ShortcodeMode::Romaji);
        let result = feed(&mut japanese, ":z");
        assert!(matches!(result.outcome, ShortcodeOutcome::Retype { .. }));
    }

    #[test]
    fn english_name_length_is_capped() {
        let mut english = machine(ShortcodeMode::English);
        let text = format!(":{}", "a".repeat(MAX_NAME_LENGTH));
        let result = feed(&mut english, &text);
        assert!(matches!(result.outcome, ShortcodeOutcome::Typing { .. }));
        let over = english.handle(&plain(ch("a")), false);
        assert!(matches!(over.outcome, ShortcodeOutcome::Retype { .. }));
        assert!(!english.is_active());
    }

    #[test]
    fn backspace_on_the_bare_colon_cancels() {
        let mut m = machine(ShortcodeMode::English);
        feed(&mut m, ":");
        let result = m.handle(&plain(KeyKind::Backspace), false);
        assert_eq!(result.outcome, ShortcodeOutcome::Cancelled);
        assert!(!m.is_active());
    }

    #[test]
    fn flush_commits_whatever_is_shown() {
        let mut idle = machine(ShortcodeMode::English);
        assert_eq!(idle.flush(), None);
        let mut typing = machine(ShortcodeMode::English);
        feed(&mut typing, ":ey");
        assert_eq!(
            typing.flush(),
            Some(ShortcodeCommit {
                text: ":ey".into(),
                reading: ":ey".into(),
                after: ShortcodeAfterCommit::Consume
            })
        );
        assert!(!typing.is_active());
        let mut choosing = machine(ShortcodeMode::English);
        feed(&mut choosing, ":eyes:");
        choosing.handle(&plain(KeyKind::Down), false);
        assert_eq!(
            choosing.flush(),
            Some(ShortcodeCommit {
                text: "👀".into(),
                reading: ":eyes".into(),
                after: ShortcodeAfterCommit::Consume
            })
        );
    }

    #[test]
    fn command_modifiers_retype_while_typing() {
        let mut english = machine(ShortcodeMode::English);
        feed(&mut english, ":e");
        let cmd_c = key(ch("c"), with(false, false, true));
        let ShortcodeOutcome::Retype { keys, literal } = english.handle(&cmd_c, false).outcome
        else {
            panic!("retype でない");
        };
        assert_eq!(literal, ":e");
        assert_eq!(keys.last(), Some(&cmd_c));
    }

    #[test]
    fn reset_drops_the_state() {
        let mut english = machine(ShortcodeMode::English);
        feed(&mut english, ":e");
        english.reset();
        assert_eq!(english.phase(), &ShortcodePhase::Inactive);
    }
}
