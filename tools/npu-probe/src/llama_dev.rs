//! llama-cpu / llama-vulkan（今の経路。`ikari_lm::LlamaScorer` の `nll` をそのまま使う。ADR 0075 §4.3）。
//! llama-cpu の子は、OpenVINO の子が使うトークン列も出す（同じ語彙で作る。ADR 0075 §3.1）。

#[cfg(feature = "llama")]
pub use imp::open;

#[cfg(not(feature = "llama"))]
pub fn open(
    _job: &crate::worker::Job,
    _items: &[crate::input::Item],
) -> Result<crate::worker::Opened, String> {
    Err("このビルドに llama.cpp が無い（feature llama / vulkan）".into())
}

/// llama.cpp に登録された装置の要約（`cpu,vulkan:IntegratedGpu` の形。モデルは読まない）
#[cfg(feature = "llama")]
pub fn devices() -> Option<String> {
    ikari_lm::probe_devices().ok()
}

#[cfg(not(feature = "llama"))]
pub fn devices() -> Option<String> {
    None
}

#[cfg(feature = "llama")]
mod imp {
    use std::time::Instant;

    use ikari_lm::{CancelToken, LlamaScorer, LmDevice, LmScorer};

    use crate::device::Device;
    use crate::input::Item;
    use crate::score::{ItemTokens, TokenSet};
    use crate::worker::{ItemScorer, Job, Mode, Opened, gguf_path};

    struct LlamaItems {
        scorer: LlamaScorer,
        items: Vec<(String, Vec<String>)>,
        cancel: CancelToken,
    }

    impl ItemScorer for LlamaItems {
        fn score(&mut self, index: usize) -> Result<Vec<Option<f32>>, String> {
            let (prompt, cands) = self.items.get(index).ok_or("件の番号が範囲の外")?;
            self.scorer
                .nll(prompt, cands, &self.cancel)
                .map(|v| v.into_iter().map(Some).collect())
                .map_err(|e| e.to_string())
        }
    }

    pub fn open(job: &Job, items: &[Item]) -> Result<Opened, String> {
        let gguf = gguf_path(&job.dir).ok_or_else(|| {
            format!(
                "GGUF が無い（{}\\{} に置く）",
                crate::MODELS_DIR,
                crate::GGUF_NAME
            )
        })?;
        let started = Instant::now();
        let scorer = match job.device {
            Device::LlamaVulkan => LlamaScorer::load_with(&gguf, true),
            _ => LlamaScorer::load_cpu_only(&gguf),
        }
        .map_err(|e| e.to_string())?;
        let load = started.elapsed();
        let device = scorer.device();
        let devices = ikari_lm::devices().unwrap_or_default();
        let not_ok = (job.device == Device::LlamaVulkan && device != LmDevice::Gpu)
            .then(|| format!("GPU に載らなかった（CPU に落ちた。devices={devices}）"));
        let (tokens, note) = if job.device == Device::LlamaCpu && job.mode == Mode::Full {
            match token_set(&scorer, items) {
                Ok(t) => (Some(t), None),
                Err(e) => (None, Some(e)),
            }
        } else {
            (None, None)
        };
        Ok(Opened {
            scorer: Box::new(LlamaItems {
                scorer,
                items: items
                    .iter()
                    .map(|i| (i.prompt.clone(), i.candidates.clone()))
                    .collect(),
                cancel: CancelToken::new(),
            }),
            load,
            ran_on: Some(format!(
                "{} ({devices})",
                match device {
                    LmDevice::Gpu => "gpu",
                    LmDevice::Cpu => "cpu",
                    LmDevice::Unknown => "?",
                }
            )),
            compile_ms: Vec::new(),
            too_long: 0,
            tokens,
            note,
            not_ok,
        })
    }

    /// `LlamaScorer::nll` と同じトークン列（BOS はモデルが要るときだけ。zenz は付けない）
    fn token_set(scorer: &LlamaScorer, items: &[Item]) -> Result<TokenSet, String> {
        let info = scorer.info();
        if info.adds_bos {
            return Err("BOS を付けるモデルなので、トークン列を出せない（OpenVINO の行は入力の prompt_ids を使う）".into());
        }
        let ids = |text: &str| -> Vec<i64> {
            scorer
                .debug_tokens(text)
                .into_iter()
                .map(|(id, _)| i64::from(id))
                .collect()
        };
        Ok(TokenSet {
            eos: i64::from(info.end_token),
            items: items
                .iter()
                .map(|item| ItemTokens {
                    prompt: ids(&item.prompt),
                    candidates: item.candidates.iter().map(|c| ids(c)).collect(),
                })
                .collect(),
        })
    }
}
