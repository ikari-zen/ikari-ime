//! 変換の設定（`ConvertOptions`）と LM を待つ時間の規則（`LmBudget`）。

use std::collections::BTreeMap;
use std::time::Duration;

use super::katakana_like::KatakanaLikeRule;
use crate::ranker::RankerConfigError;
use crate::ranker::config::ScopedKv;

const DEFAULT_PAGE_SIZE: usize = 9;
const DEFAULT_POOL_SIZE: usize = 30;
const DEFAULT_MAX_ASTAR_POPS: usize = 20_000;
/// Rewriter の候補を入れる位置の既定（辞書の候補を昇順に並べたときの番号。1 = 辞書の1位の直後）
const DEFAULT_REWRITER_ANCHOR: usize = 1;
/// 日付の Rewriter の候補を入れる位置の既定（`DEFAULT_REWRITER_ANCHOR` と同じ。#80）
const DEFAULT_DATE_ANCHOR: usize = 1;
/// 日付の位置を `--ranker-config` で指す接頭辞とキー（`rewriter.date_anchor`）
const DATE_ANCHOR_PREFIX: &str = "rewriter";
const DATE_ANCHOR_KEYS: [&str; 1] = ["date_anchor"];
/// LM を待つ時間の既定（ADR 0004 §3.4、ADR 0003 §7.2）。`max(30ms, 読み1文字あたり 6ms)`、上限 300ms
const DEFAULT_LM_BUDGET_BASE: Duration = Duration::from_millis(30);
const DEFAULT_LM_BUDGET_PER_CHAR: Duration = Duration::from_millis(6);
const DEFAULT_LM_BUDGET_MAX: Duration = Duration::from_millis(300);
/// 副詞の直後の名詞の辺に足す値の既定（ADR 0008 §2.1、§11.1）
const DEFAULT_ADVERB_NOUN_PENALTY: i32 = 6000;
/// 「覆う名詞」とみなす変換用のコスト（`cost + penalty`）の上限の既定（ADR 0008 §2.1、§11.1）
const DEFAULT_ADVERB_NOUN_COVER_MAX_COST: i32 = 10_000;
/// 名詞 → 接尾辞 の辺に足す値の既定（ADR 0008 §13。B25）
const DEFAULT_NOUN_SUFFIX_PENALTY: i32 = 6000;
/// 名詞 → 接尾辞 の辺の「覆う名詞」のコストの上限の既定（ADR 0008 §13。副詞の直後の名詞と同じ値）
const DEFAULT_NOUN_SUFFIX_COVER_MAX_COST: i32 = 10_000;
const DEFAULT_NUMERAL_COVER_PENALTY: i32 = 6000;
const DEFAULT_NUMERAL_COVER_COVER_MAX_COST: i32 = 10_000;
/// サ変でない名詞 → する の辺に足す値の既定（ADR 0016 §2.1、§9）
const DEFAULT_NOUN_SURU_PENALTY: i32 = 3000;
/// 助詞・副詞・接続詞 → しよう（サ変の意志形）の辺から引く値、しよう → 動詞・名詞・接尾辞 の辺に足す値の既定（ADR 0016 §10。B63）
const DEFAULT_VOLITIONAL_BONUS: i32 = 5000;
/// サ変可能の名詞 → しよう の辺から引く値の既定（ADR 0016 §12。#230。実測の幅は 1200〜1500。LM ありの害が最小の上限）
const DEFAULT_VOLITIONAL_SAHEN_BONUS: i32 = 1500;
/// 数詞 → 助数詞可能の名詞 の辺から引く値の既定（ADR 0019 §15。実測の安全な幅は 2800〜4000）
const DEFAULT_NUMERAL_COUNTER_BONUS: i32 = 3500;
/// 数詞 → 接尾辞,助数詞（個・年生）の辺から引く値の既定（ADR 0019 §16。実測の安全な幅は 5000〜10000）
const DEFAULT_NUMERAL_COUNTER_SUFFIX_BONUS: i32 = 7000;
/// 数詞 → 日・カ（読み か）の辺に足す値の既定（ADR 0019 §16。実測の下限は 4000。上は 30000 まで壊れない）
const DEFAULT_NUMERAL_COUNTER_KA_PENALTY: i32 = 8000;
/// 数詞 → 住所・番号の助数詞の表記（番・番地 など。`ADDRESS_COUNTER_SURFACES`）の辺から引く値の既定（ADR 0030 §13.5）
const DEFAULT_NUMERAL_COUNTER_ADDRESS_BONUS: i32 = 4000;
/// か国 → 語 の辺から引く値（ADR 0019 §18。4500 までは足りず、5000〜20000 は同じ結果）
const DEFAULT_NUMERAL_COUNTER_LANGUAGE_BONUS: i32 = 8000;
// 「第」+ 数詞のあとの序数の助数詞（#240）。掃引（LM あり・なし、全評価セット）で 1 位が悪化せず 第5話 まで直る最小の値（7000 では 第5羽 が残る）
const DEFAULT_NUMERAL_COUNTER_ORDINAL_BONUS: i32 = 8000;
/// 上のうち、かな読みの数詞（さん → 三）からの辺の既定（ADR 0030 §13.5）
const DEFAULT_NUMERAL_COUNTER_ADDRESS_KANA_BONUS: i32 = 6000;
/// 番号ラベル 1〜9 に合わせた1ページの件数の上限
pub(super) const MAX_PAGE_SIZE: usize = 9;

