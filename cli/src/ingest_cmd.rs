//! `ikari ingest`: 文章の取り込み（初期学習）。処理は `ikari-import`、ここは表示と引数の橋渡し。

use std::error::Error;
use std::path::Path;

use ikari_core::LearningStore;
use ikari_core::user::{
    IngestCandidates, IngestSources, UserDictEntry, UserDictionary, UserDir, format_utc,
};
use ikari_import::{
    CLOUD_LLM_WARNING, Env, INGEST_PROMPT, IngestOutcome, RunOptions, remove_source, reset_all,
    run_file,
};

use crate::args::{CommonArgs, IngestArgs, IngestCommand};
use crate::engine_lock::ensure_engine_stopped;
use crate::stack::{load_dictionaries, load_vectors};
use crate::user_cmds::user_dir_of;

pub fn run_ingest(args: &IngestArgs) -> Result<(), Box<dyn Error>> {
    match &args.command {
        IngestCommand::Prompt => {
            // 注意は画面にも出す（NFR-28）。プロンプトの冒頭にも同じ注意が入っている
            eprintln!("{CLOUD_LLM_WARNING}");
            print!("{INGEST_PROMPT}");
            Ok(())
        }
        IngestCommand::Run {
            file,
            dry_run,
            apply_profile,
            vectors,
        } => run(args, file, *dry_run, *apply_profile, vectors.as_deref()),
        IngestCommand::List => list(&user_dir_of(args.user_dir.as_deref())?),
        IngestCommand::Remove { id } => {
            let dir = user_dir_of(args.user_dir.as_deref())?;
            ensure_engine_stopped(&dir, args.force)?;
            let store = open_store(&dir)?;
            remove_source(&dir, &store, *id)?;
            println!(
                "取り込み元 id={id} と、その初期学習・登録候補を消しました（取り込みファイルは残っています）"
            );
            Ok(())
        }
        IngestCommand::Reset => {
            let dir = user_dir_of(args.user_dir.as_deref())?;
            ensure_engine_stopped(&dir, args.force)?;
            let store = open_store(&dir)?;
            reset_all(&dir, &store)?;
            println!("取り込みをすべて消しました（実際の使用による学習は残っています）");
            Ok(())
        }
        IngestCommand::Candidates => candidates(&user_dir_of(args.user_dir.as_deref())?),
        IngestCommand::Accept { numbers, pos } => {
            accept(&user_dir_of(args.user_dir.as_deref())?, numbers, pos)
        }
    }
}

fn open_store(dir: &UserDir) -> Result<LearningStore, Box<dyn Error>> {
    LearningStore::open(&dir.learning_tsv())
        .map_err(|e| format!("{}: {e}", dir.learning_tsv().display()).into())
}

fn run(
    args: &IngestArgs,
    file: &Path,
    dry_run: bool,
    apply_profile: bool,
    vectors: Option<&Path>,
) -> Result<(), Box<dyn Error>> {
    let dir = user_dir_of(args.user_dir.as_deref())?;
    if !dry_run {
        ensure_engine_stopped(&dir, args.force)?;
    }
    let set = load_dictionaries(args.dict.as_deref(), Some(&dir))?;
    let common = CommonArgs {
        dict: args.dict.clone(),
        vectors: vectors.map(Path::to_path_buf),
        ..CommonArgs::default()
    };
    let vectors = load_vectors(&common, &set)?;
    let store = open_store(&dir)?;
    store.attach_grammar(set.grammar_dyn());
    let env = Env {
        dict: &*set.dict,
        grammar: &*set.grammar,
        vectors: vectors.as_deref(),
    };
    let report = run_file(
        &dir,
        file,
        &env,
        &store,
        RunOptions::now(dry_run, apply_profile),
    )?;
    eprintln!("{CLOUD_LLM_WARNING}");

    match report.source_id {
        Some(id) => println!("取り込みました: {}（id={id}）", file.display()),
        None => println!("dry-run: 何も保存していません: {}", file.display()),
    }
    print_outcome(&report.outcome);
    if let Some(applied) = &report.applied_profile {
        let text: Vec<String> = applied
            .iter()
            .map(|(id, w)| format!("{id}={}", w.as_str()))
            .collect();
        println!("プロファイルに書きました: {}", text.join(" "));
    } else if apply_profile && !dry_run {
        println!("プロファイル: 十分なスコアの分野がなかったので書きませんでした");
    }
    Ok(())
}

