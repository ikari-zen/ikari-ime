//! 再変換の読みの推定の一致率（ADR 0006 §7.1-5、`ikari-eval reading`）。
//!
//! 既存の評価セットの `(reading, expected)` の組を使う。`expected` の表記だけを `Converter::reconvert` に渡し、
//! 推定した読み（文節の読みをつないだもの）が `reading` と一致する率を出す。
//! 読みが一意に決まらない語（今日: きょう／こんにち）は、推定が違っても失敗ではなく仕様
//! （ユーザーが BackSpace で読みを直せる。§9.3-3）なので、この率は 100% を目指さず、記録して比べる。

use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;

use crate::EvalCase;
use crate::segment_engine::SegmentEngine;

/// 推定する 1 組
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadingPair {
    /// 出どころの評価セット（ファイル名など）
    pub set: String,
    pub id: String,
    pub context: String,
    pub reading: String,
    pub surface: String,
}

/// 評価セットのケースから組を作る。`expected` の別解は別の組にし、同じセットの中で同じ組は 1 つにする
pub fn pairs_from_cases(set: &str, cases: &[EvalCase]) -> Vec<ReadingPair> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for c in cases {
        for surface in c.alternatives() {
            if seen.insert((c.reading.clone(), surface.to_owned())) {
                out.push(ReadingPair {
                    set: set.to_owned(),
                    id: c.id.clone(),
                    context: c.context.clone(),
                    reading: c.reading.clone(),
                    surface: surface.to_owned(),
                });
            }
        }
    }
    out
}

/// 1 組の結果
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadingResult {
    pub pair: ReadingPair,
    /// 推定した読み。再変換を始められなかった（`Empty` など）ときは None
    pub estimated: Option<String>,
}

impl ReadingResult {
    pub fn matched(&self) -> bool {
        self.estimated.as_deref() == Some(self.pair.reading.as_str())
    }
}

/// 全組の読みを推定する
pub fn evaluate_readings<E: SegmentEngine>(
    engine: &E,
    pairs: &[ReadingPair],
) -> Vec<ReadingResult> {
    pairs
        .iter()
        .map(|p| {
            // エラー（読める部分がない `Empty` など）は、再変換を始められなかったものとして数える
            let estimated = engine.reconvert(&p.surface, &p.context).ok().map(|layout| {
                layout
                    .spans
                    .iter()
                    .map(|s| s.reading.as_str())
                    .collect::<String>()
            });
            ReadingResult {
                pair: p.clone(),
                estimated,
            }
        })
        .collect()
}

/// 集計 1 行分
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReadingGroup {
    pub total: usize,
    pub matched: usize,
    /// 再変換を始められなかった数（一致しなかった数に含む）
    pub not_started: usize,
}

impl ReadingGroup {
    pub fn rate(&self) -> f64 {
        if self.total == 0 {
            1.0
        } else {
            self.matched as f64 / self.total as f64
        }
    }
}

/// セットごとと全体の集計
pub fn summarize_readings(
    results: &[ReadingResult],
) -> (ReadingGroup, BTreeMap<String, ReadingGroup>) {
    let mut all = ReadingGroup::default();
    let mut by_set: BTreeMap<String, ReadingGroup> = BTreeMap::new();
    for r in results {
        for g in [&mut all, by_set.entry(r.pair.set.clone()).or_default()] {
            g.total += 1;
            g.matched += usize::from(r.matched());
            g.not_started += usize::from(r.estimated.is_none());
        }
    }
    (all, by_set)
}

