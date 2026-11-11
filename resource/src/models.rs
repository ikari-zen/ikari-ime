//! アプリに埋めた既知のモデルの表（ADR 0058 §5.2、§6.1）。
//!
//! inbox が受けるモデルは、この表にあるものだけ（gguf の中身は llama.cpp のパーサに渡り、こちらは検査していない）。
//! 取得元の URL と元データの sha256 は `xtask/src/data/fetch.rs` に残る。モデルの値は同じなので、
//! このファイルのテストが fetch.rs の定数と一致することを確かめる。

/// 既知のモデル 1 件（入れるファイルの名前と sha256）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KnownModel {
    pub file_name: &'static str,
    pub sha256: &'static str,
    /// ファイルの大きさ（バイト）。sha256 を取る前に、安く弾くために使う
    pub size: u64,
}

/// zenz-v3.2 small の入れる名前（`engine` の `MODEL_FILE_NAME` と同じ）
pub const ZENZ_SMALL_FILE_NAME: &str = "zenz-v3.2-small-Q5_K_M.gguf";
/// 配布元（Hugging Face）のファイルの sha256
pub const ZENZ_SMALL_SHA256: &str =
    "29c223d4c23327b80fd13ebb5ab2555057a46317997d5da391584ffbef0db673";

/// 配布元のファイルの大きさ（バイト）
pub const ZENZ_SMALL_SIZE: u64 = 73_871_936;

/// 今アプリが受けるモデル。モデルを足す・替えるときは ADR で実測してから（CLAUDE.md）
pub const KNOWN_MODELS: &[KnownModel] = &[KnownModel {
    file_name: ZENZ_SMALL_FILE_NAME,
    sha256: ZENZ_SMALL_SHA256,
    size: ZENZ_SMALL_SIZE,
}];

#[cfg(test)]
mod tests {
    use super::*;

    /// 固定表が 2 か所に分かれている間の取り違えを防ぐ（xtask は変えない契約のため、ソースの文字列で照合）
    #[test]
    fn sha256_matches_xtask_fetch_table() {
        let fetch = include_str!("../../xtask/src/data/fetch.rs");
        let want = format!("const SHA_ZENZ_SMALL: &str = \"{ZENZ_SMALL_SHA256}\";");
        assert!(
            fetch.contains(&want),
            "xtask の SHA_ZENZ_SMALL と食い違っています"
        );
    }

    #[test]
    fn file_name_matches_engine() {
        let engine = include_str!("../../engine/src/lib.rs");
        let want = format!("pub const MODEL_FILE_NAME: &str = \"{ZENZ_SMALL_FILE_NAME}\";");
        assert!(
            engine.contains(&want),
            "engine の MODEL_FILE_NAME と食い違っています"
        );
    }

    #[test]
    fn known_sha256_are_well_formed() {
        for m in KNOWN_MODELS {
            assert!(crate::sha256::is_sha256_hex(m.sha256), "{}", m.file_name);
        }
    }
}