fn print_outcome(o: &IngestOutcome) {
    println!("プロンプトの版: {}", o.prompt_version);
    println!(
        "有効な文: {}、捨てた行・文: {}",
        o.sentences_valid,
        o.sentences_dropped()
    );
    for (reason, n) in o.drops.iter() {
        println!("  捨てた理由: {reason}: {n}");
    }
    println!(
        "アライメントできた文: {}（評価用に取り分け {}、初期学習に使用 {}、未知語を含み学習せず {}）",
        o.aligned, o.holdout, o.learned, o.with_unknown
    );
    println!("初期学習の記録（語単位）: {}", o.initial_records);
    println!(
        "登録候補: {} 語（`ikari ingest candidates` で確認、`ikari ingest accept` で登録）",
        o.candidates.len()
    );
    if !o.domains.is_empty() {
        println!("分野の推定:");
        for d in &o.domains {
            println!("  {}\t{}\t{:.3}\t{:?}", d.id, d.name, d.score, d.method);
        }
    }
}

fn list(dir: &UserDir) -> Result<(), Box<dyn Error>> {
    let ledger = IngestSources::open(&dir.ingest_sources_tsv())?;
    for s in ledger.list() {
        let missing = if Path::new(&s.path).exists() {
            ""
        } else {
            "\t(ファイルが見つかりません)"
        };
        println!(
            "{}\t{}\t{}\t{:016x}\t{}\t{}\t{}{missing}",
            s.id,
            format_utc(s.imported_at),
            s.path,
            s.fnv64,
            s.prompt_version,
            s.sentences_ok,
            s.sentences_dropped
        );
    }
    if ledger.list().is_empty() {
        println!("(取り込みなし)");
    }
    Ok(())
}

fn candidates(dir: &UserDir) -> Result<(), Box<dyn Error>> {
    let c = IngestCandidates::open(&dir.ingest_candidates_tsv())?;
    let list = c.list();
    for (i, cand) in list.iter().enumerate() {
        println!(
            "{}\t{}\t{}\t{}",
            i + 1,
            cand.reading,
            cand.surface,
            cand.count
        );
    }
    if list.is_empty() {
        println!("(登録候補なし)");
    }
    Ok(())
}

fn accept(dir: &UserDir, numbers: &[usize], pos: &str) -> Result<(), Box<dyn Error>> {
    let mut cands = IngestCandidates::open(&dir.ingest_candidates_tsv())?;
    let list = cands.list();
    let mut chosen = Vec::new();
    for &n in numbers {
        let c = n
            .checked_sub(1)
            .and_then(|i| list.get(i))
            .ok_or_else(|| format!("番号 {n} の候補はありません（1〜{}）", list.len()))?;
        chosen.push(c.clone());
    }
    let mut dict = UserDictionary::open(&dir.user_dict_tsv())?;
    let entries: Vec<UserDictEntry> = chosen
        .iter()
        .map(|c| UserDictEntry {
            reading: c.reading.clone(),
            surface: c.surface.clone(),
            pos: pos.to_string(),
            comment: "ingest".to_string(),
        })
        .collect();
    // 1 つでも不正なら何も変えない
    let n = dict.add_all(entries)?;
    for c in &chosen {
        cands.take(&c.reading, &c.surface);
    }
    dict.save(&dir.user_dict_tsv())?;
    cands.save(&dir.ingest_candidates_tsv())?;
    println!("{n} 語をユーザー辞書に登録しました（品詞: {pos}）");
    Ok(())
}
