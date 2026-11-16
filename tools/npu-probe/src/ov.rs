//! OpenVINO Runtime を直に使う採点（ADR 0075 §3「C」）。`openvino` crate の実行時の読み込み（`runtime-linking`）で、
//! exe の隣の `openvino_c.dll` を読む。ビルドに OpenVINO は要らない。
//!
//! IR の取り決め（**仮置き**。lane/npu-adr の契約が出たら合わせる。README の「契約」）:
//!   - 置き場所: `models\ir\<fp16|int8>\openvino_model.xml` と `.bin`。形ごとの IR
//!     （`openvino_model_b<B>_t<T>.xml`）があれば、そちらを使う
//!   - 入力: `input_ids`（必須）、`attention_mask`、`position_ids`（あれば）。i64 か i32。形 [B, T]
//!   - 出力: 最初の出力が logits。f32 か f16。形 [B, T, 語彙]
//!   - KV キャッシュなし（stateless）。動的な形の IR は、ここで [B, T] に固定してからコンパイルする

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use openvino::{
    CompiledModel, Core, DeviceType, ElementType, InferRequest, PartialShape, PropertyKey,
    RwPropertyKey, Shape, Tensor,
};

use crate::device::{OvTarget, Precision};
use crate::score::{self, Plan, SEQ_LENS, TokenSet};
use crate::worker::{ItemScorer, Job, Opened};

/// IR の置き場所（`models\` の下）
pub const IR_DIR: &str = "ir";
const IR_STEM: &str = "openvino_model";

/// C API の DLL の名前
#[cfg(windows)]
pub const LIB_NAME: &str = "openvino_c.dll";
#[cfg(target_os = "macos")]
pub const LIB_NAME: &str = "libopenvino_c.dylib";
#[cfg(all(unix, not(target_os = "macos")))]
pub const LIB_NAME: &str = "libopenvino_c.so";

/// NPU のドライバの版の属性（NPU のプラグイン）
const NPU_DRIVER_VERSION: &str = "NPU_DRIVER_VERSION";

/// 精度ごとの IR のフォルダ
pub fn ir_dir(dir: &Path, precision: Precision) -> PathBuf {
    dir.join(crate::MODELS_DIR)
        .join(IR_DIR)
        .join(precision.name())
}

/// 形 [B, T] に使う IR（xml と bin）。形ごとの IR があればそれ、無ければ共通の IR
pub fn ir_files(
    dir: &Path,
    precision: Precision,
    batch: usize,
    seq_len: usize,
) -> (PathBuf, PathBuf) {
    let base = ir_dir(dir, precision);
    let shaped = base.join(format!("{IR_STEM}_b{batch}_t{seq_len}.xml"));
    let xml = if shaped.is_file() {
        shaped
    } else {
        base.join(format!("{IR_STEM}.xml"))
    };
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

/// 何を詰めるかで決まる入力の種類
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InputKind {
    Ids,
    Mask,
    Positions,
}

fn input_kind(name: &str) -> Option<InputKind> {
    match name {
        "input_ids" => Some(InputKind::Ids),
        "attention_mask" => Some(InputKind::Mask),
        "position_ids" => Some(InputKind::Positions),
        _ => None,
    }
}

/// 長さ T ごとのコンパイル済みのモデルと、使い回す入力のテンソル
struct Bucket {
    seq_len: usize,
    // 宣言順 = drop 順。要求を先に捨てる
    request: InferRequest,
    inputs: Vec<(InputKind, ElementType, Tensor)>,
    _compiled: CompiledModel,
}

struct OvItems {
    buckets: Vec<Bucket>,
    plans: Vec<Plan>,
    batch: usize,
}

fn ov_err(what: &str) -> impl Fn(openvino::InferenceError) -> String + '_ {
    move |e| format!("{what}: {e}")
}

