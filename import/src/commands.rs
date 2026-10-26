//! `ikari ingest` の各コマンドの処理（ファイルの読み書きまで）。表示は CLI が受け持つ。

use std::fmt;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use ikari_core::UserDataError;
use ikari_core::user::{
    IngestCandidates, IngestSource, IngestSources, LayerSel, LearningStore, Profile, UserDir,
    today_unix_day,
};

use crate::io::{ReadError, read_ingest_file};
use crate::pipeline::{Env, IngestOutcome, ingest, suggest_profile};

#[derive(Debug)]
pub enum ImportError {
    Read(ReadError),
    Data(UserDataError),
    /// 同じパスがすでに取り込み済み
    AlreadyIngested {
        id: u32,
    },
    NoSuchSource(u32),
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImportError::Read(e) => write!(f, "{e}"),
            ImportError::Data(e) => write!(f, "{e}"),
            ImportError::AlreadyIngested { id } => write!(
                f,
                "このファイルは取り込み済みです（id={id}）。やり直すときは `ikari ingest remove {id}` で消してから取り込んでください"
            ),
            ImportError::NoSuchSource(id) => write!(f, "取り込み元 id={id} はありません"),
        }
    }
}

impl std::error::Error for ImportError {}

impl From<ReadError> for ImportError {
    fn from(e: ReadError) -> Self {
        ImportError::Read(e)
    }
}

