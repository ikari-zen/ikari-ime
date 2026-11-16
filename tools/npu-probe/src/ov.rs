//! OpenVINO Runtime を直に使う採点（ADR 0075 §3「C」）。`openvino` crate の実行時の読み込み（`runtime-linking`）で、
//! exe の隣の `openvino_c.dll` を読む。ビルドに OpenVINO は要らない。
//!
//! IR の取り決めは README の「契約」（lane/npu-adr）:
//!   - ファイル: `models\zenz-v3.2-small-L128-<fp16|int8>.xml`（B = 1）、`models\zenz-v3.2-small-B32-L128-<…>.xml`（B = 32）
//!   - 入力: `input_ids` だけ。`i64`、静的な形 [B, L]。B と L は IR から読む（決め打ちしない）
//!   - 出力: `logits`。`f32`、形 [B, L, 語彙]。KV キャッシュなし

use std::path::{Path, PathBuf};
use std::time::Instant;

use openvino::{
    CompiledModel, Core, DeviceType, ElementType, InferRequest, PropertyKey, RwPropertyKey, Shape,
    Tensor,
};

use crate::device::{OvTarget, Precision};
use crate::score::{self, Plan, TokenSet};
use crate::worker::{ItemScorer, Job, Opened};

const IR_PREFIX: &str = "zenz-v3.2-small";
const INPUT_NAME: &str = "input_ids";

/// C API の DLL の名前
#[cfg(windows)]
pub const LIB_NAME: &str = "openvino_c.dll";
#[cfg(target_os = "macos")]
pub const LIB_NAME: &str = "libopenvino_c.dylib";
#[cfg(all(unix, not(target_os = "macos")))]
pub const LIB_NAME: &str = "libopenvino_c.so";

/// NPU のドライバの版の属性（NPU のプラグイン）
const NPU_DRIVER_VERSION: &str = "NPU_DRIVER_VERSION";

/// IR のファイルの名前（契約）。B = 1 は `…-L128-…`、ほかは `…-B<B>-L128-…`
pub fn ir_name(precision: Precision, batch: usize) -> String {
    let p = precision.name();
    if batch == 1 {
        format!("{IR_PREFIX}-L128-{p}.xml")
    } else {
        format!("{IR_PREFIX}-B{batch}-L128-{p}.xml")
    }
}

/// 精度と B の IR（xml と bin）
pub fn ir_files(dir: &Path, precision: Precision, batch: usize) -> (PathBuf, PathBuf) {
    let xml = dir.join(crate::MODELS_DIR).join(ir_name(precision, batch));
    let bin = xml.with_extension("bin");
    (xml, bin)
}

/// DLL を読む（1 プロセスで 1 回。2 回目からは何もしない）
pub fn load_lib(ov_lib: Option<&Path>) -> Result<(), String> {
    let path = match ov_lib {
        Some(p) => p.to_path_buf(),
        None => crate::exe_dir()
            .ok_or("exe の場所が分からない")?
            .join(LIB_NAME),
    };
    if !path.is_file() {
        return Err(format!("OpenVINO の DLL が無い（{LIB_NAME}）"));
    }
    openvino_sys::library::load_from(path)
}

fn device_type(target: OvTarget) -> DeviceType<'static> {
    match target {
        OvTarget::Cpu => DeviceType::CPU,
        OvTarget::Gpu => DeviceType::GPU,
        OvTarget::Npu => DeviceType::NPU,
    }
}

/// OpenVINO の版と、見えている装置
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct OvInfo {
    pub version: String,
    /// （装置名、正式名、NPU のドライバの版）
    pub devices: Vec<(String, String, Option<String>)>,
}

