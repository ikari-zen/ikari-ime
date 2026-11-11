//! engine-host の結合テストで共有する、テスト用の辞書とエンジン（`testdata/sudachi-mini`）。

use std::path::Path;
use std::sync::OnceLock;

use ikari_core::user::UserDir;
use ikari_engine::{Engine, EngineConfig};

/// fixture の辞書をビルドした `dict/`。テスト全体で 1 回だけ作る。
fn fixture_dict_dir() -> &'static Path {
    static DICT: OnceLock<tempfile::TempDir> = OnceLock::new();
    DICT.get_or_init(|| {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/sudachi-mini");
        let out = tempfile::tempdir().unwrap();
        ikari_dict_builder::build(&ikari_dict_builder::BuildConfig {
            lex: vec![src.join("lex.csv")],
            matrix: src.join("matrix.def"),
            out_dir: out.path().to_owned(),
            unk_pos: "名詞,普通名詞,一般,*,*,*".to_owned(),
            unk_cost: 10_000,
            label: "engine-host-test".to_owned(),
        })
        .expect("build the fixture dictionary");
        out
    })
    .path()
}

/// 辞書を置いたデータのディレクトリで、LM なしのエンジンを開く。
pub fn open_engine() -> (Engine, tempfile::TempDir) {
    let data = tempfile::tempdir().unwrap();
    let dict = UserDir::new(data.path()).dict_dir();
    std::fs::create_dir_all(&dict).unwrap();
    for entry in std::fs::read_dir(fixture_dict_dir()).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), dict.join(entry.file_name())).unwrap();
    }
    let engine = Engine::open(&EngineConfig {
        data_dir: data.path().to_string_lossy().into_owned(),
        enable_lm: false,
        idle_unload_secs: 300,
    })
    .expect("open the engine");
    (engine, data)
}
