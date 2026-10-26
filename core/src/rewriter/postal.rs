//! 住所 ↔ 郵便番号の Rewriter（ADR 0032 §2、§4、§5、§6）。
//!
//! - 読み全体が住所の連なり（表の `by_reading` の完全一致）→ 郵便番号の候補。
//! - 読み全体が郵便番号の形 → 住所の候補（町が複数なら全部）。
//! - 町名だけの読みでは郵便番号を出さない（§6。表の読みは連なりだけで、町名単独の行はない）。
//!
//! `て` + 郵便番号（〒つき。§8。`PostalOptions::te_prefix`）は、〒つきの 2 つと住所を先頭（1 位から）に置く。
//! 組の中では `NumberRewriter` より前に置く（住所が数字の書き換えより先に出る。§4）。

use std::sync::Arc;

use ikari_dict_format::postal::POSTAL_CODE_DIGITS;

use super::{RewriteCandidate, RewriteInput, Rewriter};
use crate::postal::PostalTable;

/// 郵便番号の前半の桁数（`100-0001` の `100`）
const HEAD_DIGITS: usize = 3;
/// 住所 → 郵便番号で出す区切り（半角のハイフン）
const CODE_HYPHEN: char = '-';
/// 読みで受ける区切り（ADR 0032 §4。ローマ字の `-` は `ー` になる）
const CODE_SEPARATORS: [char; 5] = ['ー', '-', '－', '‐', '−'];
/// 〒つきの入力の頭（`テ`、`te` も読みでは `て`）
const TE_PREFIX: char = 'て';
/// 〒（郵便記号）
const POSTMARK: char = '〒';

// 書式ID（固定。変えない。学習が書式IDで行われるため。このRewriter の名前の中で一意）
/// 住所 → 郵便番号 `1000001`
pub const FORMAT_CODE_PLAIN: u16 = 1;
/// 住所 → 郵便番号 `100-0001`
pub const FORMAT_CODE_HYPHEN: u16 = 2;
/// 郵便番号 → 住所
pub const FORMAT_ADDRESS: u16 = 3;
/// `て` + 郵便番号 → `〒100-0001`
pub const FORMAT_POSTMARK_HYPHEN: u16 = 4;
/// `て` + 郵便番号 → `〒1000001`
pub const FORMAT_POSTMARK_PLAIN: u16 = 5;

/// 設定から決まる動作（`postal.japanese`、`postal.te_prefix`。`postal.english` は殻の側）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PostalOptions {
    /// `postal.japanese`（既定 on）: 住所 ↔ 郵便番号。off なら何も出さない
    pub japanese: bool,
    /// `postal.te_prefix`（既定 off）: `て` + 郵便番号 → 〒つき
    pub te_prefix: bool,
}

impl Default for PostalOptions {
    fn default() -> Self {
        Self {
            japanese: true,
            te_prefix: false,
        }
    }
}

pub struct PostalRewriter {
    table: Arc<PostalTable>,
    options: PostalOptions,
}

impl PostalRewriter {
    pub fn new(table: Arc<PostalTable>, options: PostalOptions) -> Self {
        Self { table, options }
    }

    /// 住所の連なり → 郵便番号（ADR 0032 §1、§5）。
    /// 郵便番号を昇順に、番号ごとに `1000001`、`100-0001` の順で出す。数では切らない。
    fn push_codes(&self, reading: &str, out: &mut Vec<RewriteCandidate>) {
        let mut codes: Vec<u32> = self
            .table
            .by_reading(reading)
            .iter()
            .flat_map(|m| m.codes.iter().copied())
            .collect();
        codes.sort_unstable();
        codes.dedup();
        for code in codes {
            let plain = format_code(code);
            let hyphen = format!(
                "{}{CODE_HYPHEN}{}",
                &plain[..HEAD_DIGITS],
                &plain[HEAD_DIGITS..]
            );
            out.push(RewriteCandidate::new(plain, FORMAT_CODE_PLAIN));
            out.push(RewriteCandidate::new(hyphen, FORMAT_CODE_HYPHEN));
        }
    }

