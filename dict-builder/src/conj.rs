//! 活用規則の導出（ADR 0002 §3.3）。
//!
//! 活用型が `*` でない各行について、辞書形ID の行（基準形）との差から
//! 「基準形の品詞 -> この行の品詞」の規則（末尾を削る文字数、足す表記、足す読み）を求める。
//! `(base_pos, target_pos)` ごとに最も多い規則を採用し、支持数が `MIN_SUPPORT` 未満なら捨てる。

use std::collections::{BTreeMap, HashMap};

use crate::lex::InflectRow;

/// 規則を採用する最小の支持数
pub const MIN_SUPPORT: u32 = 3;

/// 導出した規則 1件（品詞は最終的な pos_id）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedRule {
    pub base_pos: u16,
    pub target_pos: u16,
    pub strip_chars: u8,
    pub add_surface: String,
    pub add_reading: String,
    /// この規則を支持する行の数
    pub support: u32,
}

/// 導出の統計。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConjStats {
    /// 辞書形ID を持つ活用行の数
    pub candidates: usize,
    /// 辞書形が見つからない、または自分と同じ品詞（恒等）なので使わなかった行
    pub skipped_no_base: usize,
    /// 表記と読みで削る文字数が違う、または長さが u8 に収まらないので使えなかった行
    pub skipped_unusable: usize,
    /// 支持数が足りずに捨てた `(base_pos, target_pos)` の数
    pub dropped_pairs: usize,
}

type RuleKey = (u8, String, String);

/// `(strip, add)` を求める。共通接頭辞は文字単位。
fn diff(base: &str, target: &str) -> (usize, String) {
    let common = base
        .chars()
        .zip(target.chars())
        .take_while(|(a, b)| a == b)
        .count();
    let strip = base.chars().count() - common;
    let add: String = target.chars().skip(common).collect();
    (strip, add)
}

