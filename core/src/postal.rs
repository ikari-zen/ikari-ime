//! 住所 ↔ 郵便番号の表（`postal.tsv`）。ADR 0032 §3。
//!
//! 契約コミットの時点で、読み込みと引きは最小の実装まで入っている（レーン C は形を変えずに使う）。
//! ファイルの形は `ikari_dict_format::postal` が決める（書く側の `town` と共有）。
//!
//! # 性能の設計（未計測。レーン C が測って PR に書く）
//! 規模の見込み: 郵便番号は約 11 万種、住所の連なり（`f` と `s` の行）は約 27 万行。
//! - 行ごとに `String` や `Vec` を持たない（27 万 × 数回の確保を避ける）。読みと表記は 1 本の
//!   `String`（arena）に連結し、行は (オフセット, 長さ) だけを持つ。郵便番号は 1 本の `Vec<u32>` に
//!   並べ、行は範囲だけを持つ。見込みは arena 20〜30MB 弱 + 行 8MB 弱。
//! - 読み → 行は、読み込み時に行番号を読みでソートした索引（`by_reading_order`）を作り、二分探索する。
//!   ハッシュ表は作らない（キーの `String` を 27 万個持つため）。ソートは 27 万件で数十 ms の見込み。
//! - 郵便番号 → 住所は、`f` の行だけの (郵便番号, 行番号) の配列を郵便番号で安定ソートして二分探索する。
//!   同じ郵便番号の中の順は、ファイルでの順（= 町字 ID の順。ADR 0032 §5）。
//! - 読み込みは `read_to_string` で 1 回。mmap や事前に作った索引は、測って遅いと分かってから考える。
//! - 目安: 辞書の読み込みと比べて目立たないこと。超えたら、ファイルを読みの順に書いてソートを
//!   省く、または固定形式にして mmap する（形式の版を上げる）。

use std::fmt;
use std::io;
use std::path::Path;

use ikari_dict_format::postal::{PostalLineError, is_skipped_line, parse_line};
use ikari_dict_format::reading::fold_fullwidth_digits;

pub use ikari_dict_format::postal::POSTAL_TSV_FILE_NAME;

/// `postal.tsv` の読み込みの失敗
#[derive(Debug)]
pub enum PostalError {
    /// 行の形が違う（行番号は 1 始まり）
    BadLine {
        line: usize,
        reason: PostalLineError,
    },
    /// ファイルが読めない（なかった場合は `load` が空の表を返すのでここには来ない）
    Io(io::Error),
    /// 文字列の領域が 4GiB を超える（壊れたファイル）
    TooLarge,
}

impl fmt::Display for PostalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PostalError::BadLine { line, reason } => {
                write!(f, "postal.tsv: bad line {line}: {reason}")
            }
            PostalError::Io(e) => write!(f, "postal.tsv: {e}"),
            PostalError::TooLarge => write!(f, "postal.tsv: too large"),
        }
    }
}

impl std::error::Error for PostalError {}

/// 読みで引いた 1 行（読みが同じで表記が違う行は複数ありうる）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PostalMatch<'a> {
    pub surface: &'a str,
    /// 昇順・重複なしの郵便番号（7 桁の数。先頭の 0 は数では消える。`format_code` で 7 桁に戻す）
    pub codes: &'a [u32],
}

#[derive(Debug, Clone, Copy)]
struct Row {
    reading_start: u32,
    reading_len: u32,
    surface_len: u32,
    codes_start: u32,
    codes_len: u32,
}

/// `postal.tsv` を読んだ表。
#[derive(Debug, Clone, Default)]
pub struct PostalTable {
    /// 各行の 読み + 表記 を順に連結したもの（表記は読みの直後）
    arena: String,
    rows: Vec<Row>,
    codes: Vec<u32>,
    /// 行番号を読みの昇順（同じ読みはファイル順）に並べたもの
    by_reading_order: Vec<u32>,
    /// `f` の行の (郵便番号, 行番号)。郵便番号の昇順（同じ郵便番号はファイル順）
    by_code: Vec<(u32, u32)>,
}

impl PostalTable {
    /// 空の表（`postal.tsv` がないとき。Rewriter は何も出さない）
    pub fn empty() -> PostalTable {
        PostalTable::default()
    }

    /// ファイルから読む。ファイルがなければ空の表（エラーにしない。ADR 0032 §3）。
    /// 行の形が違えばエラー（呼び出し側は警告を出して空の表で続ける）。
    pub fn load(path: &Path) -> Result<PostalTable, PostalError> {
        match std::fs::read_to_string(path) {
            Ok(text) => PostalTable::from_tsv_str(&text),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(PostalTable::empty()),
            Err(e) => Err(PostalError::Io(e)),
        }
    }

    /// 本文から作る（テスト用にも使う）。ヘッダー・空行は読み飛ばす。仕様は
    /// `ikari_dict_format::postal`。同じ (読み, 表記) の行が重複しても、合わせずにそのまま持つ。
    pub fn from_tsv_str(text: &str) -> Result<PostalTable, PostalError> {
        let mut table = PostalTable::default();
        let mut codes = Vec::new();
        let mut full_rows: Vec<(u32, u32)> = Vec::new();
        for (i, raw) in text.lines().enumerate() {
            let line = raw.strip_suffix('\r').unwrap_or(raw);
            if is_skipped_line(line) {
                continue;
            }
            let row = parse_line(line, &mut codes).map_err(|reason| PostalError::BadLine {
                line: i + 1,
                reason,
            })?;
            let index = u32::try_from(table.rows.len()).map_err(|_| PostalError::TooLarge)?;
            let reading_start =
                u32::try_from(table.arena.len()).map_err(|_| PostalError::TooLarge)?;
            let codes_start =
                u32::try_from(table.codes.len()).map_err(|_| PostalError::TooLarge)?;
            table.arena.push_str(row.reading);
            table.arena.push_str(row.surface);
            table.codes.extend_from_slice(&codes);
            table.rows.push(Row {
                reading_start,
                reading_len: row.reading.len() as u32,
                surface_len: row.surface.len() as u32,
                codes_start,
                codes_len: codes.len() as u32,
            });
            if row.is_full {
                full_rows.extend(codes.iter().map(|&c| (c, index)));
            }
        }
        u32::try_from(table.arena.len()).map_err(|_| PostalError::TooLarge)?;
        let mut order: Vec<u32> = (0..table.rows.len() as u32).collect();
        // 安定ソート: 同じ読みはファイル順のまま
        order.sort_by(|&a, &b| table.reading_of(a).cmp(table.reading_of(b)));
        table.by_reading_order = order;
        full_rows.sort_by_key(|&(code, _)| code);
        table.by_code = full_rows;
        Ok(table)
    }