pub fn info(ov_lib: Option<&Path>) -> Result<OvInfo, String> {
    load_lib(ov_lib)?;
    let version = openvino::version().build_number;
    let core = Core::new().map_err(|e| e.to_string())?;
    let mut devices = Vec::new();
    for d in core.available_devices().map_err(|e| e.to_string())? {
        let full = core
            .get_property(&d, &PropertyKey::DeviceFullName)
            .unwrap_or_default();
        let driver = d
            .as_ref()
            .starts_with("NPU")
            .then(|| {
                core.get_property(&d, &PropertyKey::Other(NPU_DRIVER_VERSION.into()))
                    .ok()
            })
            .flatten();
        devices.push((d.as_ref().to_owned(), full, driver));
    }
    Ok(OvInfo { version, devices })
}

struct OvItems {
    // 宣言順 = drop 順。要求を先に捨てる
    request: InferRequest,
    input: Tensor,
    _compiled: CompiledModel,
    plans: Vec<Plan>,
    batch: usize,
    seq_len: usize,
}

fn ov_err(what: &str) -> impl Fn(openvino::InferenceError) -> String + '_ {
    move |e| format!("{what}: {e}")
}

impl ItemScorer for OvItems {
    fn score(&mut self, index: usize) -> Result<Vec<Option<f32>>, String> {
        let plan = self.plans.get(index).ok_or("件の番号が範囲の外")?;
        let mut out = vec![None; plan.rows.len() + plan.too_long.len()];
        let (b_n, t) = (self.batch, self.seq_len);
        for chunk in &plan.chunks {
            let ids = score::fill(plan, chunk, b_n, t);
            self.input
                .get_data_mut::<i64>()
                .map_err(ov_err("input"))?
                .copy_from_slice(&ids);
            self.request.infer().map_err(ov_err("infer"))?;
            let logits = self
                .request
                .get_output_tensor_by_index(0)
                .map_err(ov_err("output"))?;
            let shape = logits.get_shape().map_err(ov_err("output shape"))?;
            let dims = shape.get_dimensions();
            if dims.len() != 3 || dims[0] as usize != b_n || dims[1] as usize != t {
                return Err(format!(
                    "出力の形が [B={b_n}, L={t}, 語彙] でない: {dims:?}"
                ));
            }
            let vocab = dims[2] as usize;
            let et = logits.get_element_type().map_err(ov_err("output type"))?;
            if et != ElementType::F32 {
                return Err(format!("出力の型が f32 でない: {et:?}"));
            }
            let values = logits.get_data::<f32>().map_err(ov_err("output"))?;
            // 余りの行は読まない（契約）
            for (b, &r) in chunk.rows.iter().enumerate() {
                let row = &plan.rows[r];
                let slice = values
                    .get(b * t * vocab..(b + 1) * t * vocab)
                    .ok_or("出力が短い")?;
                out[row.candidate] =
                    Some(score::row_nll(slice, vocab, row)).filter(|x| x.is_finite());
            }
        }
        Ok(out)
    }
}