/// 変換1回あたりに LM を待つ時間の規則。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LmBudget {
    /// 下限（既定 30ms）
    pub base: Duration,
    /// 読み1文字あたり（既定 6ms）
    pub per_char: Duration,
    /// 上限（既定 300ms）
    pub max: Duration,
}

impl Default for LmBudget {
    fn default() -> Self {
        Self {
            base: DEFAULT_LM_BUDGET_BASE,
            per_char: DEFAULT_LM_BUDGET_PER_CHAR,
            max: DEFAULT_LM_BUDGET_MAX,
        }
    }
}

impl LmBudget {
    /// `min(max, max(base, per_char * 読みの文字数))`
    pub fn for_reading(&self, reading: &str) -> Duration {
        let chars = u32::try_from(reading.chars().count()).unwrap_or(u32::MAX);
        self.per_char
            .saturating_mul(chars)
            .max(self.base)
            .min(self.max)
    }
}

/// 変換のラティスの辺の補正: 副詞の直後の名詞（ADR 0008）。
///
/// 副詞のノード A → 名詞のノード N の辺に `penalty` を足す。ただし、A の開始位置から N の終了位置までを
/// ちょうど1語で覆う、品詞1 = 名詞で変換用のコスト（`cost + penalty`）が `cover_max_cost` 以下の
/// 辞書のノードがラティスにあるときだけ（そう＋時期 と 掃除機）。BOS と EOS の辺には足さない。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdverbNounRule {
    /// 辺に足す値。0 で規則を止める。既定 6000
    pub penalty: i32,
    /// 覆う名詞の変換用のコストの上限。既定 10000
    pub cover_max_cost: i32,
}

impl Default for AdverbNounRule {
    fn default() -> Self {
        Self {
            penalty: DEFAULT_ADVERB_NOUN_PENALTY,
            cover_max_cost: DEFAULT_ADVERB_NOUN_COVER_MAX_COST,
        }
    }
}

