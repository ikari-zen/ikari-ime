//! `ikari user-dict` / `learn` / `profile` / `data`: ユーザーデータの管理コマンド。

use std::error::Error;
use std::path::Path;

use ikari_core::user::{
    LayerSel, MAX_LEARNING_ROWS, UserDictEntry, UserDictionary, UserDir, export_text, import_text,
    normalize_reading,
};
use ikari_core::{LearningStore, Profile};

use crate::args::{
    DataArgs, DataCommand, LayerArg, LearnArgs, LearnCommand, ProfileArgs, ProfileCommand,
    UserDictArgs, UserDictCommand, UserDictFormat, parse_profile_settings,
};
use crate::stack::load_dictionaries;

const NO_USER_DIR: &str =
    "ユーザーデータの置き場所を決められません（--user-dir か $IKARI_USER_DIR を指定してください）";

pub fn user_dir_of(explicit: Option<&Path>) -> Result<UserDir, Box<dyn Error>> {
    UserDir::resolve(explicit).ok_or_else(|| NO_USER_DIR.into())
}

pub fn run_user_dict(args: &UserDictArgs) -> Result<(), Box<dyn Error>> {
    let dir = user_dir_of(args.user_dir.as_deref())?;
    let path = dir.user_dict_tsv();
    let mut dict = UserDictionary::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    match &args.command {
        UserDictCommand::Add {
            reading,
            surface,
            pos,
            comment,
        } => {
            dict.add(UserDictEntry {
                reading: reading.clone(),
                surface: surface.clone(),
                pos: pos.clone(),
                comment: comment.clone(),
            })?;
            dict.save(&path)?;
            println!("登録しました（{} 語）", dict.len());
        }
        UserDictCommand::Remove { reading, surface } => {
            if !dict.remove(reading, surface) {
                return Err(format!("登録されていません: {reading} {surface}").into());
            }
            dict.save(&path)?;
            println!("削除しました（{} 語）", dict.len());
        }
        UserDictCommand::List => {
            for e in dict.entries() {
                println!("{}\t{}\t{}\t{}", e.reading, e.surface, e.pos, e.comment);
            }
        }
        UserDictCommand::Import { format, file } => {
            let text =
                std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
            let (entries, skipped) = match format {
                UserDictFormat::Ikari => {
                    let parsed = UserDictionary::parse(&text)?;
                    (parsed.entries().to_vec(), Vec::new())
                }
                UserDictFormat::Mozc => {
                    let parsed = UserDictionary::parse_mozc_report(&text)?;
                    (parsed.entries, parsed.skipped)
                }
            };
            for s in &skipped {
                eprintln!("skip: line {}: {}", s.line, s.reason);
            }
            let n = dict.add_all(entries)?;
            dict.save(&path)?;
            println!(
                "{n} 語を取り込みました（登録は合計 {} 語、飛ばした行 {}）",
                dict.len(),
                skipped.len()
            );
        }
        UserDictCommand::Check => {
            let set = load_dictionaries(args.dict.as_deref(), None)?;
            let report = dict.check(&*set.grammar);
            for w in &report.warnings {
                println!("warning: {w}");
            }
            for e in &report.errors {
                println!("error: {e}");
            }
            println!(
                "{} 品詞を確認しました（エラー {}、警告 {}）",
                report.checked,
                report.errors.len(),
                report.warnings.len()
            );
            if !report.is_ok() {
                return Err(
                    format!("検査に失敗しました（エラー {} 件）", report.errors.len()).into(),
                );
            }
        }
    }
    Ok(())
}

pub fn run_learn(args: &LearnArgs) -> Result<(), Box<dyn Error>> {
    let dir = user_dir_of(args.user_dir.as_deref())?;
    let path = dir.learning_tsv();
    let store = LearningStore::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    match &args.command {
        LearnCommand::Show { reading, limit } => {
            let (usage, initial) = store.layer_counts();
            println!("# usage={usage} initial={initial}（上限 {MAX_LEARNING_ROWS} 行）");
            let records = match reading {
                Some(r) => store.lookup(&normalize_reading(r)),
                None => store.records(),
            };
            for r in records.iter().take(*limit) {
                println!(
                    "{}\t{}\t{}\t{}\t{}\t{}\t{:016x}\t{}\t{}",
                    r.layer.as_str(),
                    r.source.map_or("-".to_string(), |s| s.to_string()),
                    r.unit.as_str(),
                    r.reading,
                    r.surface,
                    r.segments,
                    r.feature,
                    r.count,
                    r.last_day
                );
            }
            if records.len() > *limit {
                println!(
                    "# ほか {} 行（--limit で増やせます）",
                    records.len() - limit
                );
            }
        }
        LearnCommand::Reset { layer } => {
            crate::engine_lock::ensure_engine_stopped(&dir, args.force)?;
            let (usage, initial) = store.layer_counts();
            let sel = match layer {
                LayerArg::Usage => LayerSel::Usage,
                LayerArg::Initial => LayerSel::Initial,
                LayerArg::All => LayerSel::All,
            };
            store.reset(sel);
            store.save()?;
            let removed = match layer {
                LayerArg::Usage => usage,
                LayerArg::Initial => initial,
                LayerArg::All => usage + initial,
            };
            println!("{removed} 行を消しました");
        }
    }
    Ok(())
}

pub fn run_profile(args: &ProfileArgs) -> Result<(), Box<dyn Error>> {
    match &args.command {
        ProfileCommand::Presets => {
            for d in Profile::presets() {
                println!("{}\t{}\t種語 {} 語", d.id, d.name, d.seeds.len());
            }
            println!("# 種語は下書きです（ユーザー確認待ち）");
            Ok(())
        }
        ProfileCommand::Show => {
            let dir = user_dir_of(args.user_dir.as_deref())?;
            match Profile::load(&dir)? {
                Some(p) => {
                    for d in &p.domains {
                        println!("{}\t{}\t{}", d.id, d.weight.as_str(), d.name);
                    }
                }
                None => println!("(設定なし)"),
            }
            Ok(())
        }
        ProfileCommand::Set { settings } => {
            let dir = user_dir_of(args.user_dir.as_deref())?;
            let settings = parse_profile_settings(settings)?;
            let profile = Profile::from_settings(&settings)?;
            profile.save(&dir)?;
            println!("{} 分野を設定しました", profile.domains.len());
            Ok(())
        }
        ProfileCommand::Clear => {
            let dir = user_dir_of(args.user_dir.as_deref())?;
            Profile::clear(&dir)?;
            println!("設定を消しました");
            Ok(())
        }
    }
}

pub fn run_data(args: &DataArgs) -> Result<(), Box<dyn Error>> {
    let dir = user_dir_of(args.user_dir.as_deref())?;
    match &args.command {
        DataCommand::Export { file } => {
            let text = export_text(&dir)?;
            std::fs::write(file, &text).map_err(|e| format!("{}: {e}", file.display()))?;
            println!("{} に書き出しました", file.display());
        }
        DataCommand::Import { file } => {
            crate::engine_lock::ensure_engine_stopped(&dir, args.force)?;
            let text =
                std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
            let summary = import_text(&dir, &text)?;
            println!(
                "取り込みました（プロファイル {} 分野、ユーザー辞書 {} 語、学習 {} 行、取り込み元 {} 件）。既存のデータは上書きしました",
                summary.profile_domains,
                summary.user_dict_entries,
                summary.learning_rows,
                summary.ingest_sources
            );
        }
    }
    Ok(())
}
