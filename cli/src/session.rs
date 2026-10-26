//! `ikari session`: 1 行ずつ読みを入力し、番号で確定する。確定履歴を左文脈に積んでいく。
//!
//! - 対話: 読み（ローマ字、かな）を入力すると 1 ページ目が出る。番号で確定、`n` で次のページ、
//!   空行で確定せずに次の読みへ、`q` か EOF で終了。
//! - `--script <TSV>`: 「読み<TAB>選ぶ表記」の行を非対話で実行する。選ぶ表記が空なら確定せず（probe）、
//!   1 位を表示するだけ。結果は stdout に `読み<TAB>1位<TAB>確定した表記<TAB>順位` の TSV で出す。
//!   選ぶ表記が候補にないときは警告を出して確定せず、終了コードは 1。
//!
//! 確定した候補は学習ファイルに保存する（`--secure` のときは学習も文脈の積み上げもしない）。

use std::error::Error;
use std::io::{BufRead, Write};

use clap::error::ErrorKind;
use ikari_core::user::today_unix_day;
use ikari_core::{Commit, Context, Conversion};
use ikari_input::RomajiTable;

use crate::args::SessionArgs;
use crate::convert::collect_candidates;
use crate::output::{pending_notice, render_text};
use crate::reading::compose_reading;
use crate::stack::{Stack, StackRequest, build_stack};

/// 確定の候補を探すときに生成する最大件数
const MAX_SEARCH_CANDIDATES: usize = 200;
const SCRIPT_SEPARATOR: char = '\t';
const NEXT_PAGE_COMMAND: &str = "n";
const QUIT_COMMAND: &str = "q";

/// 左文脈と確定履歴
struct History {
    left_text: String,
    commits: Vec<Commit>,
}

impl History {
    fn new() -> Self {
        Self {
            left_text: String::new(),
            commits: Vec::new(),
        }
    }

    fn push(&mut self, reading: &str, surface: &str) {
        self.left_text.push_str(surface);
        self.commits.push(Commit {
            reading: reading.to_string(),
            surface: surface.to_string(),
        });
    }
}

pub fn run_session(args: &SessionArgs) -> Result<(), Box<dyn Error>> {
    if !args.secure {
        crate::engine_lock::ensure_engine_stopped_at(args.common.user_dir.as_deref(), args.force)?;
    }
    let stack = build_stack(&StackRequest {
        common: &args.common,
        page_size: args.page_size,
        open_learning: !args.secure,
        always_learning_ranker: !args.secure,
    })?;
    let table = RomajiTable::builtin();
    let mut history = History::new();
    let mut learned = false;

    let failed = match &args.script {
        Some(path) => {
            let text =
                std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
            run_script(args, &stack, &table, &text, &mut history, &mut learned)?
        }
        None => {
            run_interactive(args, &stack, &table, &mut history, &mut learned)?;
            false
        }
    };
    if learned && let Some(store) = &stack.learning {
        store.save()?;
    }
    if failed {
        return Err("候補にない表記があったため、確定しなかった行があります".into());
    }
    Ok(())
}

fn compose(args: &SessionArgs, table: &RomajiTable, text: &str) -> String {
    match compose_reading(args.input, text, table) {
        Ok(c) => {
            if let Some(notice) = pending_notice(&c.pending) {
                eprintln!("{notice}");
            }
            c.kana
        }
        Err(msg) => clap::Error::raw(ErrorKind::ValueValidation, format!("{msg}\n")).exit(),
    }
}

fn context_of(args: &SessionArgs, stack: &Stack, history: &History) -> Context {
    Context::new()
        .with_left_text(history.left_text.as_str())
        .with_history(history.commits.clone())
        .with_secure(args.secure)
        .with_profile(stack.profile.clone())
}

/// 候補を探して確定する。確定したら (順位, 表記)。
fn commit_surface(
    conversion: &mut Conversion,
    stack: &Stack,
    surface: &str,
    secure: bool,
) -> Result<Option<(usize, String)>, Box<dyn Error>> {
    let candidates = collect_candidates(conversion, MAX_SEARCH_CANDIDATES);
    let Some(i) = candidates.iter().position(|c| c.surface == surface) else {
        return Ok(None);
    };
    record(conversion, stack, i + 1, secure)?;
    Ok(Some((i + 1, candidates[i].surface.clone())))
}

