//! mmap した vectors.bin（ADR 0002 §3.4）。
//!
//! 開くときはヘッダとセクション範囲、`dict_id` の照合だけを行う（O(1)）。行は触れたページだけが常駐する。
//! 読み出しは範囲付きで、壊れたファイルでもパニックしない。

use std::fs::File;
use std::path::Path;
use std::sync::Arc;

use ikari_dict_format::{FormatError, VectorsLayout, parse_vectors_layout, read_lemma_row};
use memmap2::Mmap;

use super::WordVectors;
use crate::dict::DictError;

struct MmapVectors {
    mmap: Mmap,
    layout: VectorsLayout,
}

/// dict_id が system.dict のものと一致しなければエラー。
pub fn open_vectors(path: &Path, dict_id: u64) -> Result<Arc<dyn WordVectors>, DictError> {
    let file = File::open(path).map_err(|e| {
        DictError::Io(std::io::Error::new(
            e.kind(),
            format!("cannot open {}: {e}", path.display()),
        ))
    })?;
    if file.metadata()?.len() == 0 {
        return Err(FormatError::Truncated {
            what: "vectors.bin",
            need: 1,
            actual: 0,
        }
        .into());
    }
    // SAFETY: マップしたファイルが他のプロセスやスレッドによって変更・切り詰めされないことが前提。
    // 辞書と同じく、更新は別ファイルを作って rename で切り替える運用で守る。
    let mmap = unsafe { Mmap::map(&file)? };
    let layout = parse_vectors_layout(&mmap)?;
    if layout.dict_id != dict_id {
        return Err(FormatError::BadSection {
            name: "dict_id",
            message: format!(
                "vectors.bin was built for dictionary {:#018x} but the dictionary is {dict_id:#018x}; rebuild vectors.bin",
                layout.dict_id
            ),
        }
        .into());
    }
    Ok(Arc::new(MmapVectors { mmap, layout }))
}

impl MmapVectors {
    fn dims_usize(&self) -> usize {
        usize::from(self.layout.dims)
    }

    fn row_bytes(&self, row: u32) -> Option<&[u8]> {
        if row >= self.layout.row_count {
            return None;
        }
        let rows = self.mmap.get(self.layout.rows.clone())?;
        let dims = self.dims_usize();
        let start = (row as usize).checked_mul(dims)?;
        rows.get(start..start.checked_add(dims)?)
    }

    fn row_of(&self, lemma_id: u32) -> Option<u32> {
        let section = self.mmap.get(self.layout.lemma_rows.clone())?;
        read_lemma_row(section, lemma_id)
    }
}

/// `&[u8]` を同じ長さの `&[i8]` として見る。
fn as_i8(bytes: &[u8]) -> &[i8] {
    // SAFETY: u8 と i8 はサイズも配置も同じで、どのビット列も有効な値。長さと寿命は元のスライスのまま。
    unsafe { std::slice::from_raw_parts(bytes.as_ptr().cast::<i8>(), bytes.len()) }
}

impl WordVectors for MmapVectors {
    fn dims(&self) -> usize {
        self.dims_usize()
    }

    fn vector(&self, lemma_id: u32) -> Option<&[i8]> {
        let row = self.row_of(lemma_id)?;
        self.row_bytes(row).map(as_i8)
    }

    /// v と最も近い重心（コサイン類似度が最大。同点は番号の小さい方）。
    /// v の次元が dims と違う、v が零ベクトル、または重心がなければ None。
    fn nearest_cluster(&self, v: &[f32]) -> Option<u16> {
        let dims = self.dims_usize();
        if v.len() != dims || self.layout.cluster_count == 0 || v.iter().all(|&x| x == 0.0) {
            return None;
        }
        let centroids = as_i8(self.mmap.get(self.layout.centroids.clone())?);
        let mut best: Option<(u16, f32)> = None;
        for (cluster, centroid) in centroids.chunks_exact(dims).enumerate() {
            let (mut dot, mut norm_sq) = (0.0f32, 0.0f32);
            for (&c, &x) in centroid.iter().zip(v) {
                let c = f32::from(c);
                dot += c * x;
                norm_sq += c * c;
            }
            if norm_sq == 0.0 {
                continue;
            }
            // v のノルムは全重心で共通なので比較には要らない
            let score = dot / norm_sq.sqrt();
            let cluster = u16::try_from(cluster).ok()?;
            if best.is_none_or(|(_, s)| score > s) {
                best = Some((cluster, score));
            }
        }
        best.map(|(cluster, _)| cluster)
    }

    fn rank(&self, lemma_id: u32) -> Option<u32> {
        self.row_of(lemma_id).filter(|&r| r < self.layout.row_count)
    }
}
