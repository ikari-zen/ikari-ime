//! カタカナ語らしさの補正（#271）。
//!
//! 辞書にない語（未知語）の読みから作るカタカナの候補は、`Conversion::build_first_pool` で
//! 「辞書の候補の最大 base_cost + 1」（最後）に置かれる。読みに外来語に多い文字
//! （ヴ、小書きの母音、ぷ・ぺ・ー など）が多いときだけ、この base_cost を下げる。
//!
//! 考え方だけを azooKey の文字別の補正から借りている（表や値は写していない）。値は計測で決める
//! （ユーザー決定 2026-10-24: 採用。既定は強い手がかりだけ 5000、下限は辞書の最小コスト - 500）。

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::ranker::RankerConfigError;
use crate::ranker::config::ScopedKv;

/// 強い手がかり: 外来音に特有の文字（ゔ と小書きの母音。てぃ・ふぁ・しぇ などはこの小書きを含む）
const DEFAULT_STRONG_CHARS: &str = "ゔぁぃぅぇぉ";
/// 弱い手がかり: カタカナ語に多いが和語にもある文字
const DEFAULT_WEAK_CHARS: &str = "ぱぴぷぺぽふぐむー";
/// 強い手がかり 1 文字の下げ幅、弱い手がかり 1 文字の下げ幅、下限の既定（掃引で決めた。ADR 0020 §14）
const DEFAULT_STRONG_BONUS: i32 = 5000;
const DEFAULT_WEAK_BONUS: i32 = 0;
const DEFAULT_BELOW_BEST: i32 = 500;
/// K1 の既定は無効（0/0）。値は掃引の後に決める（ADR 0037 §5）
const DEFAULT_BASE: i32 = 0;
const DEFAULT_PER_CHAR: i32 = 0;
/// 弱い手がかりを数え始める個数の既定
const DEFAULT_WEAK_MIN_COUNT: u32 = 1;

/// カタカナの候補の base_cost の補正。`strong_bonus` と `weak_bonus` が両方 0 なら働かない。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KatakanaLikeRule {
    /// 強い手がかりの文字 1 つにつき下げる値。既定 5000（掃引で、全評価セットの 1 位が悪化しない最小の値。5000 未満は動かない）
    pub strong_bonus: i32,
    /// 弱い手がかりの文字 1 つにつき下げる値。既定 0（擬音語・和語を壊さない幅では利益がないので使わない）
    pub weak_bonus: i32,
    /// 強い手がかりの文字の集合
    pub strong_chars: Arc<str>,
    /// 弱い手がかりの文字の集合
    pub weak_chars: Arc<str>,
    /// 弱い手がかりが読みにこの個数以上あるときだけ数える。既定 1
    pub weak_min_count: u32,
    /// 下げた base_cost の下限 = 辞書の候補の最小 base_cost - この値。既定 500（0 だと同値で後に回るので上がらない）
    pub below_best: i32,
    /// K1（ADR 0037）: 未知語のカタカナの base_cost の上限 = `base + per_char × 読みの文字数`。この定数項。既定 0
    pub base: i32,
    /// K1: 読み 1 文字あたりの値。`base` と両方 0 なら K1 は働かない（既定。今と同じ）
    pub per_char: i32,
}

impl Default for KatakanaLikeRule {
    fn default() -> Self {
        Self {
            strong_bonus: DEFAULT_STRONG_BONUS,
            weak_bonus: DEFAULT_WEAK_BONUS,
            strong_chars: Arc::from(DEFAULT_STRONG_CHARS),
            weak_chars: Arc::from(DEFAULT_WEAK_CHARS),
            weak_min_count: DEFAULT_WEAK_MIN_COUNT,
            below_best: DEFAULT_BELOW_BEST,
            base: DEFAULT_BASE,
            per_char: DEFAULT_PER_CHAR,
        }
    }
}

impl KatakanaLikeRule {
    /// `--ranker-config` のキーの接頭辞
    pub const CONFIG_PREFIX: &'static str = "katakana_like";
    const CONFIG_KEYS: [&'static str; 8] = [
        "strong_bonus",
        "weak_bonus",
        "strong_chars",
        "weak_chars",
        "weak_min_count",
        "below_best",
        "base",
        "per_char",
    ];

    pub fn is_enabled(&self) -> bool {
        self.strong_bonus != 0 || self.weak_bonus != 0
    }

