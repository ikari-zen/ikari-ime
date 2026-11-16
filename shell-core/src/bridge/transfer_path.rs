//! 書き出しと取り込みのファイルのパスの検査（Windows の設定の口。ADR 0068 §4、security-design.md §8.3 の B-2）。
//! **OS に依存しない**（Mac でテストする。パスは Windows の形の文字列として見る）。
//!
//! Windows では、学習の全文を設定の口に流さない。設定画面は、利用者が選んだファイルのパスだけを送り、engine-host が
//! そのファイルを読み書きする。パスは**信用しない入力**として、ここで形だけを確かめる（ファイルシステムには触れない）。
//! ふつうのファイルか、大きさ、既にあるかは、開いたあとに engine-host が見る（`windows/engine-host/src/transfer.rs`）。
//!
//! # 通すもの
//! - ドライブ文字で始まる絶対パス（`C:\…`）。末尾はファイルの名前。
//!
//! # 拒むもの
//! - 相対パス、`C:foo`（ドライブの今のフォルダからの相対）、`\foo`（今のドライブの根からの相対）
//! - `\\` で始まるもの（ネットワークの場所 `\\server\share`、デバイスの名前空間 `\\.\`、`\\?\`）
//! - `/` を含むもの（ファイルを選ぶ窓は返さない。区切りの解釈を 1 つにする）
//! - 2 文字目より後ろの `:`（代替データストリーム `a.tsv:x`）
//! - 空の名前（`\\` の重なり、末尾の `\`）、`.` と `..`、末尾が `.` か空白の名前（Windows が黙って削り、別の名前に化ける）
//! - 制御文字と、名前に使えない文字（`<>"|?*`）
//! - 予約されたデバイスの名前（`CON`、`NUL`、`COM1` など。拡張子つきでも）
//! - 長すぎるもの（UTF-16 で `MAX_PATH_UTF16` を超える）
//!
//! macOS は形が違う（エンジンが Sandbox の中にいて、パスを受けない。#274 の設計のまま）。
//!
//! 判断の置き場所: Rust。設定画面（C#）と engine-host のどちらも、この関数の結果だけを使う。

/// パスの長さの上限（UTF-16 の単位。Windows の長いパスの上限）
pub const MAX_PATH_UTF16: usize = 32_767;

const SEPARATOR: char = '\\';
const ALT_SEPARATOR: char = '/';
const DRIVE_SEPARATOR: char = ':';
/// 名前に使えない文字（制御文字のほか）
const FORBIDDEN_CHARS: [char; 6] = ['<', '>', '"', '|', '?', '*'];
/// 予約されたデバイスの名前（拡張子を除いた部分を、ASCII の大文字・小文字を区別せずに比べる）
const RESERVED_STEMS: [&str; 6] = ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"];
/// 後ろに数字が 1 つ付くと予約になる名前（`COM1`、`LPT9`、`COM¹` など）
const RESERVED_NUMBERED_STEMS: [&str; 2] = ["COM", "LPT"];
/// 予約の数字として扱われる上付きの数字（¹ ² ³）
const SUPERSCRIPT_DIGITS: [char; 3] = ['\u{b9}', '\u{b2}', '\u{b3}'];

/// 検査を通ったパス（Windows の形のまま）。engine-host はこれだけを開く。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferPath(String);

impl TransferPath {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// パスを拒んだ理由（ログと返事には、この固定の語だけを出す。パスそのものは出さない）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferPathError {
    /// 空か、長すぎる
    Length,
    /// ドライブ文字で始まる絶対パスでない
    NotAbsolute,
    /// ネットワークの場所か、デバイスの名前空間
    NetworkOrDevice,
    /// 名前の形が使えない（空、`.`・`..`、末尾の `.` や空白、使えない文字、`:`、`/`）
    BadName,
    /// 予約されたデバイスの名前
    ReservedName,
}

impl TransferPathError {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Length => "length",
            Self::NotAbsolute => "not-absolute",
            Self::NetworkOrDevice => "network-or-device",
            Self::BadName => "bad-name",
            Self::ReservedName => "reserved-name",
        }
    }
}

/// `path`（設定画面が送ってきた文字列）を検査する。
pub fn check_transfer_path(path: &str) -> Result<TransferPath, TransferPathError> {
    if path.is_empty() || path.encode_utf16().count() > MAX_PATH_UTF16 {
        return Err(TransferPathError::Length);
    }
    if path.starts_with(r"\\") {
        return Err(TransferPathError::NetworkOrDevice);
    }
    if path.contains(ALT_SEPARATOR) {
        return Err(TransferPathError::BadName);
    }
    let mut chars = path.chars();
    let (Some(letter), Some(DRIVE_SEPARATOR), Some(SEPARATOR)) =
        (chars.next(), chars.next(), chars.next())
    else {
        return Err(TransferPathError::NotAbsolute);
    };
    if !letter.is_ascii_alphabetic() {
        return Err(TransferPathError::NotAbsolute);
    }
    // ドライブ（`C:\`）の後ろ。少なくともファイルの名前が 1 つ要る
    let below = &path[3..];
    if below.is_empty() {
        return Err(TransferPathError::BadName);
    }
    for name in below.split(SEPARATOR) {
        check_name(name)?;
    }
    Ok(TransferPath(path.to_owned()))
}

