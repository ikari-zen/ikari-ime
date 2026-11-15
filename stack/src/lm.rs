//! LM の組み立て。推論の場所は 2 つ: 別プロセス（`ikari-lm-host`。ADR 0060 Q4。製品のエンジン）と、
//! 同じプロセス（feature `lm` のときだけ。なければ `LmNotBuilt`。CLI、評価、テスト）。

use super::*;

use ikari_lm::host::{HostHealth, HostSpec, host_loader};
use ikari_lm::{LmLoader, ZenzConfig, ZenzRanker};

/// `host` が Some なら、その実行ファイルを子として起動して採点を頼む（このビルドに llama.cpp は要らない）。
/// None なら同じプロセスで動かす。
pub fn build(
    path: &Path,
    kv: &BTreeMap<String, String>,
    host: Option<&Path>,
) -> Result<LmParts, StackError> {
    if host.is_none() && !cfg!(feature = "lm") {
        return Err(StackError::LmNotBuilt);
    }
    if !path.exists() {
        return Err(StackError::LmModelNotFound(path.to_path_buf()));
    }
    let cfg = ZenzConfig::from_kv(kv).map_err(|e| config_err("zenz", e))?;
    let (loader, host) = match host {
        Some(program) => {
            let (loader, health) = host_loader(HostSpec {
                program: program.to_path_buf(),
                model: path.to_path_buf(),
                use_gpu: cfg.gpu,
                limits: cfg.host.clone(),
            });
            (loader, Some(health))
        }
        None => (in_process_loader(path, cfg.gpu)?, None),
    };
    let ranker = Arc::new(ZenzRanker::new(loader, cfg));
    let handle = Arc::new(ZenzHandle {
        ranker: Arc::clone(&ranker),
        host,
    });
    Ok((ranker, handle))
}

#[cfg(feature = "lm")]
fn in_process_loader(path: &Path, gpu: bool) -> Result<LmLoader, StackError> {
    Ok(ikari_lm::LlamaScorer::loader_with(path, gpu))
}

#[cfg(not(feature = "lm"))]
fn in_process_loader(_path: &Path, _gpu: bool) -> Result<LmLoader, StackError> {
    Err(StackError::LmNotBuilt)
}

struct ZenzHandle {
    ranker: Arc<ZenzRanker>,
    /// 別プロセスで動かしているときの、子の様子
    host: Option<Arc<HostHealth>>,
}

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

    fn device(&self) -> LmDevice {
        self.ranker.device()
    }

    /// GPU の扱いとデバイスの一覧。別プロセスのときは、子の様子（起動の回数、連続失敗、断念）を足す
    fn load_note(&self) -> Option<String> {
        match (ikari_lm::log_note(), &self.host) {
            (Some(note), Some(host)) => Some(format!("{note} {}", host.log_note())),
            (None, Some(host)) => Some(host.log_note()),
            (note, None) => note,
        }
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