    /// `katakana_like.*` で `self` を上書きする（`NumeralCounterRule::with_kv` と同じ扱い）。
    pub fn with_kv(self, kv: &BTreeMap<String, String>) -> Result<Self, RankerConfigError> {
        let scope = ScopedKv::new(Self::CONFIG_PREFIX, &Self::CONFIG_KEYS, kv)?;
        Ok(Self {
            strong_bonus: scope.get("strong_bonus", self.strong_bonus)?,
            weak_bonus: scope.get("weak_bonus", self.weak_bonus)?,
            strong_chars: scope.get_arc_str("strong_chars", &self.strong_chars),
            weak_chars: scope.get_arc_str("weak_chars", &self.weak_chars),
            weak_min_count: scope.get("weak_min_count", self.weak_min_count)?,
            below_best: scope.get("below_best", self.below_best)?,
            base: scope.get("base", self.base)?,
            per_char: scope.get("per_char", self.per_char)?,
        })
    }

    /// K1（ADR 0037）が働くか。`base` と `per_char` が両方 0 なら働かない。
    pub fn is_unknown_cost_enabled(&self) -> bool {
        self.base != 0 || self.per_char != 0
    }

    /// K1: 未知語のカタカナの base_cost。`cost`（辞書の最大 + 1）より高くはしない。
    /// 辞書に同じ表記の 1 語があるカタカナには使わない（呼び出し側で分ける）。
    /// 文字数は読みのひらがなの文字数（拗音・長音も 1）。
    pub fn unknown_cost(&self, reading: &str, cost: i32) -> i32 {
        if !self.is_unknown_cost_enabled() {
            return cost;
        }
        let n = i32::try_from(reading.chars().count()).unwrap_or(i32::MAX);
        let cap = self.base.saturating_add(self.per_char.saturating_mul(n));
        cost.min(cap)
    }

    /// 読み（ひらがな）から下げる値。反復の読み（ぷんぷん、ぽかぽか）は擬音語・擬態語なので 0。
    pub fn discount(&self, reading: &str) -> i32 {
        if !self.is_enabled() || is_reduplication(reading) {
            return 0;
        }
        let (mut strong, mut weak) = (0i32, 0i32);
        for c in reading.chars() {
            if self.strong_chars.contains(c) {
                strong += 1;
            } else if self.weak_chars.contains(c) {
                weak += 1;
            }
        }
        let weak_counted = if u32::try_from(weak).is_ok_and(|w| w >= self.weak_min_count) {
            weak
        } else {
            0
        };
        strong
            .saturating_mul(self.strong_bonus)
            .saturating_add(weak_counted.saturating_mul(self.weak_bonus))
    }

    /// カタカナの候補の base_cost を下げた値。`cost` は今の値（未知語のカタカナなら辞書の最大 + 1、
    /// 辞書に同じ表記があればその候補の値）、`best` は辞書の最小。
    /// 上げることはなく、下限は `best - below_best`（`cost` が下限より低ければそのまま）。
    pub fn lowered_cost(&self, reading: &str, cost: i32, best: i32) -> i32 {
        let discount = self.discount(reading);
        if discount == 0 {
            return cost;
        }
        let floor = best.saturating_sub(self.below_best).min(cost);
        cost.saturating_sub(discount).max(floor)
    }
}

