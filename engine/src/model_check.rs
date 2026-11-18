//! LM のモデル（`models/` の GGUF）を、読む前に既知の表と照合する（#270）。
//!
//! 通らなかったもの（未知の名前、大きさ違い、sha256 違い、読めない）は、LM なしで起動する
//! （変換は辞書だけで動く）。理由は固定の語で `lm_reject` の行に書く（NFR-19: ファイル名と理由の種類だけ）。
//! 判断の置き場所は Rust（殻は触らない）。
//!
//! 照合は、開いたハンドル（[`LmModelHandle`]）で行い、そのハンドルをエンジンの寿命の間持って、子に渡す
//! （照合した物と子が読む物を同じにする。C-03、FC-3）。

use std::path::Path;

use ikari_resource::{KnownModel, ResourceError, verify_model_open};
use ikari_stack::LmModelHandle;

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
    /// 自由指定のモデルの一覧（`models/MANIFEST.json`）が壊れている（既定のモデルに戻した。D-4）
    Manifest,
    /// 自由指定のモデルに要る段 2 が、この構成に無い（LM なし。D-4）
    NoCage,
    /// 前の起動で、このモデルの読み込みが終わらなかった（既定のモデルに戻した。D-4）
    LoadingMark,
}

impl ModelReject {
    pub(crate) fn as_word(self) -> &'static str {
        match self {
            ModelReject::UnknownName => "unknown_name",
            ModelReject::Size => "size",
            ModelReject::Sha256 => "sha256",
            ModelReject::Unreadable => "unreadable",
            ModelReject::Manifest => "manifest",
            ModelReject::NoCage => "no_cage",
            ModelReject::LoadingMark => "loading_mark",
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

/// `model` を読んでよいか。開いたハンドルで照合し、通ればそのハンドル、通らなければ `None` と理由
pub(crate) fn vet_model(
    model: &Path,
    file_name: &str,
    known: &[KnownModel],
) -> (Option<LmModelHandle>, Option<ModelReject>) {
    // 表にない名前は開かない
    if !known.iter().any(|m| m.file_name == file_name) {
        return (None, Some(ModelReject::UnknownName));
    }
    let handle = match LmModelHandle::open(model) {
        Ok(handle) => handle,
        Err(_) => return (None, Some(ModelReject::Unreadable)),
    };
    match verify_model_open(handle.file(), model, file_name, known) {
        Ok(()) => (Some(handle), None),
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
) -> (Option<LmModelHandle>, Option<ModelReject>) {
    if !lm_wanted {
        return (None, None);
    }
    // 開く前に種類を見る（FIFO などを open して止まらない）。なければ黙る。あるのに普通のファイルでなければ拒む
    match std::fs::metadata(model) {
        Ok(meta) if meta.is_file() => vet_model(model, file_name, known),
        Ok(_) => (None, Some(ModelReject::Unreadable)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (None, None),
        Err(_) => (None, Some(ModelReject::Unreadable)),
    }
}