/// 学習に記録する（保存はセッションの終わりにまとめて行う）。secure なら何もしない。
fn record(
    conversion: &Conversion,
    stack: &Stack,
    rank: usize,
    secure: bool,
) -> Result<(), Box<dyn Error>> {
    if secure {
        return Ok(());
    }
    if let Some(event) = conversion.commit(rank - 1) {
        let store = stack
            .learning
            .as_ref()
            .ok_or("学習データの置き場所を決められません（--user-dir か $IKARI_USER_DIR を指定してください）")?;
        store.record(&event, stack.vectors.as_deref(), today_unix_day());
    }
    Ok(())
}

/// 戻り値: 候補にない表記があったか
fn run_script(
    args: &SessionArgs,
    stack: &Stack,
    table: &RomajiTable,
    script: &str,
    history: &mut History,
    learned: &mut bool,
) -> Result<bool, Box<dyn Error>> {
    let mut failed = false;
    let mut out = String::new();
    for (i, line) in script.lines().enumerate() {
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let (reading_text, chosen) = line
            .split_once(SCRIPT_SEPARATOR)
            .map_or((line, ""), |(r, s)| (r, s));
        let kana = compose(args, table, reading_text.trim());
        let ctx = context_of(args, stack, history);
        let mut conversion = stack.converter.convert(&kana, &ctx);
        let top = collect_candidates(&mut conversion, 1)
            .first()
            .map(|c| c.surface.clone())
            .unwrap_or_default();
        let mut chosen_surface = String::new();
        let mut rank = String::new();
        if !chosen.is_empty() {
            match commit_surface(&mut conversion, stack, chosen, args.secure)? {
                Some((r, surface)) => {
                    rank = r.to_string();
                    if !args.secure {
                        history.push(conversion.reading(), &surface);
                        *learned = true;
                    }
                    chosen_surface = surface;
                }
                None => {
                    eprintln!("warning: line {}: 候補に「{chosen}」がありません", i + 1);
                    failed = true;
                }
            }
        }
        out.push_str(&format!(
            "{}\t{top}\t{chosen_surface}\t{rank}\n",
            conversion.reading()
        ));
    }
    print!("{out}");
    Ok(failed)
}

fn run_interactive(
    args: &SessionArgs,
    stack: &Stack,
    table: &RomajiTable,
    history: &mut History,
    learned: &mut bool,
) -> Result<(), Box<dyn Error>> {
    let stdin = std::io::stdin();
    let mut lines = stdin.lock().lines();
    let grammar = stack.grammar.clone();
    let pos_name = |id: u16| grammar.pos_name(id).map(str::to_string);
    loop {
        print!("読み> ");
        std::io::stdout().flush()?;
        let Some(line) = lines.next().transpose()? else {
            break;
        };
        let text = line.trim();
        if text.is_empty() {
            continue;
        }
        if text == QUIT_COMMAND {
            break;
        }
        let kana = compose(args, table, text);
        let ctx = context_of(args, stack, history);
        let mut conversion = stack.converter.convert(&kana, &ctx);
        let mut page_index = 0;
        loop {
            let Some(page) = conversion.page(page_index) else {
                println!("(候補なし)");
                break;
            };
            let has_next = page.has_next;
            let shown = page.candidates.to_vec();
            print!(
                "{}",
                render_text(
                    conversion.reading(),
                    "",
                    &shown,
                    args.page_size,
                    false,
                    &pos_name
                )
            );
            print!(
                "番号{}> ",
                if has_next {
                    "（n=次のページ）"
                } else {
                    ""
                }
            );
            std::io::stdout().flush()?;
            let Some(answer) = lines.next().transpose()? else {
                return Ok(());
            };
            let answer = answer.trim();
            if answer == NEXT_PAGE_COMMAND && has_next {
                page_index += 1;
                continue;
            }
            if answer == QUIT_COMMAND {
                return Ok(());
            }
            if let Ok(n) = answer.parse::<usize>()
                && (1..=shown.len()).contains(&n)
            {
                let rank = page_index * args.page_size + n;
                let surface = shown[n - 1].surface.clone();
                record(&conversion, stack, rank, args.secure)?;
                if !args.secure {
                    history.push(conversion.reading(), &surface);
                    *learned = true;
                }
                println!("確定: {surface}");
            }
            break;
        }
    }
    Ok(())
}