/// 活用行から規則を導く。`rows` は `id` の昇順。`pos_final` は仮の品詞 ID から pos_id への対応。
pub fn derive(rows: &[InflectRow], pos_final: &[u16]) -> (Vec<DerivedRule>, ConjStats) {
    let mut stats = ConjStats::default();
    let mut votes: BTreeMap<(u16, u16), HashMap<RuleKey, u32>> = BTreeMap::new();
    for row in rows {
        // Full の notcore の行は数えない（grammar.bin を Core だけのときと同じにする。ADR 0012 §13）
        if row.notcore {
            continue;
        }
        let Some(base_id) = row.dict_form else {
            continue;
        };
        stats.candidates += 1;
        let base = rows
            .binary_search_by_key(&base_id, |r| r.id)
            .ok()
            .map(|i| &rows[i]);
        let Some(base) = base.filter(|b| b.pos_tmp != row.pos_tmp) else {
            stats.skipped_no_base += 1;
            continue;
        };
        let (strip_surface, add_surface) = diff(&base.surface, &row.surface);
        let (strip_reading, add_reading) = diff(&base.reading, &row.reading);
        let usable = strip_surface == strip_reading
            && u8::try_from(strip_surface).is_ok()
            && u8::try_from(add_surface.len()).is_ok()
            && u8::try_from(add_reading.len()).is_ok();
        if !usable {
            stats.skipped_unusable += 1;
            continue;
        }
        let (Some(&base_pos), Some(&target_pos)) = (
            pos_final.get(base.pos_tmp as usize),
            pos_final.get(row.pos_tmp as usize),
        ) else {
            stats.skipped_no_base += 1;
            continue;
        };
        *votes
            .entry((base_pos, target_pos))
            .or_default()
            .entry((strip_surface as u8, add_surface, add_reading))
            .or_insert(0) += 1;
    }

    let mut rules = Vec::new();
    for ((base_pos, target_pos), counts) in votes {
        // 最多の規則。同数なら (strip, add_surface, add_reading) が小さい方（決定的にする）
        let best = counts
            .into_iter()
            .max_by(|(ka, ca), (kb, cb)| ca.cmp(cb).then_with(|| kb.cmp(ka)));
        match best {
            Some(((strip_chars, add_surface, add_reading), support)) if support >= MIN_SUPPORT => {
                rules.push(DerivedRule {
                    base_pos,
                    target_pos,
                    strip_chars,
                    add_surface,
                    add_reading,
                    support,
                });
            }
            _ => stats.dropped_pairs += 1,
        }
    }
    (rules, stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(
        id: u32,
        surface: &str,
        reading: &str,
        pos_tmp: u32,
        dict_form: Option<u32>,
    ) -> InflectRow {
        InflectRow {
            id,
            surface: surface.to_string(),
            reading: reading.to_string(),
            pos_tmp,
            dict_form,
            notcore: false,
        }
    }

    #[test]
    fn diff_counts_characters_not_bytes() {
        assert_eq!(diff("書く", "書き"), (1, "き".to_string()));
        assert_eq!(diff("進める", "進め"), (1, String::new()));
        assert_eq!(diff("かく", "かい"), (1, "い".to_string()));
        assert_eq!(diff("来る", "来"), (1, String::new()));
        assert_eq!(diff("ab", "ab"), (0, String::new()));
        assert_eq!(diff("a", "xyz"), (1, "xyz".to_string()));
    }

    /// 仮の品詞 0 = 終止形、1 = 連用形、2 = 未然形。
    fn verbs(n: u32) -> Vec<InflectRow> {
        let stems = ["書", "聞", "歩", "急", "泳"];
        let kana = ["か", "き", "ある", "いそ", "およ"];
        let mut rows = Vec::new();
        let mut id = 0;
        for i in 0..n as usize {
            let (s, k) = (stems[i], kana[i]);
            let base = id;
            rows.push(row(
                base,
                &format!("{s}く"),
                &format!("{k}く"),
                0,
                Some(base),
            ));
            rows.push(row(
                base + 1,
                &format!("{s}き"),
                &format!("{k}き"),
                1,
                Some(base),
            ));
            rows.push(row(
                base + 2,
                &format!("{s}か"),
                &format!("{k}か"),
                2,
                Some(base),
            ));
            id += 3;
        }
        rows
    }

    #[test]
    fn rules_need_minimum_support() {
        let pos_final = [10u16, 11, 12];
        let (rules, stats) = derive(&verbs(3), &pos_final);
        assert_eq!(rules.len(), 2);
        assert_eq!(
            rules[0],
            DerivedRule {
                base_pos: 10,
                target_pos: 11,
                strip_chars: 1,
                add_surface: "き".to_string(),
                add_reading: "き".to_string(),
                support: 3,
            }
        );
        assert_eq!((rules[1].base_pos, rules[1].target_pos), (10, 12));
        // 終止形自身（恒等）は規則にしない
        assert_eq!(stats.skipped_no_base, 3);
        assert_eq!(stats.dropped_pairs, 0);

        // 支持が2件しかなければ捨てる
        let (rules, stats) = derive(&verbs(2), &pos_final);
        assert!(rules.is_empty());
        assert_eq!(stats.dropped_pairs, 2);
    }

    #[test]
    fn majority_rule_wins_and_ties_are_deterministic() {
        let mut rows = verbs(3);
        // 連用形に例外が1件（支持 1）
        let id = rows.len() as u32;
        rows.push(row(id, "行っ", "いっ", 1, Some(0)));
        let (rules, _) = derive(&rows, &[0, 1, 2]);
        let renyou = rules.iter().find(|r| r.target_pos == 1).unwrap();
        assert_eq!(renyou.add_surface, "き");
        assert_eq!(renyou.support, 3);
    }

    #[test]
    fn rows_with_mismatched_strip_are_unusable() {
        // 来る/くる -> 来/き: 表記は1文字、読みは2文字削る
        let mut rows = Vec::new();
        for i in 0..4 {
            rows.push(row(i * 2, "来る", "くる", 0, Some(i * 2)));
            rows.push(row(i * 2 + 1, "来", "き", 1, Some(i * 2)));
        }
        let (rules, stats) = derive(&rows, &[0, 1]);
        assert!(rules.is_empty());
        assert_eq!(stats.skipped_unusable, 4);
    }

    #[test]
    fn missing_or_forward_references() {
        let rows = vec![
            row(5, "書き", "かき", 1, Some(7)), // 辞書形が後ろ（昇順なので見つかる）
            row(7, "書く", "かく", 0, Some(7)),
            row(9, "聞き", "きき", 1, Some(100)), // 辞書形が存在しない
            row(11, "言う", "いう", 0, None),
        ];
        let (_, stats) = derive(&rows, &[0, 1]);
        assert_eq!(stats.candidates, 3);
        assert_eq!(stats.skipped_no_base, 2); // 存在しない参照と恒等
    }

    #[test]
    fn derive_is_deterministic() {
        let a = derive(&verbs(5), &[0, 1, 2]);
        let b = derive(&verbs(5), &[0, 1, 2]);
        assert_eq!(a, b);
    }
}
