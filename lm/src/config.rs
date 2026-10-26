use std::collections::BTreeMap;
use std::time::Duration;

use crate::error::LmError;

/// `--ranker-config` で上書きするときのキーの接頭辞（他の Ranker と同じファイルに書けるように）
pub const CONFIG_PREFIX: &str = "zenz.";

/// 待ちの上限。実際の待ちは変換1回の締め切り（ADR 0004 §3.4。30ms〜300ms）が決める
const DEFAULT_BUDGET_MS: u64 = 300;
const DEFAULT_IDLE_UNLOAD_SECS: u64 = 300; // NFR-11
const DEFAULT_ALPHA: f32 = 1.0;
const DEFAULT_MAX_TOTAL: i32 = 6000;
const DEFAULT_CONTEXT_CHARS: usize = 40;
/// 右文脈の先頭から渡す字数。測定で 10 字を採用（0 字より悪化 0 件。ADR 0015 付録 C。#270）
const DEFAULT_RIGHT_CONTEXT_CHARS: usize = 10;
const DEFAULT_TOPIC_WORDS: usize = 3;
const DEFAULT_MAX_CANDIDATES: usize = 32;
const DEFAULT_LOAD_RETRY_SECS: u64 = 10;
/// NLL（nat）の差 1 あたりのコスト（ADR 0002 §13.2 の `× 1000`）
pub const COST_PER_NAT: f32 = 1000.0;

/// モデルの種類。プロンプトの形式が違う。採点の仕組み（プロンプトの続きとして候補を出す確率）は同じ。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ModelKind {
    /// zenz v3: `[EE03 プロフィール][EE04 話題] [EE02 左文脈] EE00 読み EE01`。条件は空なら省く
    #[default]
    ZenzV3,
    /// kanakanji-edge: `EE02 左文脈 EE00 読み EE01`。EE02 は文脈が空でも置く。プロフィールと話題は学習していない
    KanakanjiEdge,
}

