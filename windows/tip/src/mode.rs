//! 入力モード（日本語・英語）の表示と切り替えの写し（純粋。ADR 0040 §7 の W4、ADR 0026）。
//!
//! **判断は `shell-core`**（いつ切り替わるか、切り替えで何を確定するか、英語のモードで打鍵を素通しにするか）。
//! ここにあるのは、OS の側の約束事の表だけ。
//!
//! - macOS は OS がモードを選び（`client.selectMode`）、`InputModeChanged` で知らせてくる。Windows にはその相手がいないので、
//!   TIP が OS の役を引き受ける: `shell-core` が頼んだモード（`Shell::take_mode_request`）を、そのまま通知として返す（`settle`）。
//! - macOS の OS は、英数キー・かなキーでモードを替えてから、そのキーを殻に届ける。Windows では、キーを受けた TIP が
//!   同じことをする（英数 → 英語、かな → 日本語。`requested_by_event`）。
//! - 今のモードは、compartment の値（`ModeValues`）と、言語バーのボタンの文字（`label`）で OS に伝える。
//!   OS 側からの変更（言語バーのクリック、IME のオン・オフ）は、compartment の値から読む（`from_open`、`from_conversion`）。

use ikari_shell_core::english::InputMode;
use ikari_shell_core::{KeyKind, Shell, ShellAction, ShellEvent};

use crate::tsf_op::TsfOp;

/// 変換モードの値の bit（`imm.h` の `IME_CMODE_*`。`TF_CONVERSIONMODE_*` と同じ値）。
pub mod conversion {
    /// 日本語（かな）。立っていなければ英数
    pub const NATIVE: u32 = 0x0001;
    /// 全角
    pub const FULLSHAPE: u32 = 0x0008;
    /// ローマ字入力
    pub const ROMAN: u32 = 0x0010;
    /// 英数（半角。bit が立たない）
    pub const ALPHANUMERIC: u32 = 0;
}

/// モードを OS に伝えるときの compartment の値。
///
/// - 日本語: IME オン、変換モードは「ひらがな・全角・ローマ字」（MS-IME のひらがなと同じ）
/// - 英語: IME オフ、変換モードは英数（MS-IME の「A」と同じ。打鍵をそのままアプリに渡す）
///
/// 読む側（`from_open`、`from_conversion`）は、どちらの値からも同じモードに戻る。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModeValues {
    /// `GUID_COMPARTMENT_KEYBOARD_OPENCLOSE`（0 か 1）
    pub open: bool,
    /// `GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION`
    pub conversion: u32,
}

/// モードに対応する compartment の値。
pub fn values_of(mode: InputMode) -> ModeValues {
    match mode {
        InputMode::Japanese => ModeValues {
            open: true,
            conversion: conversion::NATIVE | conversion::FULLSHAPE | conversion::ROMAN,
        },
        InputMode::English => ModeValues {
            open: false,
            conversion: conversion::ALPHANUMERIC,
        },
    }
}

/// `OPENCLOSE` の値から。閉じていれば英語（打鍵をそのまま渡す）、開いていれば日本語。
pub fn from_open(open: bool) -> InputMode {
    if open {
        InputMode::Japanese
    } else {
        InputMode::English
    }
}

/// 変換モードの値から。`NATIVE` が立っていれば日本語、なければ英語。
pub fn from_conversion(value: u32) -> InputMode {
    if value & conversion::NATIVE != 0 {
        InputMode::Japanese
    } else {
        InputMode::English
    }
}

/// 言語バーのボタンの文字。
pub fn label(mode: InputMode) -> &'static str {
    match mode {
        InputMode::Japanese => "あ",
        InputMode::English => "A",
    }
}

/// 言語バーのボタンの説明（ツールチップ）。
pub fn tooltip(mode: InputMode) -> &'static str {
    match mode {
        InputMode::Japanese => "ikari: 日本語",
        InputMode::English => "ikari: 英語",
    }
}

/// このイベントが、OS の役としてモードを替えるものなら、行き先。英数キーは英語、かなキーは日本語
/// （`shell-core` は、この 2 つのキーを「OS がもう替えた」ものとして受け止めるだけにしている）。
pub fn requested_by_event(event: &ShellEvent) -> Option<InputMode> {
    match event {
        ShellEvent::Key(input) => match input.key {
            KeyKind::Eisu => Some(InputMode::English),
            KeyKind::Kana => Some(InputMode::Japanese),
            _ => None,
        },
        _ => None,
    }
}

/// `dispatch` の後始末: `shell-core` が頼んだモード（なければ `fallback`）を、OS の通知として `shell-core` に返す。
/// 返った動作（今の表示の確定など）は、頼んだ動作の後ろに足す。
pub fn settle(shell: &mut Shell, fallback: Option<InputMode>) -> Vec<ShellAction> {
    match shell.take_mode_request().or(fallback) {
        Some(mode) => shell.input_mode_changed(mode.mode_id()),
        None => Vec::new(),
    }
}

/// OS に表示したモードの記憶。変わったときだけ表示の操作を出す（打鍵のたびに書き込まない）。
#[derive(Debug, Clone, Copy, Default)]
pub struct ModeTracker {
    shown: Option<InputMode>,
}

impl ModeTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// 今のモードを表示する操作。まだ何も表示していない、または前と違うときだけ `Some`。
    pub fn sync(&mut self, current: InputMode) -> Option<TsfOp> {
        if self.shown == Some(current) {
            return None;
        }
        self.shown = Some(current);
        Some(TsfOp::ShowInputMode { mode: current })
    }

    /// 記憶を捨てる（`Shell` を作り直したとき。次の `sync` で必ず表示する）
    pub fn reset(&mut self) {
        self.shown = None;
    }
}

#[cfg(test)]
mod tests;