/// Markdown の表と、一致しなかった組の先頭 `list` 件
pub fn render_readings(results: &[ReadingResult], list: usize) -> String {
    let (all, by_set) = summarize_readings(results);
    let mut out = String::new();
    out.push_str("## 再変換の読みの推定\n\n| セット | 組 | 一致 | 一致率 | 始められない |\n|---|---:|---:|---:|---:|\n");
    let row = |out: &mut String, name: &str, g: &ReadingGroup| {
        let _ = writeln!(
            out,
            "| {name} | {} | {} | {:.1}% | {} |",
            g.total,
            g.matched,
            g.rate() * 100.0,
            g.not_started
        );
    };
    for (set, g) in &by_set {
        row(&mut out, set, g);
    }
    row(&mut out, "全体", &all);

    let misses: Vec<&ReadingResult> = results.iter().filter(|r| !r.matched()).collect();
    if list > 0 && !misses.is_empty() {
        let _ = write!(
            out,
            "\n### 一致しなかった組（先頭 {} 件 / {} 件）\n\n| セット | id | 表記 | 正解の読み | 推定 |\n|---|---|---|---|---|\n",
            list.min(misses.len()),
            misses.len()
        );
        for r in misses.into_iter().take(list) {
            let _ = writeln!(
                out,
                "| {} | {} | {} | {} | {} |",
                r.pair.set,
                r.pair.id,
                r.pair.surface,
                r.pair.reading,
                r.estimated.as_deref().unwrap_or("(始められない)")
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load_cases;
    use crate::segment_engine::mock::MockEngine;

    fn cases() -> Vec<EvalCase> {
        load_cases(
            "id\tkind\tcontext\treading\texpected\n\
             a\t話題\t\tひんし\t品詞|瀕死\n\
             b\t話題\t\tきょう\t今日\n\
             c\t話題\t\tひんし\t品詞\n\
             d\t話題\t\tふめい\t不明\n",
        )
        .unwrap()
    }

    fn engine() -> MockEngine {
        let mut e = MockEngine::default();
        let mut put = |surface: &str, segs: &[&str]| {
            e.reconverted.insert(
                surface.to_owned(),
                segs.iter().map(|s| (*s).to_owned()).collect(),
            );
        };
        put("品詞", &["ひんし"]);
        put("瀕死", &["ひんじ"]); // 読みが違う
        put("今日", &["こん", "にち"]); // 区切りが違っても、つないだ読みで比べる（これは違う読み）
        e
    }

    #[test]
    fn pairs_expand_alternatives_and_drop_duplicates() {
        let pairs = pairs_from_cases("x.tsv", &cases());
        let shown: Vec<(&str, &str)> = pairs
            .iter()
            .map(|p| (p.reading.as_str(), p.surface.as_str()))
            .collect();
        assert_eq!(
            shown,
            [
                ("ひんし", "品詞"),
                ("ひんし", "瀕死"),
                ("きょう", "今日"),
                ("ふめい", "不明")
            ]
        );
    }

    #[test]
    fn compares_the_joined_reading_and_counts_not_started() {
        let pairs = pairs_from_cases("x.tsv", &cases());
        let results = evaluate_readings(&engine(), &pairs);
        let matched: Vec<bool> = results.iter().map(ReadingResult::matched).collect();
        assert_eq!(matched, [true, false, false, false]);
        assert_eq!(results[1].estimated.as_deref(), Some("ひんじ"));
        assert_eq!(results[2].estimated.as_deref(), Some("こんにち"));
        assert_eq!(results[3].estimated, None, "reconvert fails -> not started");

        let (all, by_set) = summarize_readings(&results);
        assert_eq!(
            all,
            ReadingGroup {
                total: 4,
                matched: 1,
                not_started: 1
            }
        );
        assert_eq!(by_set["x.tsv"], all);
        assert!((all.rate() - 0.25).abs() < 1e-9);
    }

    #[test]
    fn renders_the_table_and_limits_the_miss_list() {
        let results = evaluate_readings(&engine(), &pairs_from_cases("x.tsv", &cases()));
        let md = render_readings(&results, 2);
        assert!(md.contains("| 全体 | 4 | 1 | 25.0% | 1 |"), "{md}");
        assert!(md.contains("先頭 2 件 / 3 件"), "{md}");
        assert!(
            md.contains("| x.tsv | a | 瀕死 | ひんし | ひんじ |"),
            "{md}"
        );
        assert!(!md.contains("不明"), "limited to 2 rows: {md}");
        assert!(!render_readings(&results, 0).contains("一致しなかった組"));
    }

    #[test]
    fn bundled_sets_make_a_few_hundred_unique_pairs() {
        // 同じ (読み, 表記) は文脈が違っても 1 組に数える（再変換が使うのは表記だけ）
        let mut total = 0;
        for tsv in [
            include_str!("../sets/homophones.tsv"),
            include_str!("../sets/homophones-ext.tsv"),
            include_str!("../sets/hiragana.tsv"),
            include_str!("../sets/katakana-suffix.tsv"),
        ] {
            total += pairs_from_cases("s", &load_cases(tsv).unwrap()).len();
        }
        assert!((200..3000).contains(&total), "{total}");
    }
}