impl AdverbNounRule {
    /// `--ranker-config` のキーの接頭辞（設定アプリには出さない。ADR 0008 §3.2、§11.1-4）
    pub const CONFIG_PREFIX: &'static str = "lattice.adverb_noun";
    const CONFIG_KEYS: [&'static str; 2] = ["penalty", "cover_max_cost"];

    /// 規則を止めた値（`penalty == 0`）
    pub fn disabled() -> Self {
        Self {
            penalty: 0,
            ..Self::default()
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.penalty != 0
    }

    /// `lattice.adverb_noun.penalty` と `lattice.adverb_noun.cover_max_cost` で `self` を上書きする。
    /// 他の接頭辞のキーは無視し、この接頭辞で未知のキーや負の値、数でない値ならエラー。
    pub fn with_kv(self, kv: &BTreeMap<String, String>) -> Result<Self, RankerConfigError> {
        let scope = ScopedKv::new(Self::CONFIG_PREFIX, &Self::CONFIG_KEYS, kv)?;
        Ok(Self {
            penalty: scope.get("penalty", self.penalty)?,
            cover_max_cost: scope.get("cover_max_cost", self.cover_max_cost)?,
        })
    }
}

/// 変換のラティスの辺の補正: 名詞 → 接尾辞（ADR 0008 §13。B25）。
///
/// 辞書のノード N（品詞1 = 名詞）→ 辞書のノード S（品詞1 = 接尾辞）の辺に、N の開始位置から S の終了位置までを
/// ちょうど1語で覆う、品詞1 = 名詞で変換用のコストが `cover_max_cost` 以下の辞書のノードがあるときだけ `penalty` を足す
/// （低数＋名 に対する 定数名）。BOS と EOS の辺には足さない。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NounSuffixRule {
    /// 辺に足す値。0 で規則を止める。既定 6000
    pub penalty: i32,
    /// 覆う名詞の変換用のコストの上限。既定 10000
    pub cover_max_cost: i32,
}

impl Default for NounSuffixRule {
    fn default() -> Self {
        Self {
            penalty: DEFAULT_NOUN_SUFFIX_PENALTY,
            cover_max_cost: DEFAULT_NOUN_SUFFIX_COVER_MAX_COST,
        }
    }
}

impl NounSuffixRule {
    /// `--ranker-config` のキーの接頭辞（設定アプリには出さない）
    pub const CONFIG_PREFIX: &'static str = "lattice.noun_suffix";
    const CONFIG_KEYS: [&'static str; 2] = ["penalty", "cover_max_cost"];

    /// 規則を止めた値（`penalty == 0`）
    pub fn disabled() -> Self {
        Self {
            penalty: 0,
            ..Self::default()
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.penalty != 0
    }

    /// `lattice.noun_suffix.penalty` と `lattice.noun_suffix.cover_max_cost` で `self` を上書きする。
    /// 他の接頭辞のキーは無視し、この接頭辞で未知のキーや負の値、数でない値ならエラー。
    pub fn with_kv(self, kv: &BTreeMap<String, String>) -> Result<Self, RankerConfigError> {
        let scope = ScopedKv::new(Self::CONFIG_PREFIX, &Self::CONFIG_KEYS, kv)?;
        Ok(Self {
            penalty: scope.get("penalty", self.penalty)?,
            cover_max_cost: scope.get("cover_max_cost", self.cover_max_cost)?,
        })
    }
}

/// 変換のラティスの辺の補正: 前の語 → 数詞「数」（ADR 0019 規則 N。B33）。
///
/// 辞書のノード P（品詞2 が 数詞 でない）→ 辞書のノード N（品詞 = 名詞,数詞 で、読みが すう）の辺に、
/// P の開始位置から N の終了位置までをちょうど1語で覆う、品詞1 = 名詞で変換用のコストが `cover_max_cost` 以下の
/// 辞書のノードがあるときだけ `penalty` を足す（変＋数＋名 に対する 変数名）。BOS と EOS の辺には足さない。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NumeralCoverRule {
    /// 辺に足す値。0 で規則を止める。既定 6000
    pub penalty: i32,
    /// 覆う名詞の変換用のコストの上限。既定 10000
    pub cover_max_cost: i32,
}

impl Default for NumeralCoverRule {
    fn default() -> Self {
        Self {
            penalty: DEFAULT_NUMERAL_COVER_PENALTY,
            cover_max_cost: DEFAULT_NUMERAL_COVER_COVER_MAX_COST,
        }
    }
}

impl NumeralCoverRule {
    /// `--ranker-config` のキーの接頭辞（設定アプリには出さない）
    pub const CONFIG_PREFIX: &'static str = "lattice.numeral_cover";
    const CONFIG_KEYS: [&'static str; 2] = ["penalty", "cover_max_cost"];

    /// 規則を止めた値（`penalty == 0`）
    pub fn disabled() -> Self {
        Self {
            penalty: 0,
            ..Self::default()
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.penalty != 0
    }

    /// `lattice.numeral_cover.penalty` と `lattice.numeral_cover.cover_max_cost` で `self` を上書きする。
    /// 他の接頭辞のキーは無視し、この接頭辞で未知のキーや負の値、数でない値ならエラー。
    pub fn with_kv(self, kv: &BTreeMap<String, String>) -> Result<Self, RankerConfigError> {
        let scope = ScopedKv::new(Self::CONFIG_PREFIX, &Self::CONFIG_KEYS, kv)?;
        Ok(Self {
            penalty: scope.get("penalty", self.penalty)?,
            cover_max_cost: scope.get("cover_max_cost", self.cover_max_cost)?,
        })
    }
}

/// 変換のラティスの辺の補正: サ変でない名詞＋する（ADR 0016 規則 S）。
///
/// 辞書のノード N（品詞1 = 名詞で、品詞3 が サ変可能・サ変形状詞可能 でない）→ 辞書のノード V
/// （サ行変格・文語サ行変格の動詞、または 文語下二段-サ行 の助動詞「す」）の辺に `penalty` を足す。
/// BOS と EOS の辺には足さない。文脈の解析には使わない。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NounSuruRule {
    /// 辺に足す値。0 で規則を止める。既定 3000
    pub penalty: i32,
}

impl Default for NounSuruRule {
    fn default() -> Self {
        Self {
            penalty: DEFAULT_NOUN_SURU_PENALTY,
        }
    }
}

impl NounSuruRule {
    /// `--ranker-config` のキーの接頭辞（ADR 0016 §7.1）
    pub const CONFIG_PREFIX: &'static str = "lattice.noun_suru";
    const CONFIG_KEYS: [&'static str; 1] = ["penalty"];

    /// 規則を止めた値（`penalty == 0`）
    pub fn disabled() -> Self {
        Self { penalty: 0 }
    }

    pub fn is_enabled(&self) -> bool {
        self.penalty != 0
    }

    /// `lattice.noun_suru.penalty` で `self` を上書きする。他の接頭辞のキーは無視し、
    /// この接頭辞で未知のキーや負の値、数でない値ならエラー。
    pub fn with_kv(self, kv: &BTreeMap<String, String>) -> Result<Self, RankerConfigError> {
        let scope = ScopedKv::new(Self::CONFIG_PREFIX, &Self::CONFIG_KEYS, kv)?;
        Ok(Self {
            penalty: scope.get("penalty", self.penalty)?,
        })
    }
}

/// 〜としよう／〜にしよう の しよう（ADR 0016 §10。B63）。助詞・副詞・接続詞 → 読み しよう のサ変の意志形、の辺から
/// `bonus` を引き、その しよう → 動詞・名詞・接尾辞 の辺に `bonus` を足す。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VolitionalRule {
    /// 辺から引く値（と、後ろの辺に足す値）。0 で規則を止める。既定 5000
    pub bonus: i32,
    /// サ変可能の名詞 → しよう の辺から引く値（ADR 0016 §12。#230。てすとしよう → テストしよう）。0 でこの辺は補正しない
    pub sahen_bonus: i32,
}

impl Default for VolitionalRule {
    fn default() -> Self {
        Self {
            bonus: DEFAULT_VOLITIONAL_BONUS,
            sahen_bonus: DEFAULT_VOLITIONAL_SAHEN_BONUS,
        }
    }
}

impl VolitionalRule {
    /// `--ranker-config` のキーの接頭辞
    pub const CONFIG_PREFIX: &'static str = "lattice.volitional";
    const CONFIG_KEYS: [&'static str; 2] = ["bonus", "sahen_bonus"];

    pub fn disabled() -> Self {
        Self {
            bonus: 0,
            sahen_bonus: 0,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.bonus != 0
    }

    /// `lattice.volitional.bonus` で `self` を上書きする（`NounSuruRule::with_kv` と同じ扱い）。
    pub fn with_kv(self, kv: &BTreeMap<String, String>) -> Result<Self, RankerConfigError> {
        let scope = ScopedKv::new(Self::CONFIG_PREFIX, &Self::CONFIG_KEYS, kv)?;
        Ok(Self {
            bonus: scope.get("bonus", self.bonus)?,
            sahen_bonus: scope.get("sahen_bonus", self.sahen_bonus)?,
        })
    }
}

/// 数詞のあとの助数詞（ADR 0019 §15。#219）。数詞（名詞,数詞。読みの中の数字の未知語も含む）→ 助数詞可能の名詞
/// （`名詞,普通名詞,助数詞可能`。件・秒・階・杯・倍 など）の辺から `bonus` を引く。
/// 助数詞は語のコストが高く、同音の普通名詞や接尾辞（県・病・湖 など）に負けるのを直す。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NumeralCounterRule {
    /// 辺から引く値。0 で規則を止める。既定 3500
    pub bonus: i32,
    /// 数詞 → 接尾辞,名詞的,助数詞のうち、読みが こ・ねんせい のもの（個・年生。戸・湖・年性に負ける）の辺から引く値。
    /// 助数詞の接尾辞すべてに当てると 冊 が 刷 に、の が 幅 に負ける（ADR 0019 §15.2、§16）ので、読みで絞る。0 でこの種類だけ止まる
    pub suffix_bonus: i32,
    /// 数詞 → 日・カ（読み か の 接尾辞,助数詞）の辺に足す値。「か」を 3日 に取られて、か月・か国・か所 や助詞の か が出ないのを防ぐ。0 で止まる
    pub ka_penalty: i32,
    /// 数詞 → 住所・番号の助数詞の表記（番・番地 など。lattice.rs の `ADDRESS_COUNTER_SURFACES`）の辺から引く値。
    /// 番（8618）が 版（7533）に負けて 6版2号 になるのを直す。0 でこの種類だけ止まる（ADR 0030 §13.5）
    pub address_bonus: i32,
    /// 上と同じ辺のうち、数詞が読みの中の数字でなく、かな読みから作った辞書の数詞（さん → 三）のもの。
    /// 0 でこの種類だけ止まる（ADR 0030 §13.5）
    pub address_kana_bonus: i32,
    /// か国（読み かこく の 助数詞可能）→ 語 の辺から引く値。3かこくご が 3か国後 になる（後が語より安い）のを直す。
    /// 0 でこの種類だけ止まる（ADR 0019 §18。#222）
    pub language_bonus: i32,
    /// 「だい」の直後の数詞 → 序数の助数詞（巻・回・章 など。lattice.rs の `ORDINAL_COUNTER_SURFACES`）の辺から引く値。
    /// 第10かん が 第10缶 になるのを直す。0 でこの種類だけ止まる（#240）
    pub ordinal_bonus: i32,
}

impl Default for NumeralCounterRule {
    fn default() -> Self {
        Self {
            bonus: DEFAULT_NUMERAL_COUNTER_BONUS,
            suffix_bonus: DEFAULT_NUMERAL_COUNTER_SUFFIX_BONUS,
            ka_penalty: DEFAULT_NUMERAL_COUNTER_KA_PENALTY,
            address_bonus: DEFAULT_NUMERAL_COUNTER_ADDRESS_BONUS,
            address_kana_bonus: DEFAULT_NUMERAL_COUNTER_ADDRESS_KANA_BONUS,
            language_bonus: DEFAULT_NUMERAL_COUNTER_LANGUAGE_BONUS,
            ordinal_bonus: DEFAULT_NUMERAL_COUNTER_ORDINAL_BONUS,
        }
    }
}

impl NumeralCounterRule {
    /// `--ranker-config` のキーの接頭辞
    pub const CONFIG_PREFIX: &'static str = "lattice.numeral_counter";
    const CONFIG_KEYS: [&'static str; 7] = [
        "bonus",
        "suffix_bonus",
        "ka_penalty",
        "address_bonus",
        "address_kana_bonus",
        "language_bonus",
        "ordinal_bonus",
    ];