/// 読みが同じ塊の 2 回以上の繰り返し（ぷんぷん、ぽかぽか、ふーふー、ふふふ）か。擬音語・擬態語・笑い声に多い。
fn is_reduplication(reading: &str) -> bool {
    let chars: Vec<char> = reading.chars().collect();
    let n = chars.len();
    (1..=n / 2)
        .any(|unit| n.is_multiple_of(unit) && chars.chunks(unit).all(|c| c == &chars[..unit]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kv(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    fn rule(strong: i32, weak: i32) -> KatakanaLikeRule {
        KatakanaLikeRule {
            strong_bonus: strong,
            weak_bonus: weak,
            ..KatakanaLikeRule::default()
        }
    }

    #[test]
    fn default_is_the_adopted_values_and_zero_bonus_stops_the_rule() {
        let r = KatakanaLikeRule::default();
        assert!(r.is_enabled());
        assert_eq!((r.strong_bonus, r.weak_bonus, r.below_best), (5000, 0, 500));
        // 強い手がかり（ぃ）が 1 つあれば、5000 下がる（下限は最小コスト - 500）
        assert_eq!(r.lowered_cost("てぃー", 9000, 1000), 4000);
        // 手がかりがなければ変わらない
        assert_eq!(r.lowered_cost("さくら", 9000, 1000), 9000);
        // 0 にすれば止まる
        let off = rule(0, 0);
        assert!(!off.is_enabled());
        assert_eq!(off.lowered_cost("てぃー", 9000, 1000), 9000);
    }

    #[test]
    fn counts_strong_and_weak_chars() {
        let r = rule(1000, 100);
        // ふぁ: ふ(弱) ぁ(強)、ー(弱)
        assert_eq!(r.discount("ふぁー"), 1000 + 200);
        assert_eq!(r.discount("さくら"), 0);
    }

    #[test]
    fn reduplication_is_not_katakana_like() {
        let r = rule(1000, 1000);
        assert_eq!(r.discount("ぷんぷん"), 0);
        assert_eq!(r.discount("ぽかぽか"), 0);
        assert_eq!(r.discount("ふふふ"), 0);
        assert!(r.discount("ぐふふ") > 0);
        assert!(r.discount("ぷろぐらみんぐ") > 0);
    }

    #[test]
    fn weak_min_count_gates_weak_chars() {
        let r = KatakanaLikeRule {
            weak_min_count: 2,
            ..rule(0, 500)
        };
        assert_eq!(r.discount("ぷらんく"), 0);
        // ぷ ぐ ぐ の 3 つ
        assert_eq!(r.discount("ぷろぐらみんぐ"), 1500);
    }

    #[test]
    fn cost_is_clamped_between_floor_and_kana_cost() {
        let r = KatakanaLikeRule {
            below_best: 0,
            ..rule(10_000, 0)
        };
        // 大きく下げても、辞書の最小(1000) - below_best(0) より下がらない
        assert_eq!(r.lowered_cost("ふぁ", 9000, 1000), 1000);
        let r = KatakanaLikeRule {
            below_best: 300,
            ..r
        };
        assert_eq!(r.lowered_cost("ふぁ", 9000, 1000), 700);
        // 下げ幅が小さければその分だけ
        let r = rule(100, 0);
        assert_eq!(r.lowered_cost("ふぁ", 9000, 1000), 8900);
    }

    fn k1(base: i32, per_char: i32) -> KatakanaLikeRule {
        KatakanaLikeRule {
            base,
            per_char,
            ..rule(0, 0)
        }
    }

    #[test]
    fn unknown_cost_is_disabled_by_default() {
        let r = KatakanaLikeRule::default();
        assert!(!r.is_unknown_cost_enabled());
        assert_eq!((r.base, r.per_char), (0, 0));
        assert_eq!(r.unknown_cost("ばーじょにんぐ", 9000), 9000);
    }

    #[test]
    fn unknown_cost_caps_by_char_count_and_never_exceeds_cost() {
        let r = k1(1000, 500);
        // ばーじょにんぐ は 7 文字: 1000 + 500 * 7 = 4500
        assert_eq!(r.unknown_cost("ばーじょにんぐ", 9000), 4500);
        // 拗音（ょ）も 1 文字
        assert_eq!(r.unknown_cost("きょ", 9000), 2000);
        // 上限が cost より高ければ cost のまま（最大 + 1 を超えない）
        assert_eq!(r.unknown_cost("ばーじょにんぐ", 3000), 3000);
        // 桁あふれしない
        let big = k1(i32::MAX, i32::MAX);
        assert_eq!(big.unknown_cost("あい", 9000), 9000);
    }

    #[test]
    fn unknown_cost_is_applied_before_katakana_like_discount() {
        let r = KatakanaLikeRule {
            base: 1000,
            per_char: 500,
            ..rule(1000, 0)
        };
        let capped = r.unknown_cost("ふぁい", 9000);
        assert_eq!(capped, 2500);
        assert_eq!(r.lowered_cost("ふぁい", capped, 1000), 1500);
    }

    #[test]
    fn with_kv_overrides_k1_and_rejects_bad_input() {
        let base = KatakanaLikeRule::default();
        let r = base
            .clone()
            .with_kv(&kv(&[
                ("katakana_like.base", "1200"),
                ("katakana_like.per_char", "300"),
            ]))
            .unwrap();
        assert_eq!((r.base, r.per_char), (1200, 300));
        assert!(r.is_unknown_cost_enabled());
        for bad in [
            ("katakana_like.base", "x"),
            ("katakana_like.per_char", "1.5"),
            ("katakana_like.per_char", ""),
        ] {
            assert!(base.clone().with_kv(&kv(&[bad])).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn with_kv_overrides_and_rejects_bad_input() {
        let base = KatakanaLikeRule::default();
        let r = base
            .clone()
            .with_kv(&kv(&[
                ("katakana_like.strong_bonus", "2500"),
                ("katakana_like.weak_chars", "ぷぺ"),
                ("katakana_like.below_best", "50"),
                ("lattice.numeral_counter.bonus", "1"),
            ]))
            .unwrap();
        assert_eq!(r.strong_bonus, 2500);
        assert_eq!(&*r.weak_chars, "ぷぺ");
        assert_eq!(r.below_best, 50);
        assert_eq!(base.clone().with_kv(&BTreeMap::new()).unwrap(), base);
        for bad in [
            ("katakana_like.strong_bonus", "-1"),
            ("katakana_like.weak_bonus", "x"),
            ("katakana_like.nope", "1"),
        ] {
            assert!(base.clone().with_kv(&kv(&[bad])).is_err(), "{bad:?}");
        }
    }
}
