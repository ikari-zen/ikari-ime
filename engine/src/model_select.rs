//! LM のモデルの解決（ADR 0060 §7、ADR 0072 §4・§12.8 の 1）。**契約（第 4 弾）。中身はレーン D-4**。
//!
//! 起動のときに、どのモデルを、どの檻の段で読むかを決める。今は、今までどおり固定のプリセット
//! （`models/` の `MODEL_FILE_NAME` を既知の表と照合する）だけを返す（挙動は変えていない）。
//!
//! D-4 で足すもの: 設定 `lm.model` の解決、`models/MANIFEST.json` の一覧、自由指定のモデル（`origin = user`）、
//! `models/.loading` の印、自由指定は段 2 のある OS でだけ選べること。
//!
//! 判断の置き場所: Rust（エンジン）。

use std::path::{Path, PathBuf};

use ikari_stack::HostCage;

use crate::model_check::{self, ModelReject};
use crate::settings::Settings;
use crate::{MODEL_FILE_NAME, MODELS_DIR_NAME};

/// モデルの出どころ（檻の段の決め手）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModelOrigin {
    /// アプリに埋めた表の sha256 で確かめたもの
    Preset,
    /// 利用者が指定したもの（ファイル、URL。ADR 0060 の段 4・5）
    #[allow(dead_code)] // D-4 で使う
    User,
}

/// 解決したモデル
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedModel {
    /// 読むモデル。None なら LM なし
    pub(crate) path: Option<PathBuf>,
    /// ログの `lm_reject` に書くファイルの名前（パスは書かない）
    pub(crate) file_name: String,
    /// 通さなかった理由（通したとき、LM を使わないときは None）
    pub(crate) reject: Option<ModelReject>,
    pub(crate) origin: ModelOrigin,
}

/// 起動のときに読むモデルを決める（契約のスタブ: 今は固定のプリセットだけ。今までと同じ照合）
pub(crate) fn resolve_model(
    lm_requested: bool,
    data_dir: &Path,
    settings: &Settings,
) -> ResolvedModel {
    let _ = settings; // D-4: `lm.model` を読む
    let model = data_dir.join(MODELS_DIR_NAME).join(MODEL_FILE_NAME);
    // 読む前に、既知のモデルの表と照合する。通らなければ LM なしで起動する（理由はログに書く）
    // （LM のないビルドでも照合して記録する。読むのは LM のあるビルドだけ）
    let (path, reject) = model_check::select_lm_model(
        lm_requested,
        &model,
        MODEL_FILE_NAME,
        ikari_resource::KNOWN_MODELS,
    );
    ResolvedModel {
        path,
        file_name: MODEL_FILE_NAME.to_owned(),
        reject,
        origin: ModelOrigin::Preset,
    }
}

/// 出どころから、求める檻の段を決める（ADR 0072 §12.8 の 1。プリセットは段 1（`PreferAppContainer` に替えるかは
/// #424 のあと）、自由指定は段 2 だけ）
pub(crate) fn cage_for(origin: ModelOrigin) -> HostCage {
    match origin {
        ModelOrigin::Preset => HostCage::Standard,
        ModelOrigin::User => HostCage::RequireAppContainer,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_models_require_level_2_and_presets_stay_at_level_1() {
        assert_eq!(cage_for(ModelOrigin::Preset), HostCage::Standard);
        assert_eq!(cage_for(ModelOrigin::User), HostCage::RequireAppContainer);
    }

    #[test]
    #[ignore = "D-4: 設定 lm.model で、models/MANIFEST.json にある自由指定のモデルを選べる"]
    fn lm_model_setting_selects_a_user_model() {
        unimplemented!("D-4")
    }

    #[test]
    #[ignore = "D-4: models/.loading が残っていたら、選択を既定に戻す"]
    fn a_leftover_loading_mark_falls_back_to_the_default() {
        unimplemented!("D-4")
    }

    #[test]
    #[ignore = "D-4: 段 2 の無い OS では、自由指定のモデルを選ばない（LM なし）"]
    fn user_models_are_not_used_without_level_2() {
        unimplemented!("D-4")
    }
}