impl ModelKind {
    pub const ZENZ_V3: &'static str = "zenz-v3";
    pub const KANAKANJI_EDGE: &'static str = "kanakanji-edge";

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            Self::ZENZ_V3 => Some(Self::ZenzV3),
            Self::KANAKANJI_EDGE => Some(Self::KanakanjiEdge),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ZenzV3 => Self::ZENZ_V3,
            Self::KanakanjiEdge => Self::KANAKANJI_EDGE,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ZenzConfig {
    /// モデルの種類（プロンプトの形式）。既定は zenz v3
    pub model_kind: ModelKind,
    /// `rank()` が結果を待つ時間の上限。実際は `min(budget, 締め切りまでの残り)`（NFR-04）
    pub budget: Duration,
    /// 最後の要求からこの時間が経ったらモデルを解放する（NFR-11）
    pub idle_unload: Duration,
    /// 加点は `alpha * (NLL - min NLL) * 1000`（dev 分割で調整する）
    pub alpha: f32,
    /// 1候補あたりの加算の上限（コスト）
    pub max_total: i32,
    /// プロンプトに入れる左文脈の末尾の文字数
    pub context_chars: usize,
    /// プロンプトに入れる右文脈の先頭の文字数（0 = 渡さない。#270）
    pub right_context_chars: usize,
    /// プロンプトの話題に入れる内容語の数
    pub topic_words: usize,
    /// 1回の要求で採点する候補の数の上限（プールの先頭から）
    pub max_candidates: usize,
    /// モデルの読み込みに失敗したあと、次に試すまでの時間
    pub load_retry: Duration,
}

impl Default for ZenzConfig {
    fn default() -> Self {
        Self {
            model_kind: ModelKind::default(),
            budget: Duration::from_millis(DEFAULT_BUDGET_MS),
            idle_unload: Duration::from_secs(DEFAULT_IDLE_UNLOAD_SECS),
            alpha: DEFAULT_ALPHA,
            max_total: DEFAULT_MAX_TOTAL,
            context_chars: DEFAULT_CONTEXT_CHARS,
            right_context_chars: DEFAULT_RIGHT_CONTEXT_CHARS,
            topic_words: DEFAULT_TOPIC_WORDS,
            max_candidates: DEFAULT_MAX_CANDIDATES,
            load_retry: Duration::from_secs(DEFAULT_LOAD_RETRY_SECS),
        }
    }
}

impl ZenzConfig {
    /// `zenz.` で始まるキーだけを読む（ほかのキーは他の Ranker のものとして無視する）。
    /// 未知の `zenz.*` キーと、読めない値はエラー。
    ///
    /// キー: `zenz.alpha`、`zenz.max_total`、`zenz.budget_ms`、`zenz.idle_unload_secs`、
    /// `zenz.context_chars`、`zenz.right_context_chars`、`zenz.topic_words`、`zenz.max_candidates`、`zenz.load_retry_secs`
    pub fn from_kv(kv: &BTreeMap<String, String>) -> Result<Self, LmError> {
        let mut cfg = Self::default();
        for (key, value) in kv {
            let Some(name) = key.strip_prefix(CONFIG_PREFIX) else {
                continue;
            };
            let bad = |message: &str| LmError::Config {
                key: key.clone(),
                message: format!("{message}: `{value}`"),
            };
            match name {
                "model_kind" => {
                    cfg.model_kind = ModelKind::parse(value)
                        .ok_or_else(|| bad("expected `zenz-v3` or `kanakanji-edge`"))?;
                }
                "alpha" => {
                    let v: f32 = value.trim().parse().map_err(|_| bad("not a number"))?;
                    if !v.is_finite() || v < 0.0 {
                        return Err(bad("must be a non-negative finite number"));
                    }
                    cfg.alpha = v;
                }
                "max_total" => {
                    let v: i32 = value.trim().parse().map_err(|_| bad("not an integer"))?;
                    if v < 0 {
                        return Err(bad("must be non-negative"));
                    }
                    cfg.max_total = v;
                }
                "budget_ms" => {
                    cfg.budget = Duration::from_millis(
                        parse_u64(value).ok_or_else(|| bad("not an integer"))?,
                    );
                }
                "idle_unload_secs" => {
                    cfg.idle_unload =
                        Duration::from_secs(parse_u64(value).ok_or_else(|| bad("not an integer"))?);
                }
                "load_retry_secs" => {
                    cfg.load_retry =
                        Duration::from_secs(parse_u64(value).ok_or_else(|| bad("not an integer"))?);
                }
                "context_chars" => {
                    cfg.context_chars = parse_usize(value).ok_or_else(|| bad("not an integer"))?;
                }
                "right_context_chars" => {
                    cfg.right_context_chars =
                        parse_usize(value).ok_or_else(|| bad("not an integer"))?;
                }
                "topic_words" => {
                    cfg.topic_words = parse_usize(value).ok_or_else(|| bad("not an integer"))?;
                }
                "max_candidates" => {
                    let v = parse_usize(value).ok_or_else(|| bad("not an integer"))?;
                    if v == 0 {
                        return Err(bad("must be at least 1"));
                    }
                    cfg.max_candidates = v;
                }
                _ => {
                    return Err(LmError::Config {
                        key: key.clone(),
                        message: "unknown key".to_owned(),
                    });
                }
            }
        }
        Ok(cfg)
    }
}

fn parse_u64(s: &str) -> Option<u64> {
    s.trim().parse().ok()
}

fn parse_usize(s: &str) -> Option<usize> {
    s.trim().parse().ok()
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
    fn defaults_match_the_adr() {
        let c = ZenzConfig::default();
        assert_eq!(c.budget, Duration::from_millis(300));
        assert_eq!(c.idle_unload, Duration::from_secs(300));
        assert_eq!(c.context_chars, 40);
        assert_eq!(c.right_context_chars, 10);
        assert_eq!(c.topic_words, 3);
    }

    #[test]
    fn from_kv_overrides_and_ignores_other_prefixes() {
        let c = ZenzConfig::from_kv(&kv(&[
            ("zenz.alpha", "0.5"),
            ("zenz.budget_ms", "50"),
            ("zenz.idle_unload_secs", "10"),
            ("topic.weight", "123"),
            ("surface_bonus", "9"),
        ]))
        .unwrap();
        assert_eq!(c.alpha, 0.5);
        assert_eq!(c.budget, Duration::from_millis(50));
        assert_eq!(c.idle_unload, Duration::from_secs(10));
        assert_eq!(c.max_total, ZenzConfig::default().max_total);
    }

    #[test]
    fn model_kind_defaults_to_zenz_and_parses_both_names() {
        assert_eq!(ZenzConfig::default().model_kind, ModelKind::ZenzV3);
        let c = ZenzConfig::from_kv(&kv(&[("zenz.model_kind", "kanakanji-edge")])).unwrap();
        assert_eq!(c.model_kind, ModelKind::KanakanjiEdge);
        let c = ZenzConfig::from_kv(&kv(&[("zenz.model_kind", "zenz-v3")])).unwrap();
        assert_eq!(c.model_kind, ModelKind::ZenzV3);
        assert!(ZenzConfig::from_kv(&kv(&[("zenz.model_kind", "gpt")])).is_err());
        for k in [ModelKind::ZenzV3, ModelKind::KanakanjiEdge] {
            assert_eq!(ModelKind::parse(k.as_str()), Some(k));
        }
    }

    #[test]
    fn from_kv_rejects_bad_values() {
        for (k, v) in [
            ("zenz.alpha", "x"),
            ("zenz.alpha", "-1"),
            ("zenz.alpha", "inf"),
            ("zenz.alpha", "NaN"),
            ("zenz.max_total", "-5"),
            ("zenz.budget_ms", "-1"),
            ("zenz.max_candidates", "0"),
            ("zenz.nonexistent", "1"),
        ] {
            assert!(ZenzConfig::from_kv(&kv(&[(k, v)])).is_err(), "{k}={v}");
        }
    }
}
