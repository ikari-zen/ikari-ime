//! `vectors.bin` のビルド（ADR 0002 §3.4, §13.1）。
//!
//! chiVe の word2vec テキスト（1行目は `語彙数 次元数`、以降は `語 値 値 …`。頻度の降順）を読み、
//! 辞書の lemma（Sudachi の正規化表記）と**完全一致**する語だけを、テキストの順に行として残す。
//! 各行は L2 正規化して ×127 で丸めた i8。クラスタは球面 k-means（k=64、反復20回、種は固定）で求める。

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs;
use std::io::{self, BufRead};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use ikari_dict_format::{
    FormatError, NO_VECTOR_ROW, SYSTEM_DICT_FILE_NAME, VECTORS_FILE_NAME, VectorsSpec,
    encode_vectors, parse_dict_layout, read_lemma,
};

use crate::decompose::SplitTable;
use crate::policy::ChiveVocab;
use crate::{BuildError, open_reader, write_atomically};

pub const DEFAULT_CLUSTERS: u16 = 64;
pub const DEFAULT_KMEANS_ITERS: u32 = 20;
/// k-means の乱数の種（固定）
pub const KMEANS_SEED: u64 = 0x1BAD_5EED_1CA2_1BE5;
/// k-means の学習に使う行数の上限（頻度の高い順）。全行を使うと時間がかかるため
pub const KMEANS_MAX_TRAIN_ROWS: usize = 100_000;
/// 量子化の倍率（i8 の最大値）
const QUANT_SCALE: f32 = 127.0;
/// k-means の1チャンクの点数（スレッド数に依らず結果が決まるよう、固定にする）
const KMEANS_CHUNK_POINTS: usize = 2048;
/// 内積を 8 本の独立した累積に分ける（自動ベクトル化と、順序の固定のため）
const DOT_LANES: usize = 8;

pub struct VectorsConfig {
    /// chiVe の word2vec テキスト
    pub chive: PathBuf,
    /// system.dict があるディレクトリ
    pub dict_dir: PathBuf,
    /// 出力先（vectors.bin のパス）
    pub out: PathBuf,
    /// 辞書を作ったときと同じ lex.csv（同じ順。small、core、notcore）。空なら分解をしない（ADR 0012 §12）
    pub lex: Vec<PathBuf>,
    pub clusters: u16,
    pub kmeans_iters: u32,
    /// メタ情報の `source`（例: `chive-1.3-mc90`）
    pub label: String,
}

impl VectorsConfig {
    pub fn new(chive: PathBuf, dict_dir: PathBuf, out: PathBuf) -> Self {
        Self {
            chive,
            dict_dir,
            out,
            lex: Vec::new(),
            clusters: DEFAULT_CLUSTERS,
            kmeans_iters: DEFAULT_KMEANS_ITERS,
            label: "chive-1.3-mc90".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VectorsReport {
    pub dict_id: u64,
    pub dims: usize,
    pub chive_rows: usize,
    /// lemma と一致して行にした語の数
    pub matched: usize,
    /// 零ベクトルなどで使えなかった一致
    pub skipped_zero: usize,
    /// chiVe に同じ語が2回以上あり、2回目以降を捨てた数
    pub skipped_duplicate: usize,
    /// chiVe にない lemma に、分割の部分のベクトルの平均を付けた数（`VectorsConfig::lex` があるとき）
    pub decomposed: usize,
    pub lemma_count: usize,
    pub clusters: usize,
    pub cluster_sizes: Vec<usize>,
    pub train_rows: usize,
    pub bytes: u64,
}

impl VectorsReport {
    /// 辞書の lemma のうち、ベクトルを持つ割合（%）
    pub fn lemma_coverage_percent(&self) -> f64 {
        if self.lemma_count == 0 {
            0.0
        } else {
            self.matched as f64 * 100.0 / self.lemma_count as f64
        }
    }

    /// chiVe の語のうち、辞書の lemma と一致した割合（%）
    pub fn chive_match_percent(&self) -> f64 {
        if self.chive_rows == 0 {
            0.0
        } else {
            self.matched as f64 * 100.0 / self.chive_rows as f64
        }
    }
}

impl fmt::Display for VectorsReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "dict_id:       {:#018x}", self.dict_id)?;
        writeln!(f, "dims:          {}", self.dims)?;
        writeln!(f, "chiVe words:   {}", self.chive_rows)?;
        writeln!(
            f,
            "matched:       {} ({:.1}% of chiVe, {:.1}% of {} lemmas)",
            self.matched,
            self.chive_match_percent(),
            self.lemma_coverage_percent(),
            self.lemma_count
        )?;
        writeln!(f, "skipped zero:  {}", self.skipped_zero)?;
        writeln!(f, "skipped dup:   {}", self.skipped_duplicate)?;
        writeln!(f, "decomposed:    {}", self.decomposed)?;
        writeln!(
            f,
            "clusters:      {} (trained on {} rows; sizes {:?})",
            self.clusters, self.train_rows, self.cluster_sizes
        )?;
        write!(f, "vectors.bin:   {} bytes", self.bytes)
    }
}

