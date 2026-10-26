//! 単語ベクトル（vectors.bin。ADR 0002 §3.4, §13.1）。

mod mmap;

pub use mmap::open_vectors;

pub trait WordVectors: Send + Sync {
    fn dims(&self) -> usize;
    /// lemma 番号の行ベクトル（L2 正規化して ×127 で丸めた i8）。語彙になければ None
    fn vector(&self, lemma_id: u32) -> Option<&[i8]>;
    fn nearest_cluster(&self, v: &[f32]) -> Option<u16>;
    /// 頻度順位（行番号）
    fn rank(&self, lemma_id: u32) -> Option<u32>;
}
