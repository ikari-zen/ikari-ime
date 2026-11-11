//! 設定の読み元（ファイルか文字列）と、キー割り当てとローマ字表の読み込み（今の `SettingsStore` の殻の側と `KeymapLoader`）。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::keymap::Keymap;
use crate::machine::RomajiSource;
use crate::settings::{
    SettingsChange, SettingsChanges, SettingsEntry, SettingsStore, preset_name, rewrite, validate,
};
use crate::types::SettingsSnapshot;

/// ユーザーのローマ字表のファイル名（設定のファイルと同じ場所。今の `AppContext.romajiFileName`）。
pub const ROMAJI_FILE_NAME: &str = "romaji.tsv";
/// 同梱のキー割り当ての置き場所（`<resources>/keymap/<preset>.tsv`。今の `KeymapLoader`）。
const KEYMAP_DIRECTORY: &str = "keymap";
const KEYMAP_EXTENSION: &str = "tsv";
/// プリセットの資源が見つからないときの最低限の割り当て（F6〜F10。入力は止めない）。
const FUNCTION_KEYS_ONLY_PRESET: &str = "f6\tconvert.hiragana\nf7\tconvert.fullwidth_katakana\nf8\tconvert.halfwidth_katakana\nf9\tconvert.fullwidth_alnum\nf10\tconvert.halfwidth_alnum\n";

/// 書き込みの失敗（ログには種類だけを書く）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateFailed {
    Rejected,
    WriteFailed,
}

/// 設定の今の値と版。版は値が変わるたびに増える。
pub(crate) enum SettingsBackend {
    File(SettingsStore),
    /// シナリオとテスト。書き込みはメモリの文字列に当てる（`settings::rewrite` は同じ）
    Inline {
        text: String,
        snapshot: Arc<SettingsSnapshot>,
        version: u64,
    },
}

impl SettingsBackend {
    pub(crate) fn file(path: PathBuf) -> Self {
        let (store, _issues) = SettingsStore::open(path);
        Self::File(store)
    }

    pub(crate) fn inline(text: String) -> Self {
        let snapshot = validate(&text)
            .map(|validation| validation.snapshot)
            .unwrap_or_default();
        Self::Inline {
            text,
            snapshot: Arc::new(snapshot),
            version: 0,
        }
    }

    pub(crate) fn snapshot(&self) -> &SettingsSnapshot {
        match self {
            Self::File(store) => store.snapshot(),
            Self::Inline { snapshot, .. } => snapshot,
        }
    }

    /// 今の設定の共有。複製しない（値が変わるまで同じもの）
    pub(crate) fn shared(&self) -> Arc<SettingsSnapshot> {
        match self {
            Self::File(store) => store.shared(),
            Self::Inline { snapshot, .. } => Arc::clone(snapshot),
        }
    }

    pub(crate) fn version(&self) -> u64 {
        match self {
            Self::File(store) => store.version(),
            Self::Inline { version, .. } => *version,
        }
    }

    /// ファイルが変わっていれば読み直す（文字列のときは何もしない）。
    pub(crate) fn reload_if_changed(&mut self) -> Option<SettingsChange> {
        match self {
            Self::File(store) => store.reload_if_file_changed().change,
            Self::Inline { .. } => None,
        }
    }

