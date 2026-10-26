//! `ikari [convert]`: 1 回の変換（フェーズ0のコマンドに、学習と文脈付きの順位付けを足したもの）。

use std::error::Error;
use std::time::Instant;

use clap::error::ErrorKind;
use ikari_core::user::today_unix_day;
use ikari_core::{Candidate, Context, Conversion};
use ikari_input::RomajiTable;

use crate::args::{Args, OutputFormat};
use crate::output::{pending_notice, render_text, render_tsv};
use crate::reading::compose_reading;
use crate::stack::{Stack, StackRequest, build_stack};

fn load_romaji_table(args: &Args) -> Result<RomajiTable, Box<dyn Error>> {
    match &args.romaji_table {
        None => Ok(RomajiTable::builtin()),
        Some(path) => {
            let tsv =
                std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
            RomajiTable::parse(&tsv).map_err(|e| format!("{}: {e}", path.display()).into())
        }
    }
}

/// 表示用に、必要なページを順に取得して `n` 件まで集める。
pub fn collect_candidates(conversion: &mut Conversion, n: usize) -> Vec<Candidate> {
    let mut candidates: Vec<Candidate> = Vec::new();
    let mut index = 0;
    while candidates.len() < n {
        let Some(page) = conversion.page(index) else {
            break;
        };
        candidates.extend(page.candidates.iter().cloned());
        if !page.has_next {
            break;
        }
        index += 1;
    }
    candidates.truncate(n);
    candidates
}

/// `rank`（1 始まり）の候補までを生成して、確定の事象を学習に記録する。
/// `--secure` のときは `Conversion::commit` が None を返す（学習しない。NFR-18）。
/// 記録したら true。
pub fn commit_and_learn(
    conversion: &mut Conversion,
    rank: usize,
    stack: &Stack,
) -> Result<bool, Box<dyn Error>> {
    // 確定する候補まで生成しておく
    let mut index = 0;
    while conversion.generated().len() < rank {
        if conversion.page(index).is_none() {
            break;
        }
        index += 1;
    }
    let Some(event) = conversion.commit(rank - 1) else {
        return Ok(false);
    };
    let store = stack.learning.as_ref().ok_or(
        "学習データの置き場所を決められません（--user-dir か $IKARI_USER_DIR を指定してください）",
    )?;
    store.record(&event, stack.vectors.as_deref(), today_unix_day());
    store.save()?;
    Ok(true)
}

pub fn run_convert(args: &Args) -> Result<(), Box<dyn Error>> {
    let table = load_romaji_table(args)?;
    let composed = match compose_reading(args.input, &args.reading, &table) {
        Ok(c) => c,
        // 入力記法の誤りは引数の誤りとして扱う（終了コード 2）
        Err(msg) => clap::Error::raw(ErrorKind::ValueValidation, format!("{msg}\n")).exit(),
    };
    if let Some(notice) = pending_notice(&composed.pending) {
        eprintln!("{notice}");
    }

    // secure では学習しない。--commit があっても学習ストアは開かず、ファイルに触れない
    let committing = args.commit.is_some() && !args.secure;
    if committing {
        crate::engine_lock::ensure_engine_stopped_at(args.common.user_dir.as_deref(), args.force)?;
    }
    let stack = build_stack(&StackRequest {
        common: &args.common,
        page_size: args.page_size,
        open_learning: committing,
        always_learning_ranker: false,
    })?;

    let ctx = Context::new()
        .with_left_text(args.context.as_str())
        .with_secure(args.secure)
        .with_profile(stack.profile.clone());

    let t_convert = Instant::now();
    let mut conversion = stack.converter.convert(&composed.kana, &ctx);
    let first_page_elapsed = conversion.page(0).map(|_| t_convert.elapsed());
    let candidates = collect_candidates(&mut conversion, args.num);

    if args.timing {
        eprintln!("timing: open dict {:.3} ms", ms(stack.meta.open_elapsed));
        match first_page_elapsed {
            Some(d) => eprintln!("timing: convert to first page {:.3} ms", ms(d)),
            None => eprintln!(
                "timing: convert (no candidates) {:.3} ms",
                ms(t_convert.elapsed())
            ),
        }
        eprintln!("config: {}", stack.meta.description);
    }

    let grammar = stack.grammar.clone();
    let pos_name = |id: u16| grammar.pos_name(id).map(str::to_string);
    let out = match args.format {
        OutputFormat::Text => {
            let context_shown = if args.secure { "" } else { &args.context };
            render_text(
                conversion.reading(),
                context_shown,
                &candidates,
                args.page_size,
                args.segments,
                &pos_name,
            )
        }
        OutputFormat::Tsv => render_tsv(&candidates, &pos_name),
    };
    print!("{out}");

    if let Some(rank) = args.commit {
        if args.secure {
            eprintln!("note: --secure では学習しません（学習データは変更されません）");
        } else if commit_and_learn(&mut conversion, rank, &stack)? {
            let surface = conversion
                .generated()
                .get(rank - 1)
                .map_or("", |c| c.surface.as_str());
            eprintln!("学習: {rank} 番「{surface}」を確定しました");
        } else {
            return Err(format!("順位 {rank} の候補がありません").into());
        }
    }
    Ok(())
}

fn ms(d: std::time::Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}