fn check_name(name: &str) -> Result<(), TransferPathError> {
    if name.is_empty() || name == "." || name == ".." {
        return Err(TransferPathError::BadName);
    }
    if name.ends_with('.') || name.ends_with(' ') {
        return Err(TransferPathError::BadName);
    }
    if name
        .chars()
        .any(|c| c.is_control() || c == DRIVE_SEPARATOR || FORBIDDEN_CHARS.contains(&c))
    {
        return Err(TransferPathError::BadName);
    }
    if is_reserved(name) {
        return Err(TransferPathError::ReservedName);
    }
    Ok(())
}

/// 予約されたデバイスの名前か（`NUL`、`nul.tsv`、`COM1.txt`、`CON .tsv` など）
fn is_reserved(name: &str) -> bool {
    // 拡張子（最初の `.` から後ろ）を除き、末尾の空白も除いて比べる（Windows は `CON .tsv` も CON と読む）
    let stem = name.split('.').next().unwrap_or(name).trim_end_matches(' ');
    if RESERVED_STEMS.iter().any(|r| stem.eq_ignore_ascii_case(r)) {
        return true;
    }
    RESERVED_NUMBERED_STEMS.iter().any(|prefix| {
        let mut chars = stem.chars();
        let head: String = chars.by_ref().take(prefix.len()).collect();
        let rest: Vec<char> = chars.collect();
        head.eq_ignore_ascii_case(prefix)
            && rest.len() == 1
            && (rest[0].is_ascii_digit() || SUPERSCRIPT_DIGITS.contains(&rest[0]))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use TransferPathError::*;

    fn check(path: &str) -> Result<(), TransferPathError> {
        check_transfer_path(path).map(|_| ())
    }

    #[test]
    fn absolute_drive_paths_pass_and_keep_their_text() {
        for path in [
            r"C:\Users\example\Documents\ikari-export-20261116.tsv",
            r"d:\backup\学習.tsv",
            r"C:\a.tsv",
            r"C:\Users\example\console.tsv",
            r"C:\Users\example\COM10.tsv",
            r"C:\Users\example\.hidden",
        ] {
            assert_eq!(
                check_transfer_path(path).map(|p| p.as_str().to_owned()),
                Ok(path.to_owned()),
                "{path}"
            );
        }
    }

    #[test]
    fn relative_and_drive_relative_paths_are_refused() {
        let cases = [
            ("a.tsv", NotAbsolute),
            (r"Documents\a.tsv", NotAbsolute),
            ("C:a.tsv", NotAbsolute),
            (r"\Users\a.tsv", NotAbsolute),
            (r"1:\a.tsv", NotAbsolute),
            (r"C:\", BadName),
            ("", Length),
        ];
        for (path, expected) in cases {
            assert_eq!(check(path), Err(expected), "{path}");
        }
    }

    #[test]
    fn network_and_device_paths_are_refused() {
        for path in [
            r"\\server\share\a.tsv",
            r"\\?\C:\a.tsv",
            r"\\?\UNC\server\share\a.tsv",
            r"\\.\pipe\x",
            r"\\.\C:\a.tsv",
        ] {
            assert_eq!(check(path), Err(NetworkOrDevice), "{path}");
        }
    }

    #[test]
    fn bad_names_are_refused() {
        for path in [
            r"C:\Users\example\",
            r"C:\Users\\a.tsv",
            r"C:\Users\..\a.tsv",
            r"C:\Users\.\a.tsv",
            r"C:\Users\a.tsv.",
            r"C:\Users\a.tsv ",
            r"C:\Users\a.tsv:stream",
            "C:\\Users\\a\u{0}.tsv",
            "C:\\Users\\a\t.tsv",
            r"C:\Users\a?.tsv",
            r"C:\Users\a*.tsv",
            r"C:\Users\a|b.tsv",
            r"C:\Users/a.tsv",
        ] {
            assert_eq!(check(path), Err(BadName), "{path:?}");
        }
    }

    #[test]
    fn reserved_device_names_are_refused_with_or_without_an_extension() {
        for name in [
            "NUL",
            "nul.tsv",
            "CON",
            "con .tsv",
            "aux.tar.gz",
            "PRN",
            "COM1",
            "com9.tsv",
            "LPT0.txt",
            "COM\u{b9}.tsv",
            "CONIN$",
            "conout$.tsv",
        ] {
            let path = format!(r"C:\Users\example\{name}");
            assert_eq!(check(&path), Err(ReservedName), "{name}");
        }
        // フォルダの名前でも拒む
        assert_eq!(check(r"C:\NUL\a.tsv"), Err(ReservedName));
    }

    #[test]
    fn too_long_paths_are_refused() {
        let long = format!(r"C:\{}", "a".repeat(MAX_PATH_UTF16));
        assert_eq!(check(&long), Err(Length));
        let fits = format!(r"C:\{}", "a".repeat(MAX_PATH_UTF16 - 3));
        assert_eq!(check(&fits), Ok(()));
    }

    #[test]
    fn error_texts_are_distinct() {
        let all = [Length, NotAbsolute, NetworkOrDevice, BadName, ReservedName];
        let texts: std::collections::HashSet<_> = all.iter().map(|e| e.as_str()).collect();
        assert_eq!(texts.len(), all.len());
    }
}
