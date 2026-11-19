//! アプリに埋めたモデルの表（ADR 0058 §5.2、§6.1、ADR 0060 §5）。表は `MODEL_PRESETS` の 1 つだけ。
//!
//! inbox が受けるモデルは、この表にあるものだけ（gguf の中身は llama.cpp のパーサに渡り、こちらは検査していない）。
//! 取得元の URL と元データの sha256 は `xtask/src/data/fetch.rs` に残る。モデルの値は同じなので、
//! このファイルのテストが fetch.rs の定数と一致することを確かめる。

/// 受けるモデル 1 件（`MODEL_PRESETS` から作る。入れるファイルの名前と sha256）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KnownModel {
    pub file_name: &'static str,
    pub sha256: &'static str,
    /// ファイルの大きさ（バイト）。sha256 を取る前に、安く弾くために使う
    pub size: u64,
}

use crate::profile::{ModelProfile, ZENZ_V3_2_SMALL_PROFILE};

/// zenz-v3.2 small の入れる名前（`engine` の `MODEL_FILE_NAME` と同じ）
pub const ZENZ_SMALL_FILE_NAME: &str = "zenz-v3.2-small-Q5_K_M.gguf";
/// 配布元（Hugging Face）のファイルの sha256
pub const ZENZ_SMALL_SHA256: &str =
    "29c223d4c23327b80fd13ebb5ab2555057a46317997d5da391584ffbef0db673";

/// 配布元のファイルの大きさ（バイト）
pub const ZENZ_SMALL_SIZE: u64 = 73_871_936;

/// 取得できるプリセットのモデル 1 件（ADR 0076 §4.2。表はアプリに埋め、遠くから取らない）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelPreset {
    /// `ikari-data model fetch --preset <id>` の ID
    pub id: &'static str,
    pub file_name: &'static str,
    pub sha256: &'static str,
    pub size: u64,
    /// 取得元（https。始めの URL のホストを、要求の前に確かめる）
    pub url: &'static str,
    /// `url` のホスト（小文字。表の書き損じで別のホストに行かないよう、`ikari-data` が要求の前に照らす）
    pub host: &'static str,
    /// モデルの記述（プロンプトの形式、加点の重み、上限。ADR 0060 §3）。`profile_by_id` が引く
    pub profile: ModelProfile,
}

/// zenz-v3.2 small の取得元（`xtask/src/data/fetch.rs` の `ZENZ_URL` と同じ）
pub const ZENZ_SMALL_URL: &str =
    "https://huggingface.co/Miwa-Keita/zenz-v3.2-small-gguf/resolve/main/ggml-model-Q5_K_M.gguf";

/// アプリに埋めたモデルの表（唯一。ADR 0060 §5）。取得元（`ikari-data`、xtask）、受けるファイル（`KNOWN_MODELS`）、
/// profile（lm）は、すべてこの表から読む。足すときは評価してユーザーに聞く（CLAUDE.md）
pub const MODEL_PRESETS: &[ModelPreset] = &[ModelPreset {
    id: "zenz-small",
    file_name: ZENZ_SMALL_FILE_NAME,
    sha256: ZENZ_SMALL_SHA256,
    size: ZENZ_SMALL_SIZE,
    url: ZENZ_SMALL_URL,
    host: "huggingface.co",
    profile: ZENZ_V3_2_SMALL_PROFILE,
}];

/// 受けるモデル 1 件を、表の行から作る
const fn known_of(p: &ModelPreset) -> KnownModel {
    KnownModel {
        file_name: p.file_name,
        sha256: p.sha256,
        size: p.size,
    }
}

const KNOWN_COUNT: usize = MODEL_PRESETS.len();

/// 今アプリが受けるモデル。`MODEL_PRESETS` から作る（別の表ではない）
pub const KNOWN_MODELS: &[KnownModel] = &{
    let mut out = [known_of(&MODEL_PRESETS[0]); KNOWN_COUNT];
    let mut i = 0;
    while i < KNOWN_COUNT {
        out[i] = known_of(&MODEL_PRESETS[i]);
        i += 1;
    }
    out
};

/// ID からプリセットを引く
pub fn preset_by_id(id: &str) -> Option<&'static ModelPreset> {
    MODEL_PRESETS.iter().find(|p| p.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_url_matches_xtask_fetch_table() {
        let fetch = include_str!("../../xtask/src/data/fetch.rs");
        assert!(
            fetch.contains(&format!("\"{ZENZ_SMALL_URL}\";")),
            "xtask の ZENZ_URL と食い違っています"
        );
    }

    #[test]
    fn presets_are_known_models_with_https_urls() {
        for p in MODEL_PRESETS {
            assert!(p.url.starts_with("https://"), "{}", p.id);
            let authority = p.url["https://".len()..].split('/').next().unwrap();
            assert_eq!(authority, p.host, "{}", p.id);
            assert!(
                KNOWN_MODELS.iter().any(|k| k.file_name == p.file_name
                    && k.sha256 == p.sha256
                    && k.size == p.size),
                "{}",
                p.id
            );
        }
        assert!(preset_by_id("zenz-small").is_some());
        assert!(preset_by_id("nope").is_none());
    }

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

    /// 受けるモデルは表の行から作る。行の数と ID の重なりを見つける（足すときの取り違え）
    #[test]
    fn known_models_follow_the_table_and_ids_are_unique() {
        assert_eq!(KNOWN_MODELS.len(), MODEL_PRESETS.len());
        for (k, p) in KNOWN_MODELS.iter().zip(MODEL_PRESETS) {
            assert_eq!(
                (k.file_name, k.sha256, k.size),
                (p.file_name, p.sha256, p.size)
            );
        }
        for (i, p) in MODEL_PRESETS.iter().enumerate() {
            assert!(MODEL_PRESETS[..i].iter().all(|q| q.id != p.id), "{}", p.id);
            assert!(
                MODEL_PRESETS[..i]
                    .iter()
                    .all(|q| q.file_name != p.file_name)
            );
        }
    }

    #[test]
    fn known_sha256_are_well_formed() {
        for m in KNOWN_MODELS {
            assert!(crate::sha256::is_sha256_hex(m.sha256), "{}", m.file_name);
        }
    }
}
