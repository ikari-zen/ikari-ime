//! LM のモデル（`models/` の GGUF）を、読む前に既知の表と照合する（#270）。
//!
//! 通らなかったもの（未知の名前、大きさ違い、sha256 違い、読めない）は、LM なしで起動する
//! （変換は辞書だけで動く）。理由は固定の語で `lm_reject` の行に書く（NFR-19: ファイル名と理由の種類だけ）。
//! 判断の置き場所は Rust（殻は触らない）。

use std::path::{Path, PathBuf};

use ikari_resource::{KnownModel, ResourceError, verify_model_file};

/// モデルを通さなかった理由（ログの語）。中身の違いまでは書かない
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModelReject {
    /// 表にない名前
    UnknownName,
    /// 大きさが表と違う（sha256 は取っていない）
    Size,
    /// sha256 が表と違う
    Sha256,
    /// 普通のファイルでない、または読めない
    Unreadable,
}

impl ModelReject {
    pub(crate) fn as_word(self) -> &'static str {
        match self {
            ModelReject::UnknownName => "unknown_name",
            ModelReject::Size => "size",
            ModelReject::Sha256 => "sha256",
            ModelReject::Unreadable => "unreadable",
        }
    }

    fn from_error(e: &ResourceError) -> ModelReject {
        match e {
            ResourceError::UnknownModel { .. } => ModelReject::UnknownName,
            ResourceError::Truncated { .. } | ResourceError::SizeMismatch { .. } => {
                ModelReject::Size
            }
            ResourceError::Sha256Mismatch { .. } => ModelReject::Sha256,
            _ => ModelReject::Unreadable,
        }
    }
}

/// `model` を読んでよいか。通れば `Some(model)`、通らなければ `None` と理由
pub(crate) fn vet_model(
    model: PathBuf,
    file_name: &str,
    known: &[KnownModel],
) -> (Option<PathBuf>, Option<ModelReject>) {
    match verify_model_file(&model, file_name, known) {
        Ok(()) => (Some(model), None),
        Err(e) => (None, Some(ModelReject::from_error(&e))),
    }
}

/// 起動のときに使うモデルを決める。LM を使わない設定・このビルドに LM がない・ファイルがないときは、
/// 照合せず（ファイルを読まず）`None`。
pub(crate) fn select_lm_model(
    lm_wanted: bool,
    model: &Path,
    file_name: &str,
    known: &[KnownModel],
) -> (Option<PathBuf>, Option<ModelReject>) {
    if !lm_wanted {
        return (None, None);
    }
    // 開く前に種類を見る（FIFO などを open して止まらない）。なければ黙る。あるのに普通のファイルでなければ拒む
    match std::fs::metadata(model) {
        Ok(meta) if meta.is_file() => vet_model(model.to_owned(), file_name, known),
        Ok(_) => (None, Some(ModelReject::Unreadable)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (None, None),
        Err(_) => (None, Some(ModelReject::Unreadable)),
    }
}