    pub fn disabled() -> Self {
        Self {
            bonus: 0,
            suffix_bonus: 0,
            ka_penalty: 0,
            address_bonus: 0,
            address_kana_bonus: 0,
            language_bonus: 0,
            ordinal_bonus: 0,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.bonus != 0
            || self.suffix_bonus != 0
            || self.ka_penalty != 0
            || self.address_bonus != 0
            || self.address_kana_bonus != 0
            || self.language_bonus != 0
            || self.ordinal_bonus != 0
    }

    /// `lattice.numeral_counter.bonus` で `self` を上書きする（`VolitionalRule::with_kv` と同じ扱い）。
    pub fn with_kv(self, kv: &BTreeMap<String, String>) -> Result<Self, RankerConfigError> {
        let scope = ScopedKv::new(Self::CONFIG_PREFIX, &Self::CONFIG_KEYS, kv)?;
        Ok(Self {
            bonus: scope.get("bonus", self.bonus)?,
            suffix_bonus: scope.get("suffix_bonus", self.suffix_bonus)?,
            ka_penalty: scope.get("ka_penalty", self.ka_penalty)?,
            address_bonus: scope.get("address_bonus", self.address_bonus)?,
            address_kana_bonus: scope.get("address_kana_bonus", self.address_kana_bonus)?,
            language_bonus: scope.get("language_bonus", self.language_bonus)?,
            ordinal_bonus: scope.get("ordinal_bonus", self.ordinal_bonus)?,
        })
    }
}

#[derive(Clone, Debug)]
pub struct ConvertOptions {
    /// 1..=9。既定 9（番号ラベル 1〜9）
    pub page_size: usize,
    /// 最初に生成して Ranker に渡す候補の数。既定 30
    pub pool_size: usize,
    /// 2ページ目以降で追加生成する単位。既定 page_size
    pub chunk_size: usize,
    /// 1回の追加生成で A* が展開する上限。既定 20_000（遅延の保護）
    pub max_astar_pops: usize,
    /// Rewriter の候補の base_cost = 辞書候補の base_cost を昇順に並べた `rewriter_anchor` 番目 + 1。既定 1
    pub rewriter_anchor: usize,
    /// 日付の Rewriter（`RewritePosition::AfterWords`）の候補の base_cost = 辞書候補の base_cost を昇順に並べた
    /// `date_anchor` 番目 + 1。既定 1（`rewriter_anchor` と同じ。#80）。`--ranker-config` の `rewriter.date_anchor`
    pub date_anchor: usize,
    /// 変換1回あたりの LM の待ち時間の規則（`convert` と `page` が締め切りを作るのに使う）
    pub lm_budget: LmBudget,
    /// 副詞の直後の名詞の辺の補正（ADR 0008）
    pub adverb_noun: AdverbNounRule,
    /// サ変でない名詞＋する の辺の補正（ADR 0016 規則 S）
    pub noun_suru: NounSuruRule,
    /// 名詞 → 接尾辞 の辺の補正（ADR 0008 §13。B25）
    pub noun_suffix: NounSuffixRule,
    /// 前の語 → 数詞「数」 の辺の補正（ADR 0019 規則 N。B33）
    pub numeral_cover: NumeralCoverRule,
    /// 助詞＋しよう（意志形）の辺の補正（ADR 0016 §10。B63）
    pub volitional: VolitionalRule,
    /// 数詞 → 助数詞可能の名詞 の辺の補正（ADR 0019 §15。#219）
    pub numeral_counter: NumeralCounterRule,
    /// 未知語のカタカナの候補の補正（#271。既定は無効）
    pub katakana_like: KatakanaLikeRule,
}

impl ConvertOptions {
    /// `rewriter.date_anchor` で `date_anchor` を上書きする。この接頭辞で未知のキーや数でない値ならエラー。
    pub fn with_date_anchor_kv(
        mut self,
        kv: &BTreeMap<String, String>,
    ) -> Result<Self, RankerConfigError> {
        let scope = ScopedKv::new(DATE_ANCHOR_PREFIX, &DATE_ANCHOR_KEYS, kv)?;
        self.date_anchor = scope.get("date_anchor", self.date_anchor)?;
        Ok(self)
    }
}

impl Default for ConvertOptions {
    fn default() -> Self {
        Self {
            page_size: DEFAULT_PAGE_SIZE,
            pool_size: DEFAULT_POOL_SIZE,
            chunk_size: DEFAULT_PAGE_SIZE,
            max_astar_pops: DEFAULT_MAX_ASTAR_POPS,
            rewriter_anchor: DEFAULT_REWRITER_ANCHOR,
            date_anchor: DEFAULT_DATE_ANCHOR,
            lm_budget: LmBudget::default(),
            adverb_noun: AdverbNounRule::default(),
            noun_suru: NounSuruRule::default(),
            noun_suffix: NounSuffixRule::default(),
            numeral_cover: NumeralCoverRule::default(),
            volitional: VolitionalRule::default(),
            numeral_counter: NumeralCounterRule::default(),
            katakana_like: KatakanaLikeRule::default(),
        }
    }
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

