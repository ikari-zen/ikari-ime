//! `postal.tsv`（住所の読み・表記・郵便番号の表）の形式。ADR 0032 §3。
//!
//! **契約**: 書く側（`ikari-dict-builder` の `town`）と読む側（`ikari-core::postal::PostalTable`）は、
//! この module の定数・関数だけでファイルの形を決める。ここを変えるときは両方を同時に直す。
//!
//! # ファイルの形
//! - 文字コードは UTF-8（BOM なし）。改行は LF（読む側は行末の CR を取り除く）。
//! - `#` で始まる行と空行はヘッダー・コメント。読む側は読み飛ばす。書く側は先頭に
//!   [`POSTAL_TSV_HEADER`] の行を置く（形式を変えるときは `v1` を上げる）。
//! - データ行は TAB 区切りの 4 列ちょうど: `読み<TAB>住所の表記<TAB>郵便番号<TAB>形`。
//!   （ADR 0032 §3 は 3 列だったが、郵便番号 → 住所に使う「いちばん省略のない形」を行から
//!   見分けるために 4 列目を足した。都道府県名で始まるかでは、郡だけを省いた形と区別できない。）
//!   - 読み: `normalize_key` 済み（ひらがな）で空でない。`chain_lex.csv` の連なりと同じ関数で作る。
//!     `MAX_READING_CHARS` の制限は受けない（連なりは長い）。
//!   - 住所の表記: 空でない。
//!   - 郵便番号: 半角 7 桁（先頭の 0 を保つ）。複数は `,` 区切りで、昇順・重複なし。1 つ以上。
//!   - 形: [`FORM_FULL`]（`f`。いちばん省略のない形 = 都道府県つき・郡つき。郵便番号 → 住所の
//!     逆引きに使う）か [`FORM_SHORT`]（`s`。都道府県や郡を省いた形。読み → 郵便番号にだけ使う）。
//! - 1 つの町（町字）につき、`f` の行を **1 行**、続けて `s` の行を 0 行以上書く。
//!   町の並びは (lg_code, machiaza_id) の昇順。
//! - 読む側は並びを信用しない（読みでの検索は読み込み時に索引を作る）。ただし**郵便番号 → 住所の
//!   順は、`f` の行のファイル内の順**になる（ADR 0032 §5 の町字 ID の順は、書く側がこの並びで保証する）。
//! - 同じ (読み, 表記) の行は書かない（読む側は重複をそのまま持つ）。
//!   同じ読みで表記が違う行はありうる（都道府県を省いた形の衝突）。

use std::fmt;

use crate::reading::normalize_key;

/// `postal.tsv` のファイル名（辞書と同じ `dict/` の下。`emoji.tsv` と同じ置き場所）
pub const POSTAL_TSV_FILE_NAME: &str = "postal.tsv";
/// 先頭に書くヘッダー行（`#` つき。改行は含まない）
pub const POSTAL_TSV_HEADER: &str = "# ikari-ime postal table v1";
/// 列の説明の行（ヘッダーの 2 行目。読む側は見ない）
pub const POSTAL_TSV_COLUMNS_COMMENT: &str = "# 列: 読み<TAB>住所の表記<TAB>郵便番号（7桁。複数は,区切り）<TAB>形（f=省略のない形, s=省いた形）";
pub const POSTAL_COMMENT_PREFIX: char = '#';
pub const POSTAL_COLUMN_SEPARATOR: char = '\t';
pub const POSTAL_CODE_SEPARATOR: char = ',';
pub const POSTAL_COLUMN_COUNT: usize = 4;
/// 郵便番号の桁数
pub const POSTAL_CODE_DIGITS: usize = 7;
/// 郵便番号の最大値（9999999）
pub const POSTAL_CODE_MAX: u32 = 9_999_999;
/// 形の列: いちばん省略のない形
pub const FORM_FULL: &str = "f";
/// 形の列: 都道府県や郡を省いた形
pub const FORM_SHORT: &str = "s";

/// 1 行の読み取りの失敗
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostalLineError {
    /// TAB 区切りの 4 列ではない
    ColumnCount,
    /// 読みが空、正規化済みでない、または改行を含む
    BadReading,
    /// 表記が空、または改行を含む
    BadSurface,
    /// 郵便番号が空、7 桁でない、昇順・重複なしでない
    BadCodes,
    /// 形が `f` / `s` でない
    BadForm,
}

impl fmt::Display for PostalLineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            PostalLineError::ColumnCount => "expected 4 tab-separated columns",
            PostalLineError::BadReading => "bad reading",
            PostalLineError::BadSurface => "bad surface",
            PostalLineError::BadCodes => "bad postal codes",
            PostalLineError::BadForm => "bad form (expected f or s)",
        })
    }
}

impl std::error::Error for PostalLineError {}

/// 読んだデータ行（郵便番号は `parse_line` に渡した作業用の Vec に入る）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PostalLine<'a> {
    pub reading: &'a str,
    pub surface: &'a str,
    /// `f` の行か
    pub is_full: bool,
}

/// ヘッダー・コメント・空行か（読み飛ばす行）
pub fn is_skipped_line(line: &str) -> bool {
    line.is_empty() || line.starts_with(POSTAL_COMMENT_PREFIX)
}

