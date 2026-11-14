//! デバッグの出力（`OutputDebugStringW`。DebugView などで読む）。
//!
//! **入力された文字列を出さない**（プライバシーの方針）。受けるのは `&'static str`（コードに書いた固定の文）と
//! HRESULT の値だけにして、入力の内容や周辺テキストを渡せない形にしている。

use windows::Win32::System::Diagnostics::Debug::OutputDebugStringW;
use windows_core::{HRESULT, HSTRING};

/// 出力の先頭（DebugView で絞り込むため）
const PREFIX: &str = "ikari-tip: ";

fn write(line: &str) {
    let text = HSTRING::from(line);
    // SAFETY: `text` は NUL で終わる UTF-16（HSTRING が保証する）で、呼び出しの間生きている。
    unsafe { OutputDebugStringW(&text) };
}

/// 固定の文を 1 行出す。
pub fn event(message: &'static str) {
    write(&format!("{PREFIX}{message}\n"));
}

/// 失敗を、場所の文と HRESULT の値で出す（誤りの文面は OS の言語に依存するので出さない）。
pub fn error(message: &'static str, error: &windows_core::Error) {
    hresult(message, error.code());
}

/// compartment の変更の通知 1 件（診断用。#371）。`scope` はどちらの compartment に届いたか（`thread` か `global`）、
/// 種類と結果は固定の文、値は compartment の数値（入力の内容ではない）。
pub fn compartment(
    scope: &'static str,
    kind: &'static str,
    value: Option<i32>,
    outcome: &'static str,
) {
    write(&format!(
        "{PREFIX}OnChange {scope} {kind} value={} {outcome}\n",
        number(value)
    ));
}

fn number(value: Option<i32>) -> String {
    value.map_or_else(|| "none".to_owned(), |v| v.to_string())
}

/// 前回出した入力モードの compartment の値（打鍵のたびに同じ行を出さないため）
static LAST_MODE_VALUES: std::sync::Mutex<Option<[Option<i32>; 4]>> = std::sync::Mutex::new(None);

/// 入力モードの compartment の今の値（スレッドの OPENCLOSE・CONVERSION、グローバルの OPENCLOSE・CONVERSION）を、
/// 前回と違うときだけ 1 行出す（診断用。#371。通知が来なくても値が変わったかを、次の打鍵で見るため）
pub fn mode_values(values: [Option<i32>; 4]) {
    let Ok(mut last) = LAST_MODE_VALUES.lock() else {
        return;
    };
    if *last == Some(values) {
        return;
    }
    *last = Some(values);
    let [thread_open, thread_conv, global_open, global_conv] = values.map(number);
    write(&format!(
        "{PREFIX}mode values thread open={thread_open} conv={thread_conv} global open={global_open} conv={global_conv}\n"
    ));
}

/// HRESULT を出す。
pub fn hresult(message: &'static str, code: HRESULT) {
    // HRESULT は符号つきの 32 ビット。16 進で読むのが通例なので、ビット列のまま出す
    write(&format!("{PREFIX}{message} (hr=0x{:08X})\n", code.0 as u32));
}

/// 入力スコープの 1 つの読み方の結果（診断用。#369。数値と固定の文だけで、欄の内容は持たない）
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScopeOutcome {
    /// 読めた入力スコープの値（空のこともある）
    Scopes(Vec<i32>),
    /// 値が付いていない（VARIANT の型。`VT_EMPTY` は 0）
    NotSet { vt: u16 },
    /// 読めなかった。`step` は失敗した呼び出しの名前
    Failed { step: &'static str, hr: HRESULT },
    /// 先の読み方で読めたので、試していない
    Skipped,
}

impl std::fmt::Display for ScopeOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Scopes(list) => write!(f, "scopes{list:?}"),
            Self::NotSet { vt } => write!(f, "not-set(vt={vt})"),
            Self::Failed { step, hr } => write!(f, "failed({step} hr=0x{:08X})", hr.0 as u32),
            Self::Skipped => f.write_str("skipped"),
        }
    }
}

/// 入力スコープの読み取りの診断の 1 件（#369）
pub struct ScopeDiag<'a> {
    /// 読んだ範囲（`selection` か `start`）。範囲が取れなければ `none`
    pub range: &'static str,
    /// `ITfContext::GetAppProperty`（アプリのプロパティ。Mozc と同じ読み方）
    pub app: &'a ScopeOutcome,
    /// `ITfContext::GetProperty`（以前の読み方。TIP のプロパティの置き場所）
    pub tip: &'a ScopeOutcome,
    /// 文脈の `GUID_COMPARTMENT_KEYBOARD_DISABLED`・`GUID_COMPARTMENT_EMPTYCONTEXT` の値（読むだけ。判定には使わない）
    pub disabled: Option<i32>,
    pub empty: Option<i32>,
}

/// 前回出した入力スコープの診断の行（同じ行を打鍵のたびに出さないため。最初の 1 回は必ず出る）
static LAST_SCOPE_LINE: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// 入力スコープの読み取りの結果を、前回と違うときだけ 1 行出す（空や失敗のときも出す）。secure の判定の調査用
pub fn input_scope(diag: &ScopeDiag<'_>) {
    let line = format!(
        "{PREFIX}input scope path=key range={} app={} tip={} disabled={} empty={}\n",
        diag.range,
        diag.app,
        diag.tip,
        number(diag.disabled),
        number(diag.empty),
    );
    let Ok(mut last) = LAST_SCOPE_LINE.lock() else {
        return;
    };
    if last.as_deref() == Some(line.as_str()) {
        return;
    }
    write(&line);
    *last = Some(line);
}