    /// 値を書き（`set`）、行を消す（`reset`。既定に戻す）。今の `SettingsStore.update(set:reset:)`。
    pub(crate) fn update(
        &mut self,
        set: Vec<SettingsEntry>,
        reset: Vec<String>,
    ) -> Result<Option<SettingsChange>, UpdateFailed> {
        match self {
            Self::File(store) => store
                .update(set, reset)
                .map(|updated| updated.change)
                .map_err(|error| match error {
                    crate::settings::StoreError::Rejected { .. } => UpdateFailed::Rejected,
                    _ => UpdateFailed::WriteFailed,
                }),
            Self::Inline {
                text,
                snapshot,
                version,
            } => {
                let changes = SettingsChanges { set, reset };
                let updated = rewrite(text, &changes).map_err(|_| UpdateFailed::Rejected)?;
                let next = validate(&updated)
                    .map(|validation| validation.snapshot)
                    .map_err(|_| UpdateFailed::Rejected)?;
                *text = updated;
                if next == **snapshot {
                    return Ok(None);
                }
                let old = std::mem::replace(snapshot, Arc::new(next.clone()));
                *version += 1;
                Ok(Some(SettingsChange {
                    old: Arc::unwrap_or_clone(old),
                    new: next,
                }))
            }
        }
    }

    /// ユーザーのローマ字表（設定のファイルと同じ場所の `romaji.tsv`。なければ同梱の表）。
    pub(crate) fn romaji_source(&self) -> RomajiSource {
        match self {
            Self::File(store) => store
                .path()
                .parent()
                .map(|dir| dir.join(ROMAJI_FILE_NAME))
                .and_then(|path| ikari_textfile::read_to_string(path).ok())
                .map_or_else(RomajiSource::builtin, |text| RomajiSource::custom(&text)),
            Self::Inline { .. } => RomajiSource::builtin(),
        }
    }
}

/// プリセットの資源と設定の上書きから、キー割り当てを作る。資源が読めない・壊れているときは F キーだけ（入力は止めない）。
pub(crate) fn load_keymap(resources_dir: &Path, settings: &SettingsSnapshot) -> Arc<Keymap> {
    let path = resources_dir.join(KEYMAP_DIRECTORY).join(format!(
        "{}.{KEYMAP_EXTENSION}",
        preset_name(settings.keymap_preset)
    ));
    let preset =
        ikari_textfile::read_to_string(path).unwrap_or_else(|_| FUNCTION_KEYS_ONLY_PRESET.into());
    let keymap = Keymap::from_settings(&preset, &settings.keymap_overrides)
        .or_else(|_| Keymap::from_settings(FUNCTION_KEYS_ONLY_PRESET, &settings.keymap_overrides))
        .unwrap_or_default();
    Arc::new(keymap)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::default_snapshot;
    use ikari_textfile::testing::read_variants;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ikari-backend-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn keymap_preset_reads_lf_crlf_and_bom_files() {
        let dir = temp_dir("keymap");
        let settings = default_snapshot();
        let keymap_dir = dir.join(KEYMAP_DIRECTORY);
        std::fs::create_dir_all(&keymap_dir).unwrap();
        let path = keymap_dir.join(format!(
            "{}.{KEYMAP_EXTENSION}",
            preset_name(settings.keymap_preset)
        ));
        let lf = "# preset\nf6\tconvert.hiragana\nf7\tconvert.fullwidth_katakana\n";
        std::fs::write(&path, lf).unwrap();
        let expected = load_keymap(&dir, &settings);
        // 資源が読めなかったときの F キーだけの割り当てとは違う
        let fallback =
            Keymap::from_settings(FUNCTION_KEYS_ONLY_PRESET, &settings.keymap_overrides).unwrap();
        assert_ne!(*expected, fallback);
        for (v, text) in read_variants(lf) {
            std::fs::write(&path, text).unwrap();
            assert_eq!(*load_keymap(&dir, &settings), *expected, "{v:?}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn romaji_table_reads_lf_crlf_and_bom_files() {
        let dir = temp_dir("romaji");
        let settings_path = dir.join("settings.tsv");
        for (v, text) in read_variants("# my table\nka\tか\n") {
            std::fs::write(dir.join(ROMAJI_FILE_NAME), text).unwrap();
            let backend = SettingsBackend::file(settings_path.clone());
            // 読めない表は「使えない」になる（BOM が残ると最初の行が不正になる）
            assert_eq!(
                format!("{:?}", backend.romaji_source()),
                "RomajiSource",
                "{v:?}"
            );
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
