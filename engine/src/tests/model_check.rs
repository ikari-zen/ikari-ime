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

// ---- 普通のファイルでないもの、シンボリックリンク ----

// Windows では symlink の作成に特権（開発者モード）が要るので、Unix だけで確かめる
#[cfg(unix)]
#[test]
fn symlink_to_correct_model_passes_and_to_broken_model_is_rejected() {
    let (dir, good) = model_at("model-link", GOOD);
    let link = dir.join("link.gguf");
    std::os::unix::fs::symlink(&good, &link).unwrap();
    let (model, reject) = select_lm_model(true, &link, NAME, &known());
    assert_eq!(model.as_deref(), Some(link.as_path()));
    assert_eq!(reject, None);

    let mut bytes = GOOD.to_vec();
    bytes[0] ^= 0xff;
    std::fs::write(&good, &bytes).unwrap();
    let (model, reject) = select_lm_model(true, &link, NAME, &known());
    assert_eq!(model, None);
    assert_eq!(reject, Some(ModelReject::Sha256));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn directory_and_fifo_are_rejected_without_blocking() {
    let dir = temp_dir("model-special");
    let sub = dir.join("sub.gguf");
    std::fs::create_dir(&sub).unwrap();
    assert_eq!(
        select_lm_model(true, &sub, NAME, &known()),
        (None, Some(ModelReject::Unreadable))
    );
    // FIFO は Unix にしかない（Windows の CI の Git の mkfifo は普通のファイルを作ってしまう）
    #[cfg(unix)]
    {
        let fifo = dir.join("fifo.gguf");
        let status = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("run mkfifo");
        assert!(status.success());
        // open すると書き手が来るまで止まる。止まらずに返ること（種類を先に見る）
        assert_eq!(
            select_lm_model(true, &fifo, NAME, &known()),
            (None, Some(ModelReject::Unreadable))
        );
    }
    let _ = std::fs::remove_dir_all(dir);
}

// ---- Engine::open を通す（LM のないビルドでも動く） ----

/// 辞書を置き、`models/` に `place` で指定したものを置いて、LM 有効で開く
fn open_with_model(tag: &str, place: impl FnOnce(&Path)) -> (Engine, PathBuf) {
    let dir = temp_dir(tag);
    let dict = UserDir::new(&dir).dict_dir();
    std::fs::create_dir_all(&dict).unwrap();
    for entry in std::fs::read_dir(fixture_dict_dir()).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), dict.join(entry.file_name())).unwrap();
    }
    let models = dir.join(MODELS_DIR_NAME);
    std::fs::create_dir_all(&models).unwrap();
    place(&models.join(MODEL_FILE_NAME));
    let e = Engine::open(&EngineConfig {
        data_dir: dir.to_string_lossy().into_owned(),
        enable_lm: true,
        idle_unload_secs: 300,
    })
    .expect("open without a usable model");
    (e, dir)
}

fn reject_lines(e: &Engine, dir: &Path) -> Vec<String> {
    e.inner.log.flush_for_test();
    let log = std::fs::read_to_string(dir.join("logs").join("engine.log")).unwrap_or_default();
    log.lines()
        .filter(|l| l.split_whitespace().nth(1) == Some("lm_reject"))
        .map(str::to_owned)
        .collect()
}

#[test]
fn open_with_an_unknown_model_starts_without_lm_and_converts() {
    let (e, dir) = open_with_model("open-unknown", |p| {
        std::fs::write(p, b"not a real model").unwrap();
    });
    let envelope::Body::HelloReply(h) = send(&e, envelope::Body::Hello(pb::Hello::default()))
    else {
        panic!();
    };
    assert_eq!(h.lm, LmState::LmFailed as i32);
    begin(&e, 1, false);
    let r = convert(&e, 1, "この", 1);
    assert!(!r.candidates.is_empty());
    let lines = reject_lines(&e, &dir);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].contains(&format!("file={MODEL_FILE_NAME} reason=size")),
        "{}",
        lines[0]
    );
    drop(e);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn open_without_a_model_file_writes_no_reject_line() {
    let (e, dir) = open_with_model("open-nomodel", |_| {});
    assert!(reject_lines(&e, &dir).is_empty());
    drop(e);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn open_with_a_directory_in_place_of_the_model_logs_unreadable() {
    let (e, dir) = open_with_model("open-dirmodel", |p| std::fs::create_dir(p).unwrap());
    let lines = reject_lines(&e, &dir);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("reason=unreadable"), "{}", lines[0]);
    drop(e);
    let _ = std::fs::remove_dir_all(dir);
}