    #[test]
    fn adverb_noun_defaults_are_the_approved_values() {
        let r = ConvertOptions::default().adverb_noun;
        assert_eq!(r.penalty, 6000);
        assert_eq!(r.cover_max_cost, 10_000);
        assert!(r.is_enabled());
        assert!(!AdverbNounRule::disabled().is_enabled());
    }

    #[test]
    fn adverb_noun_reads_only_its_own_keys() {
        let base = AdverbNounRule::default();
        let r = base
            .with_kv(&kv(&[
                ("lattice.adverb_noun.penalty", "0"),
                ("lattice.adverb_noun.cover_max_cost", "12000"),
                ("left_context.max_bonus", "1"),
                ("zenz.weight", "x"),
            ]))
            .unwrap();
        assert_eq!(
            r,
            AdverbNounRule {
                penalty: 0,
                cover_max_cost: 12000
            }
        );
        assert_eq!(base.with_kv(&BTreeMap::new()).unwrap(), base);
    }

    #[test]
    fn adverb_noun_rejects_bad_values_and_unknown_keys() {
        let base = AdverbNounRule::default();
        for bad in [
            ("lattice.adverb_noun.penalty", "-1"),
            ("lattice.adverb_noun.penalty", "x"),
            ("lattice.adverb_noun.cap", "1"),
        ] {
            assert!(base.with_kv(&kv(&[bad])).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn noun_suffix_defaults_and_keys() {
        let base = NounSuffixRule::default();
        assert_eq!(base.penalty, 6000);
        assert_eq!(base.cover_max_cost, 10_000);
        assert!(base.is_enabled());
        assert!(!NounSuffixRule::disabled().is_enabled());
        let r = base
            .with_kv(&kv(&[
                ("lattice.noun_suffix.penalty", "0"),
                ("lattice.noun_suffix.cover_max_cost", "9000"),
                ("lattice.adverb_noun.cover_max_cost", "1"),
            ]))
            .unwrap();
        assert_eq!(
            r,
            NounSuffixRule {
                penalty: 0,
                cover_max_cost: 9000
            }
        );
        assert_eq!(base.with_kv(&BTreeMap::new()).unwrap(), base);
        for bad in [
            ("lattice.noun_suffix.penalty", "-1"),
            ("lattice.noun_suffix.penalty", "x"),
            ("lattice.noun_suffix.cap", "1"),
        ] {
            assert!(base.with_kv(&kv(&[bad])).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn numeral_cover_defaults_and_keys() {
        let base = NumeralCoverRule::default();
        assert_eq!(base.penalty, 6000);
        assert_eq!(base.cover_max_cost, 10_000);
        assert!(base.is_enabled());
        assert!(!NumeralCoverRule::disabled().is_enabled());
        let r = base
            .with_kv(&kv(&[
                ("lattice.numeral_cover.penalty", "0"),
                ("lattice.numeral_cover.cover_max_cost", "9000"),
                ("lattice.adverb_noun.cover_max_cost", "1"),
            ]))
            .unwrap();
        assert_eq!(
            r,
            NumeralCoverRule {
                penalty: 0,
                cover_max_cost: 9000
            }
        );
        assert_eq!(base.with_kv(&BTreeMap::new()).unwrap(), base);
        for bad in [
            ("lattice.numeral_cover.penalty", "-1"),
            ("lattice.numeral_cover.penalty", "x"),
            ("lattice.numeral_cover.cap", "1"),
        ] {
            assert!(base.with_kv(&kv(&[bad])).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn volitional_default_is_the_approved_value_and_reads_only_its_own_key() {
        let base = VolitionalRule::default();
        assert_eq!(base.bonus, 5000);
        assert_eq!(base.sahen_bonus, 1500);
        assert!(!VolitionalRule::disabled().is_enabled());
        let r = base
            .with_kv(&kv(&[
                ("lattice.volitional.bonus", "0"),
                ("lattice.volitional.sahen_bonus", "0"),
                ("lattice.noun_suru.penalty", "7"),
            ]))
            .unwrap();
        assert_eq!(r, VolitionalRule::disabled());
        assert_eq!(base.with_kv(&BTreeMap::new()).unwrap(), base);
        for bad in [
            ("lattice.volitional.bonus", "-1"),
            ("lattice.volitional.bonus", "x"),
            ("lattice.volitional.cap", "1"),
        ] {
            assert!(base.with_kv(&kv(&[bad])).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn numeral_counter_default_and_keys() {
        let base = NumeralCounterRule::default();
        assert_eq!(base.bonus, 3500);
        assert_eq!(base.suffix_bonus, 7000);
        assert_eq!(base.ka_penalty, 8000);
        assert_eq!(base.address_bonus, 4000);
        assert_eq!(base.address_kana_bonus, 6000);
        assert_eq!(base.language_bonus, 8000);
        assert_eq!(base.ordinal_bonus, 8000);
        assert!(!NumeralCounterRule::disabled().is_enabled());
        let r = base
            .with_kv(&kv(&[
                ("lattice.numeral_counter.bonus", "0"),
                ("lattice.numeral_counter.suffix_bonus", "0"),
                ("lattice.numeral_counter.ka_penalty", "0"),
                ("lattice.numeral_counter.address_bonus", "0"),
                ("lattice.numeral_counter.address_kana_bonus", "0"),
                ("lattice.numeral_counter.language_bonus", "0"),
                ("lattice.numeral_counter.ordinal_bonus", "0"),
                ("lattice.volitional.bonus", "7"),
            ]))
            .unwrap();
        assert_eq!(r, NumeralCounterRule::disabled());
        // 一部だけ止めても、規則は有効のまま
        let r = base
            .with_kv(&kv(&[("lattice.numeral_counter.bonus", "0")]))
            .unwrap();
        assert!(r.is_enabled());
        assert_eq!(base.with_kv(&BTreeMap::new()).unwrap(), base);
        for bad in [
            ("lattice.numeral_counter.bonus", "-1"),
            ("lattice.numeral_counter.bonus", "x"),
            ("lattice.numeral_counter.suffix_bonus", "-1"),
            ("lattice.numeral_counter.ka_penalty", "x"),
            ("lattice.numeral_counter.cap", "1"),
        ] {
            assert!(base.with_kv(&kv(&[bad])).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn noun_suru_default_is_the_approved_value_and_reads_only_its_own_key() {
        let base = NounSuruRule::default();
        assert_eq!(base.penalty, 3000);
        assert!(base.is_enabled());
        assert!(!NounSuruRule::disabled().is_enabled());
        let r = base
            .with_kv(&kv(&[
                ("lattice.noun_suru.penalty", "0"),
                ("lattice.adverb_noun.penalty", "1"),
            ]))
            .unwrap();
        assert_eq!(r, NounSuruRule::disabled());
        assert_eq!(base.with_kv(&BTreeMap::new()).unwrap(), base);
        for bad in [
            ("lattice.noun_suru.penalty", "-1"),
            ("lattice.noun_suru.penalty", "x"),
            ("lattice.noun_suru.cap", "1"),
        ] {
            assert!(base.with_kv(&kv(&[bad])).is_err(), "{bad:?}");
        }
    }
}
