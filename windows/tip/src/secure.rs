//! secure な入力欄（パスワード欄、PIN 欄）の判定の表（ADR 0040 §6 S8。NFR-18）。
//!
//! 受け取るのは、COM の層が TSF から読んだ**値**だけ（OS の呼び出しは含めない）。結果は `RawKey::secure` と
//! `ShellEvent::Activate` の `secure` に入る。secure のときに何をするか（素通し、合成しない、周辺テキストを
//! 読まない、学習と診断に回さない）の判断は `shell-core` にある（macOS の `IsSecureEventInputEnabled()` と同じ口）。
//!
//! # TSF で読める手がかり
//! - 入力スコープ（`GUID_PROP_INPUTSCOPE`）: アプリが欄に付ける属性。`IS_PASSWORD` など。**これだけが TIP から読める**。
//! - `TS_SS_PASSWORD` という静的フラグは存在しない（`TS_STATUS` / `TF_STATUS` の `dwStaticFlags` にあるのは
//!   `DISJOINTSEL`、`REGIONS`、`TRANSITORY`、`NOHIDDENTEXT`、`TKB*`、`UWPCONTROL` だけ）。`TF_SS_TRANSITORY` は
//!   IME 自身の一時的な文脈の印で、秘密とは関係がないので使わない。
//! - `ITextStoreACP::GetAttrs` の `TS_ATTR_FIND_*` はアプリ側（テキストストア）の API で、TIP からは呼べない。
//!
//! # 読めなかったとき
//! - 入力スコープが付いていない欄（ふつうの欄の大半）、プロパティが空、読み取りの失敗は**通常として扱う**。
//!   不明を secure にすると、入力スコープを返さないアプリで変換が全部止まる。一方、パスワード欄が入力スコープを
//!   付けないとき（独自描画の欄など）は守れない。これは macOS の仕様（NFR-18 の注）と同じ限界で、実機で確かめる。
//! - 1 つでも secure な入力スコープが読めたら secure（ほかのスコープと並んでいても）。

/// `IS_PASSWORD`（`InputScope.h`）
pub const IS_PASSWORD: i32 = 31;
/// `IS_NUMERIC_PIN`
pub const IS_NUMERIC_PIN: i32 = 64;
/// `IS_ALPHANUMERIC_PIN`
pub const IS_ALPHANUMERIC_PIN: i32 = 65;
/// `IS_ALPHANUMERIC_PIN_SET`
pub const IS_ALPHANUMERIC_PIN_SET: i32 = 66;

/// `IS_PRIVATE`。Chromium 系（Edge、Chrome）は、`<input type=password>` の入力スコープに `IS_PASSWORD` ではなく
/// これを返す（`ui/base/ime/win/tsf_input_scope.cc`）。2026-11-14 ユーザー決定で secure に含める（#369）。
pub const IS_PRIVATE: i32 = 61;

/// secure な欄を表す入力スコープ
const SECURE_SCOPES: [i32; 5] = [
    IS_PASSWORD,
    IS_NUMERIC_PIN,
    IS_ALPHANUMERIC_PIN,
    IS_ALPHANUMERIC_PIN_SET,
    IS_PRIVATE,
];

/// 欄の入力スコープの読み取りの結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeReading<'a> {
    /// 読めた（空のこともある）
    Scopes(&'a [i32]),
    /// プロパティが空（アプリが入力スコープを付けていない）
    NotSet,
    /// 読めなかった（TSF がエラーを返した、型が違う、など）
    Unreadable,
}

/// この欄は secure か。
pub fn is_secure(reading: ScopeReading<'_>) -> bool {
    match reading {
        ScopeReading::Scopes(scopes) => scopes.iter().any(|scope| SECURE_SCOPES.contains(scope)),
        ScopeReading::NotSet | ScopeReading::Unreadable => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// IS_DEFAULT、IS_URL、IS_NUMBER、IS_EMAIL_SMTPEMAILADDRESS など、秘密でないスコープ
    const IS_DEFAULT: i32 = 0;
    const IS_URL: i32 = 1;
    const IS_NUMBER: i32 = 29;
    const IS_EMAIL_SMTPEMAILADDRESS: i32 = 5;

    #[test]
    fn password_and_pin_are_secure() {
        for scope in SECURE_SCOPES {
            assert!(is_secure(ScopeReading::Scopes(&[scope])), "{scope}");
        }
    }

    #[test]
    fn private_scope_from_chromium_is_secure() {
        assert!(is_secure(ScopeReading::Scopes(&[IS_PRIVATE])));
        assert!(is_secure(ScopeReading::Scopes(&[IS_DEFAULT, IS_PRIVATE])));
    }

    #[test]
    fn password_among_other_scopes_is_secure() {
        assert!(is_secure(ScopeReading::Scopes(&[IS_DEFAULT, IS_PASSWORD])));
        assert!(is_secure(ScopeReading::Scopes(&[
            IS_URL,
            IS_NUMERIC_PIN,
            IS_NUMBER
        ])));
    }

    #[test]
    fn ordinary_fields_are_not_secure() {
        assert!(!is_secure(ScopeReading::Scopes(&[IS_DEFAULT])));
        assert!(!is_secure(ScopeReading::Scopes(&[IS_URL])));
        assert!(!is_secure(ScopeReading::Scopes(&[IS_NUMBER])));
        assert!(!is_secure(ScopeReading::Scopes(&[
            IS_EMAIL_SMTPEMAILADDRESS
        ])));
        // 数字だけの欄（PIN ではない）は通常
        assert!(!is_secure(ScopeReading::Scopes(&[IS_NUMBER, IS_DEFAULT])));
    }

    #[test]
    fn empty_list_is_not_secure() {
        assert!(!is_secure(ScopeReading::Scopes(&[])));
    }

    #[test]
    fn unknown_is_treated_as_ordinary() {
        assert!(!is_secure(ScopeReading::NotSet));
        assert!(!is_secure(ScopeReading::Unreadable));
    }
}
