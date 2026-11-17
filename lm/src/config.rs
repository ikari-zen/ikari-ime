use std::collections::BTreeMap;
use std::time::Duration;

use crate::error::LmError;
use crate::host::HostLimits;
use crate::profile::{ModelProfile, PromptFormat, ZENZ_V3_2_SMALL};

/// `--ranker-config` で上書きするときのキーの接頭辞（他の Ranker と同じファイルに書けるように）
pub const CONFIG_PREFIX: &str = "zenz.";

/// 待ちの上限。実際の待ちは変換1回の締め切り（ADR 0004 §3.4。30ms〜300ms）が決める
const DEFAULT_BUDGET_MS: u64 = 300;
const DEFAULT_IDLE_UNLOAD_SECS: u64 = 300; // NFR-11
const DEFAULT_MAX_TOTAL: i32 = 6000;
const DEFAULT_TOPIC_WORDS: usize = 3;
const DEFAULT_MAX_CANDIDATES: usize = 32;
const DEFAULT_LOAD_RETRY_SECS: u64 = 10;
/// GPU を使う既定（2026-11-14 ユーザー決定。使えない機械では自動で CPU）
const DEFAULT_GPU: bool = true;
/// NLL（nat）の差 1 あたりのコスト（ADR 0002 §13.2 の `× 1000`）
pub const COST_PER_NAT: f32 = 1000.0;

#[derive(Clone, Debug, PartialEq)]
pub struct ZenzConfig {
    /// プロンプトの形式（profile から。既定は zenz v3。`zenz.model_kind` で上書きできる）
    pub format: PromptFormat,
    /// `rank()` が結果を待つ時間の上限。実際は `min(budget, 締め切りまでの残り)`（NFR-04）
    pub budget: Duration,
    /// 最後の要求からこの時間が経ったらモデルを解放する（NFR-11）
    pub idle_unload: Duration,
    /// モデルを GPU に載せているときの解放までの時間。None なら `idle_unload` と同じ。GPU は読み直しが重い
    /// （N150 の VM で約 18 秒。ADR 0059）ので、エンジンが Vulkan のビルドで長くする
    pub idle_unload_gpu: Option<Duration>,
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
    /// GPU があれば使う（設定 `engine.lm_gpu`。既定は真。ADR 0059 §2.5）。GPU が使えなければ CPU。
    /// Vulkan のビルドだけで効く（Metal は常に GPU、CPU だけのビルドは常に CPU）
    pub gpu: bool,
    /// 推論を別プロセス（`ikari-lm-host`）で動かすときの締め切りと断念の回数（ADR 0060 Q4）。
    /// 同じプロセスで動かすときは使わない
    pub host: HostLimits,
}

impl Default for ZenzConfig {
    /// zenz-v3.2 small の profile から作る（今までの既定と同じ値）
    fn default() -> Self {
        Self::for_profile(&ZENZ_V3_2_SMALL)
    }
}

impl ZenzConfig {
    /// profile の値（形式、`alpha`、文脈の文字数）から作る。ほかの値はモデルに依らない既定（ADR 0060 §3）
    pub fn for_profile(profile: &ModelProfile) -> Self {
        Self {
            format: profile.format,
            budget: Duration::from_millis(DEFAULT_BUDGET_MS),
            idle_unload: Duration::from_secs(DEFAULT_IDLE_UNLOAD_SECS),
            idle_unload_gpu: None,
            alpha: profile.alpha,
            max_total: DEFAULT_MAX_TOTAL,
            context_chars: profile.limits.context_chars,
            right_context_chars: profile.limits.right_context_chars,
            topic_words: DEFAULT_TOPIC_WORDS,
            max_candidates: DEFAULT_MAX_CANDIDATES,
            load_retry: Duration::from_secs(DEFAULT_LOAD_RETRY_SECS),
            gpu: DEFAULT_GPU,
            host: HostLimits::default(),
        }
    }
}

