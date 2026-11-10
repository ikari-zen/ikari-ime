//! LM のモデルを読む前の照合（#270）。小さなダミーのファイルで、読まれる・読まれないを確かめる

use super::*;
use crate::model_check::{ModelReject, select_lm_model};
use ikari_resource::{KnownModel, sha256_reader};

const NAME: &str = "dummy-model.gguf";
const GOOD: &[u8] = b"pretend-gguf-bytes";

fn known() -> Vec<KnownModel> {
    vec![KnownModel {
        file_name: NAME,
        sha256: Box::leak(sha256_reader(GOOD).unwrap().into_boxed_str()),
        size: GOOD.len() as u64,
    }]
}

fn model_at(tag: &str, bytes: &[u8]) -> (PathBuf, PathBuf) {
    let dir = temp_dir(tag);
    let path = dir.join(NAME);
    std::fs::write(&path, bytes).unwrap();
    (dir, path)
}

#[test]
fn correct_model_is_selected() {
    let (dir, path) = model_at("model-ok", GOOD);
    let (model, reject) = select_lm_model(true, &path, NAME, &known());
    assert_eq!(model.as_deref(), Some(path.as_path()));
    assert_eq!(reject, None);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn broken_model_of_same_size_is_not_selected() {
    let mut bytes = GOOD.to_vec();
    bytes[0] ^= 0xff;
    let (dir, path) = model_at("model-broken", &bytes);
    let (model, reject) = select_lm_model(true, &path, NAME, &known());
    assert_eq!(model, None);
    assert_eq!(reject, Some(ModelReject::Sha256));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn model_of_unknown_size_is_not_selected() {
    let (dir, path) = model_at("model-size", b"short");
    let (model, reject) = select_lm_model(true, &path, NAME, &known());
    assert_eq!(model, None);
    assert_eq!(reject, Some(ModelReject::Size));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn model_with_unknown_name_is_not_selected() {
    let (dir, path) = model_at("model-name", GOOD);
    let (model, reject) = select_lm_model(true, &path, "other.gguf", &known());
    assert_eq!(model, None);
    assert_eq!(reject, Some(ModelReject::UnknownName));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn nothing_is_checked_when_lm_is_not_wanted_or_the_file_is_absent() {
    let (dir, path) = model_at("model-skip", b"not checked");
    assert_eq!(select_lm_model(false, &path, NAME, &known()), (None, None));
    let gone = dir.join("nothing.gguf");
    assert_eq!(select_lm_model(true, &gone, NAME, &known()), (None, None));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn rejection_is_logged_with_the_file_name_and_a_reason_only() {
    let dir = temp_dir("model-log");
    let mock = MockLm::new(Duration::ZERO);
    let e = Engine::from_parts(EngineParts {
        stack: mock_stack(&mock, None),
        rebuild: None,
        data_dir: Some(dir.clone()),
        lm_requested: true,
        lm_budget: LmBudget::default(),
        convert_wait: DEFAULT_CONVERT_WAIT,
        lock: None,
    });
    e.inner.record_lm_reject(NAME, ModelReject::Sha256);
    e.inner.log.flush_for_test();
    let log = std::fs::read_to_string(dir.join("logs").join("engine.log")).unwrap();
    let line = log
        .lines()
        .find(|l| l.split_whitespace().nth(1) == Some("lm_reject"))
        .expect("an lm_reject line");
    assert!(
        line.contains("file=dummy-model.gguf reason=sha256"),
        "{line}"
    );
    let _ = std::fs::remove_dir_all(dir);
}