fn write_ints(tensor: &mut Tensor, et: ElementType, values: &[i64]) -> Result<(), String> {
    match et {
        ElementType::I64 => {
            let dst = tensor.get_data_mut::<i64>().map_err(ov_err("tensor"))?;
            dst.copy_from_slice(values);
        }
        ElementType::I32 => {
            let dst = tensor.get_data_mut::<i32>().map_err(ov_err("tensor"))?;
            for (d, &v) in dst.iter_mut().zip(values) {
                *d = i32::try_from(v).map_err(|_| "トークンの id が i32 に入らない")?;
            }
        }
        other => return Err(format!("入力の型が整数でない: {other:?}")),
    }
    Ok(())
}

impl ItemScorer for OvItems {
    fn score(&mut self, index: usize) -> Result<Vec<Option<f32>>, String> {
        let plan = self.plans.get(index).ok_or("件の番号が範囲の外")?;
        let n_cands = plan.rows.len() + plan.too_long.len();
        let mut out = vec![None; n_cands];
        for chunk in &plan.chunks {
            let bucket = self
                .buckets
                .iter_mut()
                .find(|b| b.seq_len == chunk.seq_len)
                .ok_or("長さの合うモデルが無い")?;
            let filled = score::fill(plan, chunk, self.batch);
            for (kind, et, tensor) in &mut bucket.inputs {
                let values = match kind {
                    InputKind::Ids => &filled.input_ids,
                    InputKind::Mask => &filled.attention_mask,
                    InputKind::Positions => &filled.position_ids,
                };
                write_ints(tensor, *et, values)?;
            }
            bucket.request.infer().map_err(ov_err("infer"))?;
            let logits = bucket
                .request
                .get_output_tensor_by_index(0)
                .map_err(ov_err("output"))?;
            let shape = logits.get_shape().map_err(ov_err("output shape"))?;
            let dims = shape.get_dimensions();
            let t = chunk.seq_len;
            if dims.len() != 3 || dims[0] as usize != self.batch || dims[1] as usize != t {
                return Err(format!(
                    "出力の形が [B={}, T={t}, 語彙] でない: {dims:?}",
                    self.batch
                ));
            }
            let vocab = dims[2] as usize;
            let values: Vec<f32> = match logits.get_element_type().map_err(ov_err("output type"))? {
                ElementType::F32 => logits.get_data::<f32>().map_err(ov_err("output"))?.to_vec(),
                ElementType::F16 => logits
                    .get_data::<u16>()
                    .map_err(ov_err("output"))?
                    .iter()
                    .map(|&b| score::f16_to_f32(b))
                    .collect(),
                other => return Err(format!("出力の型が f32 / f16 でない: {other:?}")),
            };
            for (b, &r) in chunk.rows.iter().enumerate() {
                let row = &plan.rows[r];
                let slice = &values[b * t * vocab..(b + 1) * t * vocab];
                let nll = score::row_nll(slice, vocab, row);
                out[row.candidate] = Some(nll).filter(|x| x.is_finite());
            }
        }
        Ok(out)
    }
}