    /// `て` + 郵便番号（ADR 0032 §8）。〒つき（ハイフンあり → なし）、住所の順に、先頭へ置く。
    /// 表にない番号でも〒つきは出す。住所は `postal.japanese` が on で表にあるときだけ。
    fn push_postmarked(&self, code: u32, out: &mut Vec<RewriteCandidate>) {
        let plain = format_code(code);
        let hyphen = format!(
            "{POSTMARK}{}{CODE_HYPHEN}{}",
            &plain[..HEAD_DIGITS],
            &plain[HEAD_DIGITS..]
        );
        out.push(RewriteCandidate::at_front(hyphen, FORMAT_POSTMARK_HYPHEN));
        out.push(RewriteCandidate::at_front(
            format!("{POSTMARK}{plain}"),
            FORMAT_POSTMARK_PLAIN,
        ));
        if self.options.japanese {
            for surface in self.table.by_code(code) {
                out.push(RewriteCandidate::at_front(
                    surface.to_owned(),
                    FORMAT_ADDRESS,
                ));
            }
        }
    }

    /// 郵便番号 → 住所（ADR 0032 §4、§5）。町をすべて、表の順に出す。表にない番号は何も出さない。
    fn push_addresses(&self, code: u32, out: &mut Vec<RewriteCandidate>) {
        for surface in self.table.by_code(code) {
            out.push(RewriteCandidate::new(surface.to_owned(), FORMAT_ADDRESS));
        }
    }
}

impl Rewriter for PostalRewriter {
    fn name(&self) -> &'static str {
        "postal"
    }

    fn rewrite(&self, input: &RewriteInput<'_>, out: &mut Vec<RewriteCandidate>) {
        if self.options.te_prefix
            && let Some(code) = input
                .reading
                .strip_prefix(TE_PREFIX)
                .and_then(parse_postal_code)
        {
            self.push_postmarked(code, out);
            return;
        }
        if !self.options.japanese || self.table.is_empty() {
            return;
        }
        match parse_postal_code(input.reading) {
            Some(code) => self.push_addresses(code, out),
            None => self.push_codes(input.reading, out),
        }
    }
}

/// 7 桁に直した郵便番号（先頭の 0 を保つ）
fn format_code(code: u32) -> String {
    format!("{code:0width$}", width = POSTAL_CODE_DIGITS)
}

/// 読みが郵便番号の形ならその数を返す（ADR 0032 §4）。
/// 数字 7 桁、または 数字 3 桁 + 区切り 1 字 + 数字 4 桁。数字は半角・全角（混在も可）。
/// 6 桁、8 桁、区切りが 2 つ以上、区切りの位置が違うものは None（途中の補完をしない）。
fn parse_postal_code(reading: &str) -> Option<u32> {
    let mut value: u32 = 0;
    let mut digits = 0usize;
    let mut separated = false;
    for c in reading.chars() {
        if let Some(d) = digit_value(c) {
            digits += 1;
            if digits > POSTAL_CODE_DIGITS {
                return None;
            }
            value = value * 10 + d;
        } else if CODE_SEPARATORS.contains(&c) {
            // 区切りは 3 桁目のあとに 1 つだけ
            if digits != HEAD_DIGITS || separated {
                return None;
            }
            separated = true;
        } else {
            return None;
        }
    }
    (digits == POSTAL_CODE_DIGITS).then_some(value)
}

