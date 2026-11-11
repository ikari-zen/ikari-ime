//! テキストファイルの形式の共通部分（ADR 0040 §2.9、§3.4）。
//!
//! - 書くときは UTF-8、改行は LF（BOM なし）。すでに CRLF のファイルを書き換えるときは CRLF のまま書く。
//!   新しく作るファイルは LF。
//! - 読むときは LF でも CRLF でも、BOM つきでも読める。BOM は [`strip_bom`] / [`read_to_string`] で外す。
//!   行に分けるときは `str::lines`（CRLF を扱える）を使う。
//!
//! 依存は標準ライブラリだけ。`core`、`shell-core`、`engine`、`dict-builder`、`xtask` のどれからも使える。

use std::io;
use std::path::Path;

/// UTF-8 の BOM（U+FEFF）。
pub const BOM: char = '\u{feff}';
const LF: &str = "\n";
const CRLF: &str = "\r\n";

/// 先頭の BOM を 1 つだけ外す。なければそのまま返す。
pub fn strip_bom(text: &str) -> &str {
    text.strip_prefix(BOM).unwrap_or(text)
}

/// ファイルを UTF-8 の文字列として読み、先頭の BOM を外して返す。
/// UTF-8 でなければ `read_to_string` と同じく `InvalidData`。
pub fn read_to_string(path: impl AsRef<Path>) -> io::Result<String> {
    let mut text = std::fs::read_to_string(path)?;
    if text.starts_with(BOM) {
        text.drain(..BOM.len_utf8());
    }
    Ok(text)
}

/// 文字列に CRLF（`\r\n`）が 1 つでもあるか。書き戻しの改行を元のファイルに合わせる判定に使う。
pub fn has_crlf(text: &str) -> bool {
    text.contains(CRLF)
}

/// 書き戻すときの改行。元の文字列が CRLF を含めば CRLF、そうでなければ（新規を含む）LF。
pub fn newline_for(original: &str) -> &'static str {
    if has_crlf(original) { CRLF } else { LF }
}

/// `text` の改行を、`original` の改行に合わせて返す（CRLF を重ねない）。
pub fn match_newlines(text: &str, original: &str) -> String {
    let normalized = text.replace(CRLF, LF);
    if has_crlf(original) {
        normalized.replace(LF, CRLF)
    } else {
        normalized
    }
}

/// テストで「LF」「CRLF」「BOM つき」の入力を作る補助（ADR 0040 §2.9）。
pub mod testing {
    use super::BOM;

    /// 入力の形。
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Variant {
        Lf,
        Crlf,
        /// BOM つき（改行は LF）
        BomLf,
        /// BOM つき（改行は CRLF。メモ帳の「UTF-8 (BOM 付き)」で保存した形）
        BomCrlf,
    }

    /// 読むテストで回す 3 通り（LF、CRLF、BOM つき）。
    pub const READ_VARIANTS: [Variant; 3] = [Variant::Lf, Variant::Crlf, Variant::BomLf];

    /// 4 通りすべて。
    pub const ALL_VARIANTS: [Variant; 4] =
        [Variant::Lf, Variant::Crlf, Variant::BomLf, Variant::BomCrlf];

    /// LF で書いた `text` を、指定の形にする。
    pub fn variant(text: &str, variant: Variant) -> String {
        let lf = text.replace("\r\n", "\n");
        let body = match variant {
            Variant::Lf | Variant::BomLf => lf,
            Variant::Crlf | Variant::BomCrlf => lf.replace('\n', "\r\n"),
        };
        match variant {
            Variant::BomLf | Variant::BomCrlf => format!("{BOM}{body}"),
            Variant::Lf | Variant::Crlf => body,
        }
    }

    /// LF の `text` から、読むテスト用の 3 通りを作る。
    pub fn read_variants(text: &str) -> Vec<(Variant, String)> {
        READ_VARIANTS
            .iter()
            .map(|&v| (v, variant(text, v)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;

    #[test]
    fn strip_bom_removes_one_leading_bom_only() {
        assert_eq!(strip_bom("\u{feff}a"), "a");
        assert_eq!(strip_bom("a\u{feff}"), "a\u{feff}");
        assert_eq!(strip_bom("\u{feff}\u{feff}a"), "\u{feff}a");
        assert_eq!(strip_bom(""), "");
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("ikari-textfile-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn read_to_string_strips_bom_for_all_variants() {
        let dir = scratch("read");
        for (v, text) in read_variants("あ\tい\nう\tえ\n") {
            let path = dir.join(format!("{v:?}.tsv"));
            std::fs::write(&path, &text).unwrap();
            let read = read_to_string(&path).unwrap();
            assert!(!read.starts_with(BOM), "{v:?}");
            assert_eq!(
                read.lines().collect::<Vec<_>>(),
                ["あ\tい", "う\tえ"],
                "{v:?}"
            );
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn read_to_string_reports_missing_and_invalid_utf8() {
        let dir = scratch("err");
        let missing = read_to_string(dir.join("none")).unwrap_err();
        assert_eq!(missing.kind(), io::ErrorKind::NotFound);
        std::fs::write(dir.join("bad"), [0xff, 0xfe, 0x41]).unwrap();
        let bad = read_to_string(dir.join("bad")).unwrap_err();
        assert_eq!(bad.kind(), io::ErrorKind::InvalidData);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn newline_detection() {
        assert!(!has_crlf(""));
        assert!(!has_crlf("a\nb\n"));
        assert!(!has_crlf("a\rb"));
        assert!(has_crlf("a\r\nb"));
        assert!(has_crlf("a\nb\r\n"));
        assert_eq!(newline_for("a\r\n"), "\r\n");
        assert_eq!(newline_for("a\n"), "\n");
        assert_eq!(newline_for(""), "\n");
    }

    #[test]
    fn match_newlines_follows_original_without_doubling() {
        assert_eq!(match_newlines("a\nb\n", "x\r\n"), "a\r\nb\r\n");
        assert_eq!(match_newlines("a\r\nb\n", "x\r\n"), "a\r\nb\r\n");
        assert_eq!(match_newlines("a\r\nb\n", "x\n"), "a\nb\n");
        assert_eq!(match_newlines("a\nb\n", ""), "a\nb\n");
    }

    #[test]
    fn variants_build_expected_shapes() {
        assert_eq!(variant("a\nb", Variant::Lf), "a\nb");
        assert_eq!(variant("a\nb", Variant::Crlf), "a\r\nb");
        assert_eq!(variant("a\nb", Variant::BomLf), "\u{feff}a\nb");
        assert_eq!(variant("a\r\nb", Variant::BomCrlf), "\u{feff}a\r\nb");
        assert_eq!(read_variants("x").len(), 3);
        assert_eq!(ALL_VARIANTS.len(), 4);
    }
}