/// IR を読み、形を確かめてコンパイルする
pub fn open(
    job: &Job,
    target: OvTarget,
    precision: Precision,
    tokens: &TokenSet,
) -> Result<Opened, String> {
    let want_batch = job.batch.unwrap_or(1);
    let (xml, bin) = ir_files(&job.dir, precision, want_batch);
    if !xml.is_file() {
        return Err(format!(
            "IR が無い（{}\\{}）",
            crate::MODELS_DIR,
            ir_name(precision, want_batch)
        ));
    }
    load_lib(job.ov_lib.as_deref())?;
    let started = Instant::now();
    let mut core = Core::new().map_err(|e| format!("Core: {e}"))?;
    let dev = device_type(target);
    let available: Vec<String> = core
        .available_devices()
        .map_err(ov_err("devices"))?
        .iter()
        .map(|d| d.as_ref().to_owned())
        .collect();
    if !available.iter().any(|d| d.starts_with(target.ov_name())) {
        return Err(format!(
            "装置が見えない（OpenVINO の一覧: {}）",
            available.join(", ")
        ));
    }
    let full_name = core
        .get_property(&dev, &PropertyKey::DeviceFullName)
        .unwrap_or_default();
    // `ov_core_set_property` は C の可変長引数の関数で、`openvino` crate は可変長でない形で呼ぶ。Windows（x64）は
    // 呼び方が同じなので効くが、arm64 の macOS では属性の名前が空で届く（Mac で確かめた）。キャッシュは Windows だけ
    let mut note = None;
    if let Some(cache) = &job.cache {
        if cfg!(windows) {
            let s = cache.to_str().ok_or("キャッシュの場所が UTF-8 でない")?;
            core.set_property(&dev, &RwPropertyKey::CacheDir, s)
                .map_err(ov_err("CACHE_DIR"))?;
        } else {
            note = Some("キャッシュ（CACHE_DIR）は Windows だけ".to_owned());
        }
    }

    let path = |p: &Path| {
        p.to_str()
            .map(str::to_owned)
            .ok_or("IR の場所が UTF-8 でない")
    };
    let model = core
        .read_model_from_file(&path(&xml)?, &path(&bin)?)
        .map_err(ov_err("read_model"))?;
    let n_inputs = model.get_inputs_len().map_err(ov_err("inputs"))?;
    let node = model.get_input_by_index(0).map_err(ov_err("input"))?;
    let name = node.get_name().map_err(ov_err("input name"))?;
    let et = node.get_element_type().map_err(ov_err("input type"))?;
    if n_inputs != 1 || name != INPUT_NAME || et != ElementType::I64 {
        return Err(format!(
            "入力が契約（{INPUT_NAME} だけ、i64）と違う: {n_inputs} 個、{name}、{et:?}"
        ));
    }
    let dims = node
        .get_shape()
        .map_err(ov_err("input shape（静的な形が要る）"))?
        .get_dimensions()
        .to_vec();
    let [b, l] = dims[..] else {
        return Err(format!("入力の形が [B, L] でない: {dims:?}"));
    };
    let (batch, seq_len) = (b as usize, l as usize);
    if batch != want_batch {
        return Err(format!(
            "IR の B（{batch}）がファイル名（{want_batch}）と違う"
        ));
    }
    let compile_started = Instant::now();
    let mut compiled = core
        .compile_model(&model, dev.to_owned())
        .map_err(ov_err(&format!("compile [B={batch}, L={seq_len}]")))?;
    let mut request = compiled
        .create_infer_request()
        .map_err(ov_err("infer request"))?;
    let compile_ms = vec![(seq_len, compile_started.elapsed().as_secs_f64() * 1000.0)];
    let input = Tensor::new(
        ElementType::I64,
        &Shape::new(&dims).map_err(ov_err("shape"))?,
    )
    .map_err(ov_err("tensor"))?;
    request
        .set_tensor(INPUT_NAME, &input)
        .map_err(ov_err("set_tensor"))?;
    let load = started.elapsed();

    let plans: Vec<Plan> = tokens
        .items
        .iter()
        .map(|it| score::plan(it, tokens.eos, batch, seq_len))
        .collect();
    let too_long = plans.iter().map(|p| p.too_long.len()).sum();
    Ok(Opened {
        scorer: Box::new(OvItems {
            request,
            input,
            _compiled: compiled,
            plans,
            batch,
            seq_len,
        }),
        load,
        ran_on: Some(full_name),
        compile_ms,
        too_long,
        tokens: None,
        note,
        not_ok: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ir_names_follow_the_contract() {
        assert_eq!(ir_name(Precision::Fp16, 1), "zenz-v3.2-small-L128-fp16.xml");
        assert_eq!(
            ir_name(Precision::Int8, 32),
            "zenz-v3.2-small-B32-L128-int8.xml"
        );
        let (xml, bin) = ir_files(Path::new("d"), Precision::Int8, 1);
        assert_eq!(xml, Path::new("d/models/zenz-v3.2-small-L128-int8.xml"));
        assert_eq!(bin, Path::new("d/models/zenz-v3.2-small-L128-int8.bin"));
    }

    #[test]
    fn missing_lib_is_an_error() {
        let err = load_lib(Some(Path::new("/nonexistent/openvino_c"))).unwrap_err();
        assert!(err.contains(LIB_NAME), "{err}");
    }
}
