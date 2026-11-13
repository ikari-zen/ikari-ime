//! LM の組み立て。feature `lm` の有無で実装が2つある（なければ `LmNotBuilt`）。

use super::*;

#[cfg(feature = "lm")]
use ikari_lm::{LlamaScorer, ZenzConfig, ZenzRanker};

#[cfg(feature = "lm")]
pub fn build(path: &Path, kv: &BTreeMap<String, String>) -> Result<LmParts, StackError> {
    if !path.exists() {
        return Err(StackError::LmModelNotFound(path.to_path_buf()));
    }
    let cfg = ZenzConfig::from_kv(kv).map_err(|e| config_err("zenz", e))?;
    let ranker = Arc::new(ZenzRanker::new(LlamaScorer::loader(path), cfg));
    let handle = Arc::new(ZenzHandle {
        ranker: Arc::clone(&ranker),
    });
    Ok((ranker, handle))
}

#[cfg(feature = "lm")]
struct ZenzHandle {
    ranker: Arc<ZenzRanker>,
}

#[cfg(feature = "lm")]
impl LmHandle for ZenzHandle {
    /// `ZenzRanker::warm_up`（ダミーの短い採点を1回。読み込みと Metal の初期化を済ませる）
    fn warm_up(&self) {
        self.ranker.warm_up();
    }

    fn stats(&self) -> LmStats {
        let s = self.ranker.stats();
        LmStats {
            calls: s.calls,
            timeouts: s.timeouts,
            late_hits: s.late_hits,
            loads: s.loads,
            load_failures: s.load_failures,
        }
    }

    fn is_loaded(&self) -> bool {
        self.ranker.stats().loaded
    }

    fn timings(&self) -> LmTimings {
        let t = self.ranker.timings();
        LmTimings {
            inference: t.inference,
            loads: t.loads,
        }
    }

    fn devices(&self) -> Option<String> {
        ikari_lm::devices()
    }
}

#[cfg(not(feature = "lm"))]
pub fn build(_path: &Path, _kv: &BTreeMap<String, String>) -> Result<LmParts, StackError> {
    Err(StackError::LmNotBuilt)
}