/// 郵便番号（半角 7 桁）を数にする。桁数・文字が違えば None。
pub fn parse_code(s: &str) -> Option<u32> {
    if s.len() != POSTAL_CODE_DIGITS || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// 数を 7 桁の半角にする（先頭を 0 で埋める）。`POSTAL_CODE_MAX` を超える値は呼ばない。
pub fn format_code(code: u32) -> String {
    format!("{code:0width$}", width = POSTAL_CODE_DIGITS)
}

/// データ行を読む。`codes` は呼び出し側の作業用の Vec（先に空にして、読んだ郵便番号を入れる。
/// 行ごとの確保を避けるため）。ヘッダー・空行はここに渡さない（[`is_skipped_line`] で先に除く）。
/// 行末の CR は呼び出し側が取り除く。
pub fn parse_line<'a>(
    line: &'a str,
    codes: &mut Vec<u32>,
) -> Result<PostalLine<'a>, PostalLineError> {
    codes.clear();
    let mut cols = line.split(POSTAL_COLUMN_SEPARATOR);
    let (Some(reading), Some(surface), Some(code_col), Some(form), None) = (
        cols.next(),
        cols.next(),
        cols.next(),
        cols.next(),
        cols.next(),
    ) else {
        return Err(PostalLineError::ColumnCount);
    };
    check_text(reading, surface)?;
    for part in code_col.split(POSTAL_CODE_SEPARATOR) {
        let code = parse_code(part).ok_or(PostalLineError::BadCodes)?;
        if codes.last().is_some_and(|&last| last >= code) {
            return Err(PostalLineError::BadCodes);
        }
        codes.push(code);
    }
    let is_full = match form {
        FORM_FULL => true,
        FORM_SHORT => false,
        _ => return Err(PostalLineError::BadForm),
    };
    Ok(PostalLine {
        reading,
        surface,
        is_full,
    })
}

/// データ行を 1 行（改行なし）に書く。[`parse_line`] が受ける形だけを返す
/// （書く側が守る仕様と読む側が守る仕様を一致させるため、同じ検査をする）。
pub fn format_line(
    reading: &str,
    surface: &str,
    codes: &[u32],
    is_full: bool,
) -> Result<String, PostalLineError> {
    check_text(reading, surface)?;
    let valid = !codes.is_empty()
        && codes.iter().all(|&c| c <= POSTAL_CODE_MAX)
        && codes.windows(2).all(|w| w[0] < w[1]);
    if !valid {
        return Err(PostalLineError::BadCodes);
    }
    let code_col = codes
        .iter()
        .map(|&c| format_code(c))
        .collect::<Vec<_>>()
        .join(&POSTAL_CODE_SEPARATOR.to_string());
    let form = if is_full { FORM_FULL } else { FORM_SHORT };
    let sep = POSTAL_COLUMN_SEPARATOR;
    Ok(format!("{reading}{sep}{surface}{sep}{code_col}{sep}{form}"))
}

fn check_text(reading: &str, surface: &str) -> Result<(), PostalLineError> {
    let has_break = |s: &str| s.contains(['\t', '\n', '\r']);
    if reading.is_empty() || has_break(reading) || normalize_key(reading) != reading {
        return Err(PostalLineError::BadReading);
    }
    if surface.is_empty() || has_break(surface) {
        return Err(PostalLineError::BadSurface);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = include_str!("../tests/fixtures/postal.sample.tsv");

    #[test]
    fn code_round_trip_keeps_leading_zeros() {
        assert_eq!(parse_code("0600042"), Some(600_042));
        assert_eq!(format_code(600_042), "0600042");
        assert_eq!(parse_code("060004"), None);
        assert_eq!(parse_code("06000420"), None);
        assert_eq!(parse_code("060-0042"), None);
        assert_eq!(parse_code("０６０００４２"), None);
    }

    #[test]
    fn every_sample_data_line_parses_and_writes_back_identically() {
        let mut codes = Vec::new();
        let mut rows = 0;
        for line in SAMPLE.lines().filter(|l| !is_skipped_line(l)) {
            let row = parse_line(line, &mut codes).expect(line);
            let written = format_line(row.reading, row.surface, &codes, row.is_full).unwrap();
            assert_eq!(written, line);
            rows += 1;
        }
        assert!(rows > 0);
        assert_eq!(SAMPLE.lines().next(), Some(POSTAL_TSV_HEADER));
    }

    #[test]
    fn rejects_malformed_lines() {
        let mut c = Vec::new();
        fn ok<'a>(s: &'a str, c: &mut Vec<u32>) -> Result<PostalLine<'a>, PostalLineError> {
            parse_line(s, c)
        }
        assert_eq!(
            ok("あ\tア\t1000001", &mut c),
            Err(PostalLineError::ColumnCount)
        );
        assert_eq!(
            ok("あ\tア\t1000001\tf\tx", &mut c),
            Err(PostalLineError::ColumnCount)
        );
        assert_eq!(
            ok("\tア\t1000001\tf", &mut c),
            Err(PostalLineError::BadReading)
        );
        // カタカナの読みは正規化済みでない
        assert_eq!(
            ok("ア\tア\t1000001\tf", &mut c),
            Err(PostalLineError::BadReading)
        );
        assert_eq!(
            ok("あ\t\t1000001\tf", &mut c),
            Err(PostalLineError::BadSurface)
        );
        assert_eq!(
            ok("あ\tア\t1000001\tx", &mut c),
            Err(PostalLineError::BadForm)
        );
        for bad in [
            "",
            "100000",
            "1000002,1000001",
            "1000001,1000001",
            "1000001,",
        ] {
            assert_eq!(
                ok(&format!("あ\tア\t{bad}\tf"), &mut c),
                Err(PostalLineError::BadCodes),
                "{bad}"
            );
        }
        assert_eq!(
            format_line("あ", "ア", &[], true),
            Err(PostalLineError::BadCodes)
        );
        assert_eq!(
            format_line("あ", "ア", &[POSTAL_CODE_MAX + 1], true),
            Err(PostalLineError::BadCodes)
        );
    }
}