/// IR を読み、長さごとに [B, T] に固定してコンパイルする
pub fn open(
    job: &Job,
    target: OvTarget,
    precision: Precision,
    tokens: &TokenSet,
) -> Result<Opened, String> {
    let batch = job.batch.unwrap_or(1).max(1);
    let (xml, _) = ir_files(&job.dir, precision, batch, SEQ_LENS[0]);
    if !xml.is_file() {
        return Err(format!(
            "IR が無い（{}\\{IR_DIR}\\{}\\{IR_STEM}.xml）",
            crate::MODELS_DIR,
            precision.name()
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

    let mut buckets = Vec::new();
    let mut compile_ms = Vec::new();
    for &t in &SEQ_LENS {
        let t0 = Instant::now();
        let bucket = compile(&mut core, &job.dir, precision, batch, t, &dev)?;
        compile_ms.push((t, t0.elapsed().as_secs_f64() * 1000.0));
        buckets.push(bucket);
    }
    let load: Duration = started.elapsed();

    let plans: Vec<Plan> = tokens
        .items
        .iter()
        .map(|it| score::plan(it, tokens.eos, batch, &SEQ_LENS))
        .collect();
    let too_long = plans.iter().map(|p| p.too_long.len()).sum();
    Ok(Opened {
        scorer: Box::new(OvItems {
            buckets,
            plans,
            batch,
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

fn compile(
    core: &mut Core,
    dir: &Path,
    precision: Precision,
    batch: usize,
    seq_len: usize,
    dev: &DeviceType<'static>,
) -> Result<Bucket, String> {
    let (xml, bin) = ir_files(dir, precision, batch, seq_len);
    let path = |p: &Path| {
        p.to_str()
            .map(str::to_owned)
            .ok_or("IR の場所が UTF-8 でない")
    };
    let mut model = core
        .read_model_from_file(&path(&xml)?, &path(&bin)?)
        .map_err(ov_err("read_model"))?;
    let n = model.get_inputs_len().map_err(ov_err("inputs"))?;
    let mut specs = Vec::with_capacity(n);
    for i in 0..n {
        let node = model.get_input_by_index(i).map_err(ov_err("input"))?;
        let name = node.get_name().map_err(ov_err("input name"))?;
        let et = node.get_element_type().map_err(ov_err("input type"))?;
        let kind = input_kind(&name)
            .ok_or_else(|| format!("知らない入力 `{name}`（KV キャッシュつきの IR は使えない）"))?;
        specs.push((name, kind, et));
    }
    if !specs.iter().any(|(_, k, _)| *k == InputKind::Ids) {
        return Err("入力 input_ids が無い".into());
    }
    let dims = [batch as i64, seq_len as i64];
    let shape = PartialShape::new_static(2, &dims).map_err(ov_err("shape"))?;
    let reshape: Vec<(&str, &PartialShape)> = specs
        .iter()
        .map(|(name, _, _)| (name.as_str(), &shape))
        .collect();
    model
        .reshape(&reshape)
        .map_err(ov_err(&format!("reshape [B={batch}, T={seq_len}]")))?;
    let mut compiled = core
        .compile_model(&model, dev.to_owned())
        .map_err(ov_err(&format!("compile [B={batch}, T={seq_len}]")))?;
    let mut request = compiled
        .create_infer_request()
        .map_err(ov_err("infer request"))?;
    let tensor_shape = Shape::new(&dims).map_err(ov_err("shape"))?;
    let mut inputs = Vec::with_capacity(specs.len());
    for (name, kind, et) in specs {
        let tensor = Tensor::new(et, &tensor_shape).map_err(ov_err("tensor"))?;
        request
            .set_tensor(&name, &tensor)
            .map_err(ov_err("set_tensor"))?;
        inputs.push((kind, et, tensor));
    }
    Ok(Bucket {
        seq_len,
        request,
        inputs,
        _compiled: compiled,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ir_files_prefers_shaped_ir() {
        let dir = tempfile::tempdir().unwrap();
        let base = ir_dir(dir.path(), Precision::Int8);
        std::fs::create_dir_all(&base).unwrap();
        let (xml, bin) = ir_files(dir.path(), Precision::Int8, 8, 32);
        assert_eq!(xml, base.join("openvino_model.xml"));
        assert_eq!(bin, base.join("openvino_model.bin"));
        std::fs::write(base.join("openvino_model_b8_t32.xml"), "").unwrap();
        let (xml, bin) = ir_files(dir.path(), Precision::Int8, 8, 32);
        assert_eq!(xml, base.join("openvino_model_b8_t32.xml"));
        assert_eq!(bin, base.join("openvino_model_b8_t32.bin"));
    }

    #[test]
    fn known_inputs() {
        assert_eq!(input_kind("input_ids"), Some(InputKind::Ids));
        assert_eq!(input_kind("beam_idx"), None);
    }

    #[test]
    fn missing_lib_is_an_error() {
        let err = load_lib(Some(Path::new("/nonexistent/openvino_c"))).unwrap_err();
        assert!(err.contains(LIB_NAME), "{err}");
    }
}