impl ZenzConfig {
    /// `zenz.` で始まるキーだけを読む（ほかのキーは他の Ranker のものとして無視する）。
    /// 未知の `zenz.*` キーと、読めない値はエラー。
    ///
    /// キー: `zenz.alpha`、`zenz.max_total`、`zenz.budget_ms`、`zenz.idle_unload_secs`、
    /// `zenz.context_chars`、`zenz.right_context_chars`、`zenz.topic_words`、`zenz.max_candidates`、`zenz.load_retry_secs`、`zenz.gpu`（`on` / `off`）、
    /// `zenz.host_ready_ms`、`zenz.host_request_ms`、`zenz.host_max_failures`（別プロセスのとき。掃引用）
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
                    cfg.format = PromptFormat::parse(value)
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
                "idle_unload_gpu_secs" => {
                    cfg.idle_unload_gpu = Some(Duration::from_secs(
                        parse_u64(value).ok_or_else(|| bad("not an integer"))?,
                    ));
                }
                "load_retry_secs" => {
                    cfg.load_retry =
                        Duration::from_secs(parse_u64(value).ok_or_else(|| bad("not an integer"))?);
                }
                "gpu" => {
                    cfg.gpu = match value.trim() {
                        "on" => true,
                        "off" => false,
                        _ => return Err(bad("expected `on` or `off`")),
                    };
                }
                "host_ready_ms" => {
                    cfg.host.ready_deadline = Duration::from_millis(
                        parse_u64(value).ok_or_else(|| bad("not an integer"))?,
                    );
                }
                "host_request_ms" => {
                    cfg.host.request_deadline = Duration::from_millis(
                        parse_u64(value).ok_or_else(|| bad("not an integer"))?,
                    );
                }
                "host_max_failures" => {
                    let v: u32 = value.trim().parse().map_err(|_| bad("not an integer"))?;
                    if v == 0 {
                        return Err(bad("must be at least 1"));
                    }
                    cfg.host.max_failures = v;
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
    fn idle_unload_gpu_is_unset_by_default_and_read_from_kv() {
        assert_eq!(ZenzConfig::default().idle_unload_gpu, None);
        let c = ZenzConfig::from_kv(&kv(&[("zenz.idle_unload_gpu_secs", "1800")])).unwrap();
        assert_eq!(c.idle_unload_gpu, Some(Duration::from_secs(1800)));
    }

    #[test]
    fn gpu_defaults_on_and_reads_on_off() {
        assert!(ZenzConfig::default().gpu);
        assert!(
            !ZenzConfig::from_kv(&kv(&[("zenz.gpu", "off")]))
                .unwrap()
                .gpu
        );
        assert!(ZenzConfig::from_kv(&kv(&[("zenz.gpu", "on")])).unwrap().gpu);
        assert!(ZenzConfig::from_kv(&kv(&[("zenz.gpu", "maybe")])).is_err());
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
        assert_eq!(ZenzConfig::default().format, PromptFormat::ZenzV3);
        let c = ZenzConfig::from_kv(&kv(&[("zenz.model_kind", "kanakanji-edge")])).unwrap();
        assert_eq!(c.format, PromptFormat::KanakanjiEdge);
        let c = ZenzConfig::from_kv(&kv(&[("zenz.model_kind", "zenz-v3")])).unwrap();
        assert_eq!(c.format, PromptFormat::ZenzV3);
        assert!(ZenzConfig::from_kv(&kv(&[("zenz.model_kind", "gpt")])).is_err());
    }

    #[test]
    fn host_limits_are_read_from_kv() {
        let c = ZenzConfig::from_kv(&kv(&[
            ("zenz.host_ready_ms", "1500"),
            ("zenz.host_request_ms", "250"),
            ("zenz.host_max_failures", "5"),
        ]))
        .unwrap();
        assert_eq!(c.host.ready_deadline, Duration::from_millis(1500));
        assert_eq!(c.host.request_deadline, Duration::from_millis(250));
        assert_eq!(c.host.max_failures, 5);
        assert_eq!(ZenzConfig::default().host, HostLimits::default());
        assert!(ZenzConfig::from_kv(&kv(&[("zenz.host_max_failures", "0")])).is_err());
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