    /// 読み（`normalize_key` 済み）に完全一致する行。なければ空。
    /// 表の読みの数字は半角（`きた1じょうにし`。ADR 0030 §3.1）なので、引く読みの全角の数字は半角にして引く。
    pub fn by_reading(&self, reading: &str) -> Vec<PostalMatch<'_>> {
        let reading = fold_fullwidth_digits(reading);
        let reading = reading.as_ref();
        let order = &self.by_reading_order;
        let start = order.partition_point(|&r| self.reading_of(r) < reading);
        order[start..]
            .iter()
            .take_while(|&&r| self.reading_of(r) == reading)
            .map(|&r| self.matched(r))
            .collect()
    }

    /// 郵便番号（7 桁の数）の住所の表記。`f` の行だけ（いちばん省略のない形）。
    /// 順はファイルの順（町字 ID の順）。なければ空。
    pub fn by_code(&self, code: u32) -> Vec<&str> {
        let start = self.by_code.partition_point(|&(c, _)| c < code);
        self.by_code[start..]
            .iter()
            .take_while(|&&(c, _)| c == code)
            .map(|&(_, r)| self.surface_of(r))
            .collect()
    }

    /// 行数
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    fn reading_of(&self, row: u32) -> &str {
        let r = &self.rows[row as usize];
        let start = r.reading_start as usize;
        &self.arena[start..start + r.reading_len as usize]
    }

    fn surface_of(&self, row: u32) -> &str {
        let r = &self.rows[row as usize];
        let start = (r.reading_start + r.reading_len) as usize;
        &self.arena[start..start + r.surface_len as usize]
    }

    fn matched(&self, row: u32) -> PostalMatch<'_> {
        let r = &self.rows[row as usize];
        let start = r.codes_start as usize;
        PostalMatch {
            surface: self.surface_of(row),
            codes: &self.codes[start..start + r.codes_len as usize],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = include_str!("../../dict-format/tests/fixtures/postal.sample.tsv");

    fn sample() -> PostalTable {
        PostalTable::from_tsv_str(SAMPLE).unwrap()
    }

    #[test]
    fn looks_up_by_reading() {
        let t = sample();
        assert_eq!(
            t.by_reading("ちよだくちよだ"),
            [PostalMatch {
                surface: "千代田区千代田",
                codes: &[1_000_001]
            }]
        );
        assert_eq!(
            t.by_reading("とうきょうとちよだくまるのうち")[0].codes,
            [1_000_005, 1_006_390]
        );
        assert!(t.by_reading("ちよだく").is_empty());
        assert!(t.by_reading("").is_empty());
    }

    #[test]
    fn digits_in_the_reading_match_in_both_widths() {
        let t = PostalTable::from_tsv_str(
            "# ikari-ime postal table v1\nさっぽろしちゅうおうくきた1じょうにし\t札幌市中央区北1条西\t0600001\ts\n",
        )
        .unwrap();
        for reading in [
            "さっぽろしちゅうおうくきた1じょうにし",
            "さっぽろしちゅうおうくきた１じょうにし",
        ] {
            assert_eq!(t.by_reading(reading)[0].codes, [600_001], "{reading}");
        }
    }

    #[test]
    fn reverse_lookup_uses_full_forms_in_file_order() {
        let t = sample();
        assert_eq!(t.by_code(1_000_001), ["東京都千代田区千代田"]);
        // 1 つの町に複数の郵便番号: どちらからも引ける
        assert_eq!(t.by_code(1_006_390), ["東京都千代田区丸の内"]);
        // 1 つの郵便番号に複数の町: ファイルの順
        assert_eq!(
            t.by_code(600_042),
            ["北海道札幌市中央区大通西", "北海道札幌市中央区大通東"]
        );
        // 郡を省いた形（s）は逆引きに出ない
        assert_eq!(t.by_code(610_200), ["北海道石狩郡当別町当別"]);
        assert!(t.by_code(9_999_999).is_empty());
    }

    #[test]
    fn missing_file_is_an_empty_table_and_bad_lines_are_errors() {
        let t = PostalTable::load(Path::new("/nonexistent/ikari/postal.tsv")).unwrap();
        assert!(t.is_empty());
        let e = PostalTable::from_tsv_str("# h\nあ\tア\t123\tf\n").unwrap_err();
        assert!(matches!(
            e,
            PostalError::BadLine {
                line: 2,
                reason: PostalLineError::BadCodes
            }
        ));
        assert!(PostalTable::from_tsv_str("").unwrap().is_empty());
    }

    #[test]
    fn crlf_is_accepted() {
        let t = PostalTable::from_tsv_str("あ\tア\t1000001\tf\r\n").unwrap();
        assert_eq!(t.by_code(1_000_001), ["ア"]);
    }
}