impl From<UserDataError> for ImportError {
    fn from(e: UserDataError) -> Self {
        ImportError::Data(e)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct RunOptions {
    /// 何も保存しない
    pub dry_run: bool,
    /// 分野の推定を `profile.tsv` に書く
    pub apply_profile: bool,
    /// UNIX 日（学習の `last_day`）
    pub today: u32,
    /// UNIX 秒（台帳の取り込み日時）
    pub now_secs: u64,
}

impl RunOptions {
    pub fn now(dry_run: bool, apply_profile: bool) -> Self {
        Self {
            dry_run,
            apply_profile,
            today: today_unix_day(),
            now_secs: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
        }
    }
}

#[derive(Debug)]
pub struct RunReport {
    /// 台帳に付けた番号（dry-run では None）
    pub source_id: Option<u32>,
    pub outcome: IngestOutcome,
    /// `profile.tsv` に書いた分野（`apply_profile` が真で、書いたときだけ）
    pub applied_profile: Option<Vec<(String, ikari_core::ProfileWeight)>>,
}

/// 取り込みファイルを検証、アライメントして、学習、登録候補、台帳を更新する。
///
/// 検証（スキーマ、ヘッダ）に失敗したら何も変えない。`store` は呼び出し側が開いて渡す
/// （文法を結びつけた状態で）。保存は最後にまとめて行う。
pub fn run_file(
    dir: &UserDir,
    path: &Path,
    env: &Env<'_>,
    store: &LearningStore,
    opts: RunOptions,
) -> Result<RunReport, ImportError> {
    let file = read_ingest_file(path)?;
    let path_text = absolute_path_text(path);

    let mut sources = IngestSources::open(&dir.ingest_sources_tsv())?;
    if let Some(existing) = sources.list().iter().find(|s| s.path == path_text) {
        return Err(ImportError::AlreadyIngested { id: existing.id });
    }
    let source_id = sources.next_id();

    let outcome = ingest(
        &file.parsed,
        env,
        (!opts.dry_run).then_some(store),
        source_id,
        opts.today,
    );
    if opts.dry_run {
        return Ok(RunReport {
            source_id: None,
            outcome,
            applied_profile: None,
        });
    }

    let mut candidates = IngestCandidates::open(&dir.ingest_candidates_tsv())?;
    for (reading, surface, n) in &outcome.candidates {
        candidates.add(source_id, reading, surface, *n);
    }
    sources.add(IngestSource {
        id: source_id,
        path: path_text,
        imported_at: opts.now_secs,
        fnv64: file.fnv64,
        prompt_version: outcome.prompt_version.clone(),
        sentences_ok: outcome.aligned,
        sentences_dropped: outcome.sentences_dropped(),
    })?;

    let mut applied_profile = None;
    if opts.apply_profile {
        let suggestion = suggest_profile(&outcome.domains);
        if !suggestion.is_empty() {
            Profile::from_settings(&suggestion)?.save(dir)?;
            applied_profile = Some(suggestion);
        }
    }

    // 台帳を最後に書く: 途中で失敗しても、台帳にない取り込み元の記録が残るだけで、
    // 台帳にあるのに記録がない状態にはならない（`ingest reset` で消せる）
    store.save()?;
    candidates.save(&dir.ingest_candidates_tsv())?;
    sources.save(&dir.ingest_sources_tsv())?;
    Ok(RunReport {
        source_id: Some(source_id),
        outcome,
        applied_profile,
    })
}

/// 取り込み元 1 件を消す: その `initial` 記録と登録候補も消す（NFR-26）。取り込みファイルは消さない。
pub fn remove_source(dir: &UserDir, store: &LearningStore, id: u32) -> Result<(), ImportError> {
    let mut sources = IngestSources::open(&dir.ingest_sources_tsv())?;
    if !sources.remove(id) {
        return Err(ImportError::NoSuchSource(id));
    }
    let mut candidates = IngestCandidates::open(&dir.ingest_candidates_tsv())?;
    candidates.remove_source(id);
    store.remove_source(id);
    store.save()?;
    candidates.save(&dir.ingest_candidates_tsv())?;
    sources.save(&dir.ingest_sources_tsv())?;
    Ok(())
}

/// 取り込みをすべてやり直せる状態に戻す: `initial` 層、台帳、登録候補を空にする。`usage` 層は残す。
pub fn reset_all(dir: &UserDir, store: &LearningStore) -> Result<(), ImportError> {
    store.reset(LayerSel::Initial);
    store.save()?;
    IngestCandidates::default().save(&dir.ingest_candidates_tsv())?;
    let mut sources = IngestSources::open(&dir.ingest_sources_tsv())?;
    sources.clear();
    sources.save(&dir.ingest_sources_tsv())?;
    Ok(())
}

/// 台帳に残すパス（相対パスは絶対パスにする。参照先を後から開き直せるように）。
fn absolute_path_text(path: &Path) -> String {
    std::path::absolute(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::Arc;

    use ikari_core::user::LayerSel;

    use super::*;
    use crate::holdout::load_holdout;
    use crate::test_support::*;

    const TODAY: u32 = 20_000;
    const NOW: u64 = 1_760_000_000;

    struct Setup {
        _tmp: tempfile::TempDir,
        dir: UserDir,
        file: std::path::PathBuf,
        m: Mini,
        store: Arc<LearningStore>,
    }

    fn setup(sentences: &[(String, String)]) -> Setup {
        let tmp = tempfile::tempdir().unwrap();
        let dir = UserDir::new(tmp.path().join("user"));
        let file = tmp.path().join("doc.jsonl");
        fs::write(&file, jsonl(sentences)).unwrap();
        let m = mini();
        let store = LearningStore::open(&dir.learning_tsv()).unwrap();
        store.attach_grammar(Arc::new(mini().grammar));
        Setup {
            _tmp: tmp,
            dir,
            file,
            m,
            store: Arc::new(store),
        }
    }

    fn env(m: &Mini) -> Env<'_> {
        Env {
            dict: &m.dict,
            grammar: &m.grammar,
            vectors: None,
        }
    }

    fn opts(dry_run: bool, apply_profile: bool) -> RunOptions {
        RunOptions {
            dry_run,
            apply_profile,
            today: TODAY,
            now_secs: NOW,
        }
    }

    fn sample() -> Vec<(String, String)> {
        vec![
            training("その話を初めて知った", "そのはなしをはじめてしった"),
            training("怒里を登録する", "いかりをとうろくする"),
            holdout("単語を始めて登録する", "たんごをはじめてとうろくする"),
            training("辞書を", "じしょに"),
        ]
    }

    #[test]
    fn run_records_learning_ledger_and_candidates() {
        let s = setup(&sample());
        let report = run_file(&s.dir, &s.file, &env(&s.m), &s.store, opts(false, false)).unwrap();
        assert_eq!(report.source_id, Some(1));
        let o = &report.outcome;
        assert_eq!((o.learned, o.with_unknown, o.holdout), (1, 1, 1));
        assert_eq!(o.sentences_dropped(), 1);

        let ledger = IngestSources::open(&s.dir.ingest_sources_tsv()).unwrap();
        let src = &ledger.list()[0];
        assert_eq!(src.id, 1);
        assert_eq!(src.prompt_version, "ingest-prompt-v1");
        assert_eq!(src.sentences_ok, 3);
        assert_eq!(src.sentences_dropped, 1);
        assert_eq!(src.imported_at, NOW);
        assert!(std::path::Path::new(&src.path).is_absolute());
        let cands = IngestCandidates::open(&s.dir.ingest_candidates_tsv()).unwrap();
        assert_eq!(cands.list().len(), 1);
        assert!(s.dir.learning_tsv().exists());
        assert!(
            !LearningStore::open(&s.dir.learning_tsv())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn dry_run_writes_nothing() {
        let s = setup(&sample());
        let report = run_file(&s.dir, &s.file, &env(&s.m), &s.store, opts(true, true)).unwrap();
        assert_eq!(report.source_id, None);
        assert!(report.applied_profile.is_none());
        assert!(!s.dir.root().exists());
        assert!(s.store.is_empty());
    }

    #[test]
    fn invalid_files_change_nothing() {
        let s = setup(&sample());
        for bad in [
            String::new(),
            "not json\n".to_string(),
            format!(
                "{}\n",
                HEADER_LINE.replace("ingest-prompt-v1", "ingest-prompt-v0")
            ),
            HEADER_LINE.replace("\"version\":1", "\"version\":9"),
        ] {
            fs::write(&s.file, &bad).unwrap();
            let err = run_file(&s.dir, &s.file, &env(&s.m), &s.store, opts(false, false));
            assert!(matches!(err, Err(ImportError::Read(_))), "{bad:?}");
            assert!(!s.dir.root().exists());
            assert!(s.store.is_empty());
        }
        let missing = s.file.with_file_name("none.jsonl");
        assert!(run_file(&s.dir, &missing, &env(&s.m), &s.store, opts(false, false)).is_err());
    }

    #[test]
    fn the_same_path_cannot_be_ingested_twice() {
        let s = setup(&sample());
        run_file(&s.dir, &s.file, &env(&s.m), &s.store, opts(false, false)).unwrap();
        let err = run_file(&s.dir, &s.file, &env(&s.m), &s.store, opts(false, false)).unwrap_err();
        assert!(matches!(err, ImportError::AlreadyIngested { id: 1 }));
    }

    #[test]
    fn remove_deletes_that_sources_records_and_candidates_only() {
        let s = setup(&sample());
        run_file(&s.dir, &s.file, &env(&s.m), &s.store, opts(false, false)).unwrap();
        let second = s.file.with_file_name("second.jsonl");
        fs::write(&second, jsonl(&[training("ユーザ辞書", "ゆーざじしょ")])).unwrap();
        let r2 = run_file(&s.dir, &second, &env(&s.m), &s.store, opts(false, false)).unwrap();
        assert_eq!(r2.source_id, Some(2));
        assert!(s.store.records().iter().any(|r| r.source == Some(1)));
        assert!(s.store.records().iter().any(|r| r.source == Some(2)));

        remove_source(&s.dir, &s.store, 1).unwrap();
        assert!(s.store.records().iter().all(|r| r.source == Some(2)));
        let ledger = IngestSources::open(&s.dir.ingest_sources_tsv()).unwrap();
        assert_eq!(ledger.list().len(), 1);
        assert!(
            IngestCandidates::open(&s.dir.ingest_candidates_tsv())
                .unwrap()
                .is_empty()
        );
        // 保存されたファイルにも反映されている
        let reopened = LearningStore::open(&s.dir.learning_tsv()).unwrap();
        assert!(reopened.records().iter().all(|r| r.source == Some(2)));
        assert!(matches!(
            remove_source(&s.dir, &s.store, 1),
            Err(ImportError::NoSuchSource(1))
        ));
        // 取り込みファイルそのものは消さない
        assert!(s.file.exists());
        // 消した番号は再利用しない
        let third = s.file.with_file_name("third.jsonl");
        fs::write(&third, jsonl(&[training("辞書を", "じしょを")])).unwrap();
        let r3 = run_file(&s.dir, &third, &env(&s.m), &s.store, opts(false, false)).unwrap();
        assert_eq!(r3.source_id, Some(3));
    }

    #[test]
    fn reset_clears_only_the_initial_layer_and_the_ledger() {
        let s = setup(&sample());
        run_file(&s.dir, &s.file, &env(&s.m), &s.store, opts(false, false)).unwrap();
        // usage 層の記録を 1 件足す
        let feats = ikari_core::user::extract(&ikari_core::AnalyzedContext::default(), None, 1);
        s.store.add_initial(
            9,
            "あ",
            &[ikari_core::user::SegmentKey {
                surface: "亜".into(),
                reading: "あ".into(),
                pos_name: "名詞".into(),
            }],
            &feats,
            TODAY,
        );
        assert!(!s.store.is_empty());
        reset_all(&s.dir, &s.store).unwrap();
        assert!(s.store.is_empty());
        assert!(
            IngestSources::open(&s.dir.ingest_sources_tsv())
                .unwrap()
                .list()
                .is_empty()
        );
        let _ = LayerSel::All;
    }

    #[test]
    fn apply_profile_writes_profile_tsv() {
        let tmp_sentences = vec![
            training(
                "ユーザー辞書に単語を登録する",
                "ゆーざーじしょにたんごをとうろくする",
            ),
            training("ユーザー辞書を登録する", "ゆーざーじしょをとうろくする"),
        ];
        let s = setup(&tmp_sentences);
        let report = run_file(&s.dir, &s.file, &env(&s.m), &s.store, opts(false, true)).unwrap();
        let applied = report.applied_profile.unwrap();
        assert_eq!(applied[0].0, "it");
        let profile = ikari_core::Profile::load(&s.dir).unwrap().unwrap();
        assert_eq!(profile.domains[0].id, "it");
    }

    #[test]
    fn holdout_is_read_back_from_the_recorded_path_without_copying() {
        let s = setup(&sample());
        run_file(&s.dir, &s.file, &env(&s.m), &s.store, opts(false, false)).unwrap();
        let set = load_holdout(&s.dir, &env(&s.m)).unwrap();
        assert_eq!(set.sentences.len(), 1);
        let h = &set.sentences[0];
        assert_eq!(
            h.surface,
            holdout("単語を始めて登録する", "たんごをはじめてとうろくする").0
        );
        assert_eq!(h.source_id, 1);
        assert!(h.tokens.iter().any(|t| t.surface == "始めて"));
        // 評価用の文は、ユーザーデータのどこにも複製されていない
        for f in [
            s.dir.learning_tsv(),
            s.dir.ingest_sources_tsv(),
            s.dir.ingest_candidates_tsv(),
        ] {
            let text = fs::read_to_string(&f).unwrap();
            assert!(!text.contains("単語を始めて"), "{}", f.display());
        }
        assert!(set.missing.is_empty() && set.changed.is_empty());

        // 参照先が変わった / なくなったときは知らせるだけ（NFR-27）
        fs::write(
            &s.file,
            format!("{}{{\"type\":\"doc_break\"}}\n", jsonl(&sample())),
        )
        .unwrap();
        let set = load_holdout(&s.dir, &env(&s.m)).unwrap();
        assert_eq!(set.changed.len(), 1);
        fs::remove_file(&s.file).unwrap();
        let set = load_holdout(&s.dir, &env(&s.m)).unwrap();
        assert_eq!(set.missing.len(), 1);
        assert!(set.sentences.is_empty());
    }

    #[test]
    fn holdout_selection_is_stable_across_runs() {
        let s = setup(&sample());
        run_file(&s.dir, &s.file, &env(&s.m), &s.store, opts(true, false)).unwrap();
        let a = crate::read_ingest_file(&s.file).unwrap();
        let first = crate::holdout_sentences(&a.parsed, &env(&s.m), 1);
        let second = crate::holdout_sentences(&a.parsed, &env(&s.m), 1);
        assert_eq!(first, second);
        assert_eq!(first.len(), 1);
    }
}