fn input_error(path: &Path, line: usize, message: impl Into<String>) -> BuildError {
    BuildError::Input {
        file: path.to_path_buf(),
        line,
        message: message.into(),
    }
}

/// word2vec テキストの1行目（`語彙数 次元数`）か。語彙数と次元数の2語だけで、どちらも整数。
fn parse_header(line: &str) -> Option<(usize, usize)> {
    let mut parts = line.split_whitespace();
    let (rows, dims) = (parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    Some((rows.parse().ok()?, dims.parse().ok()?))
}

/// L2 正規化して ×127 で丸める。零ベクトル（または非有限）なら None。
fn quantize(values: &[f32]) -> Option<(Vec<f32>, Vec<i8>)> {
    let norm = values.iter().map(|v| v * v).sum::<f32>().sqrt();
    if !norm.is_finite() || norm == 0.0 {
        return None;
    }
    let unit: Vec<f32> = values.iter().map(|v| v / norm).collect();
    let quantized = unit
        .iter()
        .map(|v| (v * QUANT_SCALE).round().clamp(-QUANT_SCALE, QUANT_SCALE) as i8)
        .collect();
    Some((unit, quantized))
}

/// chiVe の全キーを、行の並び（頻度順）のまま読む（`lemma_rank` の語彙用）。
pub fn read_chive_vocab(path: &Path) -> Result<ChiveVocab, BuildError> {
    let mut reader = open_reader(path)?;
    let mut keys = Vec::new();
    let mut line = String::new();
    let mut lineno = 0usize;
    loop {
        lineno += 1;
        if crate::read_line_at(&mut reader, &mut line, path, lineno)? == 0 {
            return Ok(ChiveVocab::from_keys(keys));
        }
        if lineno == 1 && parse_header(&line).is_some() {
            continue;
        }
        if let Some((key, _)) = line.split_once(' ') {
            keys.push(key.to_string());
        }
    }
}

#[derive(Debug)]
struct LoadedRows {
    dims: usize,
    chive_rows: usize,
    rows: Vec<i8>,
    /// 学習用の正規化済みの行（先頭 `KMEANS_MAX_TRAIN_ROWS` 行まで）
    train: Vec<f32>,
    lemma_rows: Vec<u32>,
    skipped_zero: usize,
    skipped_duplicate: usize,
    /// `extras` の語の正規化済みのベクトル（chiVe にあり、零ベクトルでないもの）
    extra_vectors: HashMap<String, Vec<f32>>,
}

/// word2vec テキストを読み、`lemma_ids` と一致する語を行にする。`extras` の語は、行にはせずベクトルだけ覚える
/// （分解で、辞書に行のない部分の語のベクトルを使うため）。
fn load_rows<R: BufRead>(
    mut reader: R,
    path: &Path,
    lemma_ids: &HashMap<&str, u32>,
    extras: &HashSet<String>,
    lemma_count: usize,
) -> Result<LoadedRows, BuildError> {
    let mut loaded = LoadedRows {
        dims: 0,
        chive_rows: 0,
        rows: Vec::new(),
        train: Vec::new(),
        lemma_rows: vec![NO_VECTOR_ROW; lemma_count],
        skipped_zero: 0,
        skipped_duplicate: 0,
        extra_vectors: HashMap::new(),
    };
    let mut line = String::new();
    let mut lineno = 0usize;
    let mut values: Vec<f32> = Vec::new();
    loop {
        lineno += 1;
        line.clear();
        let n = reader.read_line(&mut line).map_err(|e| {
            if e.kind() == io::ErrorKind::InvalidData {
                input_error(path, lineno, "invalid UTF-8")
            } else {
                BuildError::Io(e)
            }
        })?;
        if n == 0 {
            break;
        }
        let text = line.trim_end_matches(['\n', '\r']);
        if let (1, Some((_, dims))) = (lineno, parse_header(text)) {
            loaded.dims = dims;
            continue;
        }
        let Some((key, rest)) = text.split_once(' ') else {
            return Err(input_error(path, lineno, "expected `word value value ...`"));
        };
        loaded.chive_rows += 1;
        let lemma_id = lemma_ids.get(key).copied();
        let wanted_extra = extras.contains(key) && !loaded.extra_vectors.contains_key(key);
        if lemma_id.is_none() && !wanted_extra {
            continue;
        }
        values.clear();
        for token in rest.split_ascii_whitespace() {
            let v: f32 = token
                .parse()
                .map_err(|_| input_error(path, lineno, format!("`{token}` is not a number")))?;
            values.push(v);
        }
        if loaded.dims == 0 {
            loaded.dims = values.len();
        }
        if values.len() != loaded.dims {
            return Err(input_error(
                path,
                lineno,
                format!("expected {} values, found {}", loaded.dims, values.len()),
            ));
        }
        let unit_and_quantized = quantize(&values);
        if wanted_extra && let Some((unit, _)) = &unit_and_quantized {
            loaded.extra_vectors.insert(key.to_string(), unit.clone());
        }
        let Some(lemma_id) = lemma_id else {
            continue;
        };
        if loaded.lemma_rows[lemma_id as usize] != NO_VECTOR_ROW {
            loaded.skipped_duplicate += 1;
            continue;
        }
        let Some((unit, quantized)) = unit_and_quantized else {
            loaded.skipped_zero += 1;
            continue;
        };
        let row = u32::try_from(loaded.rows.len() / loaded.dims)
            .map_err(|_| input_error(path, lineno, "too many rows"))?;
        loaded.lemma_rows[lemma_id as usize] = row;
        loaded.rows.extend_from_slice(&quantized);
        if (row as usize) < KMEANS_MAX_TRAIN_ROWS {
            loaded.train.extend_from_slice(&unit);
        }
    }
    Ok(loaded)
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    let len = a.len().min(b.len());
    let body = len - len % DOT_LANES;
    let mut acc = [0.0f32; DOT_LANES];
    for start in (0..body).step_by(DOT_LANES) {
        for (lane, sum) in acc.iter_mut().enumerate() {
            *sum += a[start + lane] * b[start + lane];
        }
    }
    let tail: f32 = (body..len).map(|i| a[i] * b[i]).sum();
    acc.iter().sum::<f32>() + tail
}

fn xorshift(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

/// 球面 k-means の結果。
struct Clustering {
    /// 正規化済みの重心（`k * dims`）
    centroids: Vec<f32>,
    sizes: Vec<usize>,
}

struct ChunkResult {
    assignment: Vec<u16>,
    sums: Vec<f32>,
}

/// 球面 k-means（コサイン類似度が最大の重心に割り当て、平均を正規化して更新する）。
/// 乱数の種と、チャンクごとの和の順序が固定なので、スレッド数に依らず同じ結果になる。
fn kmeans(points: &[f32], dims: usize, k: usize, iters: u32) -> Clustering {
    let n = points.len().checked_div(dims).unwrap_or(0);
    let k = k.min(n);
    if k == 0 {
        return Clustering {
            centroids: Vec::new(),
            sizes: Vec::new(),
        };
    }
    // 初期値: 最遠点法。最初の1点は種から決め、以降は「選んだ重心との類似度が最も低い点」を選ぶ。
    let mut state = KMEANS_SEED;
    let first = (xorshift(&mut state) % n as u64) as usize;
    let mut centroids: Vec<f32> = Vec::with_capacity(k * dims);
    centroids.extend_from_slice(&points[first * dims..(first + 1) * dims]);
    let mut nearest: Vec<f32> = points
        .chunks_exact(dims)
        .map(|p| dot(p, &centroids[..dims]))
        .collect();
    while centroids.len() < k * dims {
        let farthest = nearest
            .iter()
            .enumerate()
            .fold((0usize, f32::INFINITY), |best, (i, &sim)| {
                if sim < best.1 { (i, sim) } else { best }
            })
            .0;
        let start = centroids.len();
        centroids.extend_from_slice(&points[farthest * dims..(farthest + 1) * dims]);
        let added = &centroids[start..];
        for (sim, point) in nearest.iter_mut().zip(points.chunks_exact(dims)) {
            *sim = sim.max(dot(point, added));
        }
    }

    let chunk_count = n.div_ceil(KMEANS_CHUNK_POINTS);
    let threads = std::thread::available_parallelism()
        .map_or(1, |p| p.get())
        .min(chunk_count);
    let mut sizes = vec![0usize; k];
    for _ in 0..iters.max(1) {
        let next_chunk = AtomicUsize::new(0);
        let current = &centroids;
        let mut results: Vec<Option<ChunkResult>> = (0..chunk_count).map(|_| None).collect();
        std::thread::scope(|scope| {
            let workers: Vec<_> = (0..threads)
                .map(|_| {
                    scope.spawn(|| {
                        let mut done = Vec::new();
                        loop {
                            let chunk = next_chunk.fetch_add(1, Ordering::Relaxed);
                            if chunk >= chunk_count {
                                return done;
                            }
                            let from = chunk * KMEANS_CHUNK_POINTS;
                            let to = (from + KMEANS_CHUNK_POINTS).min(n);
                            let mut result = ChunkResult {
                                assignment: Vec::with_capacity(to - from),
                                sums: vec![0.0; k * dims],
                            };
                            for p in from..to {
                                let point = &points[p * dims..(p + 1) * dims];
                                let mut best = (0usize, f32::NEG_INFINITY);
                                for (c, centroid) in current.chunks_exact(dims).enumerate() {
                                    let score = dot(point, centroid);
                                    if score > best.1 {
                                        best = (c, score);
                                    }
                                }
                                result.assignment.push(best.0 as u16);
                                let sum = &mut result.sums[best.0 * dims..(best.0 + 1) * dims];
                                for (s, x) in sum.iter_mut().zip(point) {
                                    *s += x;
                                }
                            }
                            done.push((chunk, result));
                        }
                    })
                })
                .collect();
            for worker in workers {
                if let Ok(done) = worker.join() {
                    for (chunk, result) in done {
                        results[chunk] = Some(result);
                    }
                }
            }
        });
        // チャンク番号の順に合計する
        let mut totals = vec![0.0f64; k * dims];
        sizes.iter_mut().for_each(|s| *s = 0);
        for result in results.into_iter().flatten() {
            for &a in &result.assignment {
                sizes[usize::from(a)] += 1;
            }
            for (t, s) in totals.iter_mut().zip(&result.sums) {
                *t += f64::from(*s);
            }
        }
        for c in 0..k {
            let sum = &totals[c * dims..(c + 1) * dims];
            let norm = sum.iter().map(|v| v * v).sum::<f64>().sqrt();
            // 点が割り当てられなかった（または零ベクトルになった）重心は前の値を残す
            if sizes[c] > 0 && norm > 0.0 {
                for (dst, v) in centroids[c * dims..(c + 1) * dims].iter_mut().zip(sum) {
                    *dst = (v / norm) as f32;
                }
            }
        }
    }
    Clustering { centroids, sizes }
}

fn quantize_centroids(centroids: &[f32]) -> Vec<i8> {
    centroids
        .iter()
        .map(|v| (v * QUANT_SCALE).round().clamp(-QUANT_SCALE, QUANT_SCALE) as i8)
        .collect()
}

/// chiVe に行のない辞書の lemma に、分割の部分のベクトルの平均（正規化して量子化）を行として足す。足した数を返す。
/// 行は lemma の見つかった順（lex.csv の順）に足す。
fn append_decomposed(
    loaded: &mut LoadedRows,
    table: &SplitTable,
    lemma_ids: &HashMap<&str, u32>,
) -> usize {
    let decompositions = table.decompose(
        |lemma| {
            lemma_ids
                .get(lemma)
                .is_some_and(|&id| loaded.lemma_rows[id as usize] == NO_VECTOR_ROW)
        },
        |lemma| loaded.extra_vectors.contains_key(lemma),
    );
    let mut added = 0;
    for (lemma, parts) in decompositions {
        let Some(&lemma_id) = lemma_ids.get(lemma.as_str()) else {
            continue;
        };
        let mut mean = vec![0f32; loaded.dims];
        for part in &parts {
            let Some(unit) = loaded.extra_vectors.get(part) else {
                continue;
            };
            for (m, x) in mean.iter_mut().zip(unit) {
                *m += x;
            }
        }
        let Some((unit, quantized)) = quantize(&mean) else {
            continue;
        };
        let Ok(row) = u32::try_from(loaded.rows.len() / loaded.dims) else {
            break;
        };
        loaded.lemma_rows[lemma_id as usize] = row;
        loaded.rows.extend_from_slice(&quantized);
        if (row as usize) < KMEANS_MAX_TRAIN_ROWS {
            loaded.train.extend_from_slice(&unit);
        }
        added += 1;
    }
    added
}

/// chiVe から vectors.bin を作る。
pub fn build_vectors(cfg: &VectorsConfig) -> Result<VectorsReport, BuildError> {
    if cfg.label.contains(['\n', '\r']) {
        return Err(BuildError::Config("label must be a single line".into()));
    }
    let dict_path = cfg.dict_dir.join(SYSTEM_DICT_FILE_NAME);
    let dict_bytes = fs::read(&dict_path).map_err(|e| {
        BuildError::Io(io::Error::new(
            e.kind(),
            format!("cannot read {}: {e}", dict_path.display()),
        ))
    })?;
    let layout = parse_dict_layout(&dict_bytes)?;
    let lemma_table = &dict_bytes[layout.lemma_table.clone()];
    let lemma_count = layout.lemma_count as usize;
    let mut lemma_ids: HashMap<&str, u32> = HashMap::with_capacity(lemma_count);
    for id in 0..layout.lemma_count {
        let text = read_lemma(lemma_table, layout.lemma_count, id).ok_or_else(|| {
            BuildError::Format(FormatError::BadSection {
                name: "lemma_table",
                message: format!("lemma {id} is unreadable"),
            })
        })?;
        lemma_ids.insert(text, id);
    }

    // 分解（ADR 0012 §12）: 分割の部分の語のベクトルも、chiVe を読むときに拾う
    let split_table = if cfg.lex.is_empty() {
        None
    } else {
        Some(SplitTable::read(&cfg.lex)?)
    };
    let extras = split_table.as_ref().map_or_else(HashSet::new, |table| {
        table.wanted_parts(|lemma| lemma_ids.contains_key(lemma))
    });
    let reader = open_reader(&cfg.chive)?;
    let mut loaded = load_rows(reader, &cfg.chive, &lemma_ids, &extras, lemma_count)?;
    drop(extras);
    // chiVe と一致して行にした数（分解で足す前）
    let matched = loaded.rows.len().checked_div(loaded.dims).unwrap_or(0);
    let decomposed = match &split_table {
        Some(table) => append_decomposed(&mut loaded, table, &lemma_ids),
        None => 0,
    };
    drop(split_table);
    let dims = u16::try_from(loaded.dims)
        .ok()
        .filter(|&d| d > 0)
        .ok_or_else(|| BuildError::Config(format!("unsupported dims {}", loaded.dims)))?;
    let train_rows = loaded.train.len() / loaded.dims;
    let clustering = kmeans(
        &loaded.train,
        loaded.dims,
        usize::from(cfg.clusters),
        cfg.kmeans_iters,
    );
    let cluster_count = u16::try_from(clustering.sizes.len())
        .map_err(|_| BuildError::Config("too many clusters".into()))?;
    let centroids = quantize_centroids(&clustering.centroids);

    let mut meta = String::new();
    let decomposed_meta = (!cfg.lex.is_empty()).then(|| ("decomposed", decomposed.to_string()));
    for (key, value) in [
        ("source", cfg.label.clone()),
        ("builder_version", env!("CARGO_PKG_VERSION").to_string()),
        ("chive_rows", loaded.chive_rows.to_string()),
        ("matched", matched.to_string()),
        ("lemma_count", lemma_count.to_string()),
        ("kmeans_iters", cfg.kmeans_iters.to_string()),
        ("kmeans_seed", format!("{KMEANS_SEED:#x}")),
        ("kmeans_train_rows", train_rows.to_string()),
    ]
    .into_iter()
    .chain(decomposed_meta)
    {
        meta.push_str(&format!("{key}={value}\n"));
    }
    let bytes = encode_vectors(&VectorsSpec {
        dict_id: layout.dict_id,
        dims,
        cluster_count,
        lemma_rows: &loaded.lemma_rows,
        rows: &loaded.rows,
        centroids: &centroids,
        meta: &meta,
    })?;

    let out_dir = cfg
        .out
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let file_name = cfg
        .out
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(VECTORS_FILE_NAME);
    write_atomically(out_dir, &[(file_name, &bytes)])?;

    Ok(VectorsReport {
        dict_id: layout.dict_id,
        dims: loaded.dims,
        chive_rows: loaded.chive_rows,
        matched,
        skipped_zero: loaded.skipped_zero,
        skipped_duplicate: loaded.skipped_duplicate,
        decomposed,
        lemma_count,
        clusters: usize::from(cluster_count),
        cluster_sizes: clustering.sizes,
        train_rows,
        bytes: bytes.len() as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids<'a>(words: &[&'a str]) -> HashMap<&'a str, u32> {
        words
            .iter()
            .enumerate()
            .map(|(i, w)| (*w, i as u32))
            .collect()
    }

    fn load(text: &str, words: &[&str]) -> Result<LoadedRows, BuildError> {
        load_rows(
            text.as_bytes(),
            Path::new("test.txt"),
            &ids(words),
            &HashSet::new(),
            words.len(),
        )
    }

    #[test]
    fn header_is_detected_only_when_it_is_two_integers() {
        assert_eq!(parse_header("410533 300"), Some((410_533, 300)));
        assert_eq!(parse_header("の 0.1 0.2"), None);
        assert_eq!(parse_header("3 4 5"), None);
        assert_eq!(parse_header("3 x"), None);
        assert_eq!(parse_header(""), None);
    }

    #[test]
    fn quantize_normalizes_and_rounds() {
        let (unit, q) = quantize(&[3.0, 4.0]).unwrap();
        assert!((unit[0] - 0.6).abs() < 1e-6 && (unit[1] - 0.8).abs() < 1e-6);
        assert_eq!(q, vec![76, 102]); // 0.6*127=76.2, 0.8*127=101.6
        assert!(quantize(&[0.0, 0.0]).is_none());
        assert!(quantize(&[f32::NAN, 1.0]).is_none());
        let (_, q) = quantize(&[-1.0, 0.0]).unwrap();
        assert_eq!(q, vec![-127, 0]);
    }

    #[test]
    fn rows_follow_the_text_order_and_match_lemmas_exactly() {
        let text = "5 2\n猫 1 0\n犬 0 1\nその他 1 1\n鳥 -1 0\n犬 5 5\n";
        let loaded = load(text, &["鳥", "犬", "猫", "未登場"]).unwrap();
        assert_eq!(loaded.dims, 2);
        assert_eq!(loaded.chive_rows, 5);
        // 行は 猫(0)、犬(1)、鳥(2) の順。lemma 番号は 鳥=0、犬=1、猫=2
        assert_eq!(loaded.lemma_rows, vec![2, 1, 0, NO_VECTOR_ROW]);
        assert_eq!(loaded.rows, vec![127, 0, 0, 127, -127, 0]);
        assert_eq!(loaded.skipped_duplicate, 1);
        assert_eq!(loaded.skipped_zero, 0);
    }

    #[test]
    fn works_without_a_header_and_reports_bad_lines() {
        let loaded = load("猫 1 0\n", &["猫"]).unwrap();
        assert_eq!((loaded.dims, loaded.chive_rows), (2, 1));
        // 次元が合わない
        let err = load("2 2\n猫 1 0\n犬 1 0 0\n", &["猫", "犬"]).unwrap_err();
        assert!(err.to_string().contains("test.txt:3"), "{err}");
        // 数値でない
        assert!(load("猫 1 x\n", &["猫"]).is_err());
        // 語と値の区切りがない
        assert!(load("猫\n", &["猫"]).is_err());
        // 一致しない行は、値が壊れていても読み飛ばす
        assert!(load("2 2\n犬 x y\n猫 1 0\n", &["猫"]).is_ok());
        // 零ベクトルは使わない
        let loaded = load("2 2\n猫 0 0\n", &["猫"]).unwrap();
        assert_eq!((loaded.skipped_zero, loaded.rows.len()), (1, 0));
        assert_eq!(loaded.lemma_rows, vec![NO_VECTOR_ROW]);
    }

    #[test]
    fn dot_matches_a_naive_sum() {
        let a: Vec<f32> = (0..19).map(|i| i as f32 * 0.5).collect();
        let b: Vec<f32> = (0..19).map(|i| 1.0 - i as f32 * 0.1).collect();
        let naive: f32 = a.iter().zip(&b).map(|(x, y)| x * y).sum();
        assert!((dot(&a, &b) - naive).abs() < 1e-3);
    }

    fn blobs() -> (Vec<f32>, usize) {
        // 3 方向 × 各 40 点（方向に小さなずれを足す）
        let dirs = [[1.0f32, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let mut points = Vec::new();
        let mut state = 7u64;
        for round in 0..40 {
            for d in &dirs {
                let mut p = *d;
                for v in p.iter_mut() {
                    *v += (xorshift(&mut state) % 1000) as f32 / 20_000.0 + round as f32 * 0.0001;
                }
                let norm = p.iter().map(|v| v * v).sum::<f32>().sqrt();
                points.extend(p.iter().map(|v| v / norm));
            }
        }
        (points, 3)
    }

    #[test]
    fn kmeans_separates_blobs_and_is_deterministic() {
        let (points, dims) = blobs();
        let a = kmeans(&points, dims, 3, 10);
        let b = kmeans(&points, dims, 3, 10);
        assert_eq!(a.centroids, b.centroids);
        assert_eq!(a.sizes, vec![40, 40, 40]);
        // 重心は正規化されている
        for c in a.centroids.chunks_exact(dims) {
            let norm = c.iter().map(|v| v * v).sum::<f32>().sqrt();
            assert!((norm - 1.0).abs() < 1e-4);
        }
    }

    #[test]
    fn kmeans_handles_edge_cases() {
        assert!(kmeans(&[], 3, 4, 5).sizes.is_empty());
        assert!(kmeans(&[1.0, 0.0], 0, 4, 5).sizes.is_empty());
        // 点より多い k は点の数に切り詰める
        let two = kmeans(&[1.0, 0.0, 0.0, 1.0], 2, 8, 3);
        assert_eq!(two.sizes.len(), 2);
        assert_eq!(two.sizes.iter().sum::<usize>(), 2);
    }
}