fn digit_value(c: char) -> Option<u32> {
    match c {
        '0'..='9' => Some(c as u32 - '0' as u32),
        '０'..='９' => Some(c as u32 - '０' as u32),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::Context;
    use crate::rewriter::{Clock, FixedClock, LocalDateTime, RewritePosition};

    const SAMPLE: &str = include_str!("../../../dict-format/tests/fixtures/postal.sample.tsv");

    fn rewriter(options: PostalOptions) -> PostalRewriter {
        PostalRewriter::new(
            Arc::new(PostalTable::from_tsv_str(SAMPLE).unwrap()),
            options,
        )
    }

    fn run(rewriter: &PostalRewriter, reading: &str) -> Vec<(String, u16)> {
        let ctx = Context::default();
        let clock = FixedClock(LocalDateTime {
            year: 2026,
            month: 10,
            day: 23,
            hour: 0,
            minute: 0,
            weekday: 4,
        });
        let input = RewriteInput {
            reading,
            context: &ctx,
            now: clock.now(),
        };
        let mut out = Vec::new();
        rewriter.rewrite(&input, &mut out);
        out.into_iter().map(|c| (c.surface, c.format)).collect()
    }

    fn surfaces(v: &[(String, u16)]) -> Vec<&str> {
        v.iter().map(|(s, _)| s.as_str()).collect()
    }

    #[test]
    fn address_gives_plain_then_hyphen() {
        let r = rewriter(PostalOptions::default());
        let got = run(&r, "とうきょうとちよだくちよだ");
        assert_eq!(
            got,
            [
                ("1000001".to_owned(), FORMAT_CODE_PLAIN),
                ("100-0001".to_owned(), FORMAT_CODE_HYPHEN)
            ]
        );
        // 都道府県を省いた形も同じ
        assert_eq!(
            surfaces(&run(&r, "ちよだくちよだ")),
            ["1000001", "100-0001"]
        );
    }

    #[test]
    fn leading_zero_is_kept() {
        let r = rewriter(PostalOptions::default());
        assert_eq!(
            surfaces(&run(&r, "さっぽろしちゅうおうくおおどおりにし")),
            ["0600042", "060-0042"]
        );
    }

    #[test]
    fn several_codes_of_one_town_are_all_given_in_ascending_order() {
        let r = rewriter(PostalOptions::default());
        assert_eq!(
            surfaces(&run(&r, "とうきょうとちよだくまるのうち")),
            ["1000005", "100-0005", "1006390", "100-6390"]
        );
    }

    #[test]
    fn town_name_alone_gives_nothing() {
        let r = rewriter(PostalOptions::default());
        assert!(run(&r, "ちよだ").is_empty());
        assert!(run(&r, "ちよだく").is_empty());
        assert!(run(&r, "まるのうち").is_empty());
        assert!(run(&r, "").is_empty());
    }

    #[test]
    fn code_gives_addresses_for_every_accepted_shape() {
        let r = rewriter(PostalOptions::default());
        for reading in [
            "1000001",
            "100-0001",
            "100ー0001",
            "100－0001",
            "100‐0001",
            "100−0001",
            "１０００００１",
            "１００－０００１",
            "100ー０００1",
        ] {
            assert_eq!(
                run(&r, reading),
                [("東京都千代田区千代田".to_owned(), FORMAT_ADDRESS)],
                "{reading}"
            );
        }
    }

    #[test]
    fn code_with_several_towns_gives_all_in_table_order() {
        let r = rewriter(PostalOptions::default());
        assert_eq!(
            surfaces(&run(&r, "060-0042")),
            ["北海道札幌市中央区大通西", "北海道札幌市中央区大通東"]
        );
        // 郡を省いた形（s）は出ない。f の行は 1 つ
        assert_eq!(surfaces(&run(&r, "0610200")), ["北海道石狩郡当別町当別"]);
    }

    #[test]
    fn malformed_codes_give_nothing() {
        let r = rewriter(PostalOptions::default());
        for reading in [
            "100000",
            "10000012",
            "100--0001",
            "10-00001",
            "1000-001",
            "-1000001",
            "1000001-",
            "100 0001",
            "100ー0001ー",
            "1000001あ",
            // 表にない番号
            "9999999",
            "999-9999",
        ] {
            assert!(run(&r, reading).is_empty(), "{reading}");
        }
    }

    #[test]
    fn japanese_off_gives_nothing() {
        let r = rewriter(PostalOptions {
            japanese: false,
            ..PostalOptions::default()
        });
        assert!(run(&r, "とうきょうとちよだくちよだ").is_empty());
        assert!(run(&r, "1000001").is_empty());
    }

    /// 人工データ（郵便番号 11 万種、町 13.5 万 × f と s = 27 万行）での読み込み時間と引き。
    /// 実データ（レーン D の postal.tsv）が出るまでの目安。
    ///   cargo test --release -p ikari-core --lib perf_synthetic_table -- --ignored --nocapture
    /// メモリは `/usr/bin/time -l` の maximum resident set size から、テキスト分を引いて見る。
    #[test]
    #[ignore = "perf measurement (prints numbers)"]
    fn perf_synthetic_table() {
        const TOWNS: usize = 135_000;
        const CODES: usize = 110_000;
        const KANA: [char; 10] = ['あ', 'い', 'う', 'え', 'お', 'か', 'き', 'く', 'け', 'こ'];
        let kana_of = |mut n: usize| -> String {
            let mut s = String::new();
            for _ in 0..7 {
                s.push(KANA[n % KANA.len()]);
                n /= KANA.len();
            }
            s
        };
        let mut text = String::new();
        for i in 0..TOWNS {
            let code = 1_000_000 + (i * CODES / TOWNS) as u32;
            let k = kana_of(i);
            text.push_str(&format!(
                "とうきょうとちよだく{k}\t東京都千代田区千代田{i}\t{code:07}\tf\n"
            ));
            text.push_str(&format!("ちよだく{k}\t千代田区千代田{i}\t{code:07}\ts\n"));
        }
        let t0 = std::time::Instant::now();
        let table = PostalTable::from_tsv_str(&text).unwrap();
        let load = t0.elapsed();
        let t1 = std::time::Instant::now();
        let mut hits = 0usize;
        for i in (0..TOWNS).step_by(97) {
            hits += table
                .by_reading(&format!("とうきょうとちよだく{}", kana_of(i)))
                .len();
            hits += table.by_code(1_000_000 + (i * CODES / TOWNS) as u32).len();
        }
        let lookups = TOWNS.div_ceil(97) * 2;
        let per = t1.elapsed() / lookups as u32;
        println!(
            "text {} bytes, rows {}, load {load:?}, {lookups} lookups avg {per:?}, hits {hits}",
            text.len(),
            table.len()
        );
    }

    #[test]
    fn empty_table_gives_nothing() {
        let r = PostalRewriter::new(Arc::new(PostalTable::empty()), PostalOptions::default());
        assert!(run(&r, "1000001").is_empty());
        assert!(run(&r, "ちよだくちよだ").is_empty());
        assert_eq!(r.name(), "postal");
    }

    fn te_on() -> PostalRewriter {
        rewriter(PostalOptions {
            japanese: true,
            te_prefix: true,
        })
    }

    #[test]
    fn te_prefix_gives_postmark_then_address_at_front() {
        let r = te_on();
        for reading in ["て1000001", "て100-0001", "て１００ー０００１"] {
            let got = run(&r, reading);
            assert_eq!(
                surfaces(&got),
                ["〒100-0001", "〒1000001", "東京都千代田区千代田"],
                "{reading}"
            );
            assert_eq!(got[0].1, FORMAT_POSTMARK_HYPHEN);
            assert_eq!(got[1].1, FORMAT_POSTMARK_PLAIN);
        }
        // 先頭の位置で足す
        let ctx = Context::default();
        let input = RewriteInput {
            reading: "て1000001",
            context: &ctx,
            now: FixedClock(LocalDateTime {
                year: 2026,
                month: 10,
                day: 23,
                hour: 0,
                minute: 0,
                weekday: 4,
            })
            .now(),
        };
        let mut out = Vec::new();
        r.rewrite(&input, &mut out);
        assert!(out.iter().all(|c| c.position == RewritePosition::Front));
    }

    #[test]
    fn te_prefix_is_off_by_default_and_needs_a_full_code() {
        let off = rewriter(PostalOptions::default());
        assert!(run(&off, "て1000001").is_empty());
        let r = te_on();
        assert!(run(&r, "て").is_empty());
        assert!(run(&r, "て100000").is_empty());
        assert!(run(&r, "て10000012").is_empty());
        assert!(run(&r, "てすと1000001").is_empty());
        // 〒なしの形は今までどおり（住所だけ）
        assert_eq!(surfaces(&run(&r, "1000001")), ["東京都千代田区千代田"]);
    }

    #[test]
    fn te_prefix_gives_postmarks_even_for_unknown_code_or_japanese_off() {
        let r = te_on();
        assert_eq!(surfaces(&run(&r, "て9999999")), ["〒999-9999", "〒9999999"]);
        let no_addr = rewriter(PostalOptions {
            japanese: false,
            te_prefix: true,
        });
        assert_eq!(
            surfaces(&run(&no_addr, "て1000001")),
            ["〒100-0001", "〒1000001"]
        );
    }
}
