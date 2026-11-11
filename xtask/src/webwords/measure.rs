//! `cargo xtask webwords measure`: 一覧の作り直しの、Release の手前まで（docs/webwords-update.md）。
//!
//! 一覧（`build`。あれば再利用）→ 評価用 scratch の前準備 → 承認済みの既定の値（dict-builder の既定。引数で上書きしない）で
//! 1 点を LM なし（`--lm` があれば LM ありも）で測る → 基準との比較 → 辞書の増加量 → 抜き取り 30 語 → まとめ。
//! 実機の辞書（data/build）とユーザーデータには触れない（出力先は別。`check_out_dir`）。
//! 判断（悪化があれば終了コードを非 0 にする）はここに置く。評価の実行は `eval/scripts/webwords_sweep.sh` を呼ぶ。

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::data::run::{Failure, cargo, log, repo_root, run as exec};

use super::{MANIFEST_FILE, run_build, validate_date};

/// 抜き取りの語数
pub(super) const SAMPLE_SIZE: usize = 30;
/// 抜き取りの seed の既定（固定。誰が流しても同じ語を抜く）
pub(super) const DEFAULT_SEED: u64 = 20_261_106;
/// 抜き取る区分と df の範囲（下限は配る下限 5。ADR 0055 §4.5）
const SAMPLE_CLASS: &str = "common";
const SAMPLE_DF_MIN: u64 = 5;
const SAMPLE_DF_MAX: u64 = 6;
/// 評価の点の名前。`webwords_sweep.sh` の "default" は dict-builder の既定の値で作る
const POINT: &str = "default";
const SWEEP_SCRIPT: &str = "eval/scripts/webwords_sweep.sh";
const COMPARE_SCRIPT: &str = "eval/scripts/webwords_compare.py";
const LM_TARGET_DIR: &str = "target/snapshot-bin";
const TOTAL_ROW_PREFIX: &str = "| 合計";
const APP_SUPPORT_RELATIVE: &str = "Library/Application Support/ikari-ime";
const BYTES_PER_MIB: f64 = 1024.0 * 1024.0;

/// `compare` の合計行
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Total {
    pub base_top1: u64,
    pub cand_top1: u64,
    pub worse: u64,
    pub better: u64,
}

/// `webwords_compare.py` の出力の合計行（`| 合計… | | 基準 | 候補 | 悪化 | 改善 | 表記変化 |`）を読む
pub(super) fn parse_total(text: &str) -> Result<Total, Failure> {
    let line = text
        .lines()
        .find(|l| l.starts_with(TOTAL_ROW_PREFIX))
        .ok_or_else(|| Failure::new("比較の出力に合計の行がありません"))?;
    let cells: Vec<&str> = line.split('|').map(str::trim).collect();
    let num = |i: usize| -> Result<u64, Failure> {
        cells
            .get(i)
            .and_then(|c| c.parse().ok())
            .ok_or_else(|| Failure::new(format!("合計の行を読めません: {line}")))
    };
    Ok(Total {
        base_top1: num(3)?,
        cand_top1: num(4)?,
        worse: num(5)?,
        better: num(6)?,
    })
}

/// `summary.tsv` の 1 行（名前、追加行数、system.dict の大きさ、…）
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Summary {
    pub rows: u64,
    pub dict_bytes: u64,
}

pub(super) fn parse_summary(text: &str, name: &str) -> Result<Summary, Failure> {
    let line = text
        .lines()
        .rfind(|l| l.split('\t').next() == Some(name))
        .ok_or_else(|| Failure::new(format!("summary.tsv に {name} の行がありません")))?;
    let cols: Vec<&str> = line.split('\t').collect();
    let num = |i: usize| -> Result<u64, Failure> {
        cols.get(i)
            .and_then(|c| c.trim().parse().ok())
            .ok_or_else(|| Failure::new(format!("summary.tsv の行を読めません: {line}")))
    };
    Ok(Summary {
        rows: num(1)?,
        dict_bytes: num(2)?,
    })
}

/// 一覧の本文から、抜き取りの候補（common で df が範囲内）を元の並びで取る
fn sample_candidates(list: &str) -> Vec<&str> {
    list.lines()
        .filter(|l| !l.starts_with('#'))
        .filter(|l| {
            let c: Vec<&str> = l.split('\t').collect();
            c.len() == 4
                && c[3] == SAMPLE_CLASS
                && c[2]
                    .parse::<u64>()
                    .is_ok_and(|df| (SAMPLE_DF_MIN..=SAMPLE_DF_MAX).contains(&df))
        })
        .collect()
}

/// splitmix64（seed から決まる乱数。外部の crate を足さない）
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// 候補から `count` 語を seed で抜く（同じ入力と seed なら同じ語。元の並びで返す）
pub(super) fn sample_words(list: &str, seed: u64, count: usize) -> Vec<String> {
    let candidates = sample_candidates(list);
    let mut index: Vec<usize> = (0..candidates.len()).collect();
    let take = count.min(index.len());
    let mut state = seed;
    for i in 0..take {
        let span = (index.len() - i) as u64;
        let j = i + (splitmix64(&mut state) % span) as usize;
        index.swap(i, j);
    }
    let mut chosen: Vec<usize> = index[..take].to_vec();
    chosen.sort_unstable();
    chosen.iter().map(|&i| candidates[i].to_owned()).collect()
}

fn sample_file_text(words: &[String]) -> String {
    let mut text = String::from("表記\ttf\tdf\t区分\n");
    for word in words {
        text.push_str(word);
        text.push('\n');
    }
    text
}

/// 出力先が、実機の辞書やユーザーデータの下（または同じ場所）でないことを確かめる
pub(super) fn check_out_dir(out: &Path, forbidden: &[PathBuf]) -> Result<(), Failure> {
    let abs = std::path::absolute(out)
        .map_err(|e| Failure::new(format!("{} を絶対パスにできません: {e}", out.display())))?;
    for bad in forbidden {
        if abs.starts_with(bad) || bad.starts_with(&abs) {
            return Err(Failure::new(format!(
                "--out は実機のデータの場所と重なっています: {} ({} と重なる)",
                abs.display(),
                bad.display()
            )));
        }
    }
    Ok(())
}

fn forbidden_dirs(root: &Path, data: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![root.join("data"), data.join("raw"), data.join("build")];
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(Path::new(&home).join(APP_SUPPORT_RELATIVE));
    }
    dirs.into_iter()
        .filter_map(|d| std::path::absolute(d).ok())
        .collect()
}

/// まとめの Markdown の材料
pub(super) struct ReportInput<'a> {
    pub date: &'a str,
    pub seed: u64,
    pub lm: Option<&'a Path>,
    pub plain: Total,
    pub plain_table: &'a str,
    pub with_lm: Option<(Total, &'a str)>,
    pub rows: u64,
    pub base_bytes: u64,
    pub cand_bytes: u64,
    pub sample_count: usize,
    pub list_report: &'a Path,
}

pub(super) fn worse_total(input: &ReportInput<'_>) -> u64 {
    input.plain.worse + input.with_lm.as_ref().map_or(0, |(t, _)| t.worse)
}

pub(super) fn render_report(input: &ReportInput<'_>) -> String {
    let worse = worse_total(input);
    let base_mib = input.base_bytes as f64 / BYTES_PER_MIB;
    let cand_mib = input.cand_bytes as f64 / BYTES_PER_MIB;
    let verdict = if worse == 0 {
        "悪化 0".to_owned()
    } else {
        format!("悪化 {worse} 件（Release に進まない）")
    };
    let mut t = format!("# 外来語の一覧の測り直し（jawiki-{}）\n\n", input.date);
    t.push_str(&format!("結果: **{verdict}**\n\n"));
    t.push_str("- 値: dict-builder の既定（承認済み。ADR 0055 §4.5。引数で上書きしていない）\n");
    t.push_str(&format!(
        "- 辞書の増加量: system.dict {base_mib:.1} MiB → {cand_mib:.1} MiB（{:+.1} MiB）、追加した行 {} 行\n",
        cand_mib - base_mib,
        input.rows
    ));
    t.push_str(&format!(
        "- 抜き取り: `sample.tsv` に {} 語（{SAMPLE_CLASS}、df {SAMPLE_DF_MIN}〜{SAMPLE_DF_MAX}、seed {}）\n",
        input.sample_count, input.seed
    ));
    t.push_str(&format!(
        "- 一覧のレポート（理由ごとの件数）: `{}`\n\n",
        input.list_report.display()
    ));
    t.push_str(&format!(
        "## LM なし\n\n1 位正解 {} → {}、悪化 {}、改善 {}\n\n{}\n",
        input.plain.base_top1,
        input.plain.cand_top1,
        input.plain.worse,
        input.plain.better,
        input.plain_table
    ));
    match (&input.with_lm, input.lm) {
        (Some((total, table)), Some(lm)) => t.push_str(&format!(
            "## LM あり（{}）\n\n1 位正解 {} → {}、悪化 {}、改善 {}\n\n{}\n",
            lm.display(),
            total.base_top1,
            total.cand_top1,
            total.worse,
            total.better,
            table
        )),
        _ => t.push_str("## LM あり\n\n測っていない（`--lm` を渡していない）。\n"),
    }
    t
}

fn compare(root: &Path, base: &Path, cand: &Path) -> Result<(Total, String), Failure> {
    let output = Command::new("python3")
        .current_dir(root)
        .args(["-I", COMPARE_SCRIPT])
        .arg(base)
        .arg(cand)
        .args(["--show", "100"])
        .output()
        .map_err(|e| Failure::new(format!("python3 を起動できません: {e}")))?;
    // 終了コード 1 は「悪化あり」。それ以外の失敗だけエラーにする
    if !matches!(output.status.code(), Some(0 | 1)) {
        return Err(Failure::new(format!(
            "比較に失敗しました（{}）: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    Ok((parse_total(&text)?, text))
}

fn build_binaries(root: &Path, lm: bool) -> Result<(), Failure> {
    log("評価用のバイナリをビルド");
    exec(cargo(root).args(["build", "--release", "-p", "ikari-eval", "-p", "ikari-cli"]))?;
    if lm {
        log("評価用のバイナリをビルド（LM つき）");
        exec(
            cargo(root)
                .env("CARGO_TARGET_DIR", root.join(LM_TARGET_DIR))
                .args([
                    "build",
                    "--release",
                    "-p",
                    "ikari-eval",
                    "-p",
                    "ikari-cli",
                    "--features",
                    "lm-metal",
                ]),
        )?;
    }
    Ok(())
}

fn sweep(
    root: &Path,
    scratch: &Path,
    data_raw: &Path,
    lm: Option<&Path>,
    arg: &str,
) -> Result<(), Failure> {
    let mut cmd = Command::new("bash");
    cmd.current_dir(root)
        .arg(SWEEP_SCRIPT)
        .arg(scratch)
        .arg(arg)
        .env("DATA_RAW", data_raw);
    if let Some(lm) = lm {
        cmd.env("LM", lm);
    }
    exec(&mut cmd)
}

fn read(path: &Path) -> Result<String, Failure> {
    std::fs::read_to_string(path)
        .map_err(|e| Failure::new(format!("{} を読めません: {e}", path.display())))
}

fn write(path: &Path, text: &str) -> Result<(), Failure> {
    std::fs::write(path, text)
        .map_err(|e| Failure::new(format!("{} に書けません: {e}", path.display())))
}

pub(super) struct Options {
    pub date: String,
    pub work: PathBuf,
    pub out: PathBuf,
    pub lm: Option<PathBuf>,
    pub seed: u64,
    pub data: Option<PathBuf>,
}

pub(super) fn run_measure(opts: Options) -> Result<(), Failure> {
    validate_date(&opts.date)?;
    let root = repo_root();
    let data = opts.data.clone().unwrap_or_else(|| root.join("data"));
    check_out_dir(&opts.out, &forbidden_dirs(&root, &data))?;
    if let Some(lm) = &opts.lm
        && !lm.is_file()
    {
        return Err(Failure::new(format!(
            "--lm のファイルがありません: {}",
            lm.display()
        )));
    }
    let data_raw = data.join("raw");
    if !data_raw.is_dir() {
        return Err(Failure::new(format!(
            "{} がありません（scripts/fetch-data.sh で取得する）",
            data_raw.display()
        )));
    }
    let manifest = opts.work.join(MANIFEST_FILE);
    if !manifest.is_file() {
        return Err(Failure::new(format!(
            "{} がありません（先に fetch を流す）",
            manifest.display()
        )));
    }

    let out = std::path::absolute(&opts.out)
        .map_err(|e| Failure::new(format!("--out を絶対パスにできません: {e}")))?;
    let date = &opts.date;
    let list = out.join(format!("webwords-{date}.tsv"));
    let list_report = out.join(format!("webwords-{date}-report.md"));
    if list.is_file() && list_report.is_file() {
        log(&format!("一覧があるので build を省く: {}", list.display()));
    } else {
        run_build(date, &opts.work, &out, opts.data.clone())?;
    }

    build_binaries(&root, opts.lm.is_some())?;

    let scratch = out.join("scratch");
    std::fs::create_dir_all(scratch.join("user-empty"))
        .map_err(|e| Failure::new(format!("{} を作れません: {e}", scratch.display())))?;
    let scratch_list = scratch.join("webwords-list.tsv");
    let _ = std::fs::remove_file(&scratch_list);
    std::fs::copy(&list, &scratch_list)
        .map_err(|e| Failure::new(format!("{} を作れません: {e}", scratch_list.display())))?;
    // 前の測定の summary と点を引き継がない
    let _ = std::fs::remove_file(scratch.join("summary.tsv"));
    let _ = std::fs::remove_dir_all(scratch.join("pt"));

    log("基準の辞書と評価（--no-webwords）");
    sweep(&root, &scratch, &data_raw, opts.lm.as_deref(), "--base")?;
    log("既定の値の辞書と評価");
    sweep(&root, &scratch, &data_raw, opts.lm.as_deref(), POINT)?;

    let pt = scratch.join("pt");
    let (plain, plain_table) = compare(&root, &pt.join("base"), &pt.join(POINT))?;
    let with_lm = if opts.lm.is_some() {
        Some(compare(
            &root,
            &pt.join("base-lm"),
            &pt.join(format!("{POINT}-lm")),
        )?)
    } else {
        None
    };

    let summary = parse_summary(&read(&scratch.join("summary.tsv"))?, POINT)?;
    let base_dict = pt.join("base/data/build/dict/system.dict");
    let base_bytes = std::fs::metadata(&base_dict)
        .map_err(|e| Failure::new(format!("{} を調べられません: {e}", base_dict.display())))?
        .len();

    let words = sample_words(&read(&list)?, opts.seed, SAMPLE_SIZE);
    let sample_path = out.join("sample.tsv");
    write(&sample_path, &sample_file_text(&words))?;

    let input = ReportInput {
        date,
        seed: opts.seed,
        lm: opts.lm.as_deref(),
        plain,
        plain_table: &plain_table,
        with_lm: with_lm.as_ref().map(|(t, s)| (*t, s.as_str())),
        rows: summary.rows,
        base_bytes,
        cand_bytes: summary.dict_bytes,
        sample_count: words.len(),
        list_report: &list_report,
    };
    let report_path = out.join(format!("measure-{date}.md"));
    write(&report_path, &render_report(&input))?;
    println!("まとめ: {}", report_path.display());
    println!("抜き取り: {}", sample_path.display());

    let worse = worse_total(&input);
    if worse > 0 {
        eprintln!("\n############################################");
        eprintln!("#  悪化 {worse} 件。Release に進まない。");
        eprintln!("#  {} を読む", report_path.display());
        eprintln!("############################################");
        return Err(Failure::new(format!("悪化が {worse} 件あります")));
    }
    println!(
        "悪化 0。まとめと sample.tsv を人が読み、ユーザーに聞いてから publish に進む（docs/webwords-update.md）"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMPARE: &str = "| セット | 件数 | 1 位正解 基準 | 候補 | 悪化 | 改善 | 1 位の表記が変わった |\n|---|---|---|---|---|---|---|\n| a | 10 | 5 | 6 | 0 | 1 | 1 |\n| 合計（変化のあったセットのみ表示） | | 5 | 6 | 2 | 3 | 4 |\n悪化: a x\n";

    #[test]
    fn total_row_is_parsed() {
        let t = parse_total(COMPARE).unwrap();
        assert_eq!(
            t,
            Total {
                base_top1: 5,
                cand_top1: 6,
                worse: 2,
                better: 3
            }
        );
        assert!(parse_total("no table").is_err());
        assert!(parse_total("| 合計 | | x | 1 | 1 | 1 | 1 |").is_err());
    }

    #[test]
    fn summary_row_is_found_by_name() {
        let text = "df1-c1\t5\t10\tx\n\ndefault\t120\t4096\ttop1\t\tranks\n";
        let s = parse_summary(text, POINT).unwrap();
        assert_eq!((s.rows, s.dict_bytes), (120, 4096));
        assert!(parse_summary(text, "none").is_err());
        assert!(parse_summary("default\tx\t1\n", POINT).is_err());
    }

    fn list() -> String {
        let mut t = String::from("#ikari-webwords\tschema=2\n");
        for i in 0..200 {
            let df = 3 + i % 6; // 3..=8
            let class = if i % 4 == 0 { "proper.place" } else { "common" };
            t.push_str(&format!("ゴ{i:03}\t10\t{df}\t{class}\n"));
        }
        t
    }

    #[test]
    fn sample_is_fixed_by_seed_and_filtered() {
        let list = list();
        let a = sample_words(&list, DEFAULT_SEED, SAMPLE_SIZE);
        assert_eq!(a, sample_words(&list, DEFAULT_SEED, SAMPLE_SIZE));
        assert_ne!(a, sample_words(&list, DEFAULT_SEED + 1, SAMPLE_SIZE));
        assert_eq!(a.len(), SAMPLE_SIZE);
        for line in &a {
            let c: Vec<&str> = line.split('\t').collect();
            assert_eq!(c[3], "common");
            assert!(matches!(c[2], "5" | "6"), "{line}");
        }
        let mut sorted = a.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), a.len(), "重複なし");
    }

    #[test]
    fn sample_takes_all_when_few() {
        let text = "#h\na\t1\t5\tcommon\nb\t1\t6\tcommon\nc\t1\t7\tcommon\n";
        assert_eq!(sample_words(text, 1, 30).len(), 2);
        assert!(sample_words("#h\n", 1, 30).is_empty());
    }

    #[test]
    fn out_dir_must_not_overlap_real_data() {
        // 本番の forbidden_dirs と同じく絶対パスにする（Windows では "/repo" にドライブが付く）
        let forbidden: Vec<PathBuf> = [
            "/repo/data",
            "/home/u/Library/Application Support/ikari-ime",
        ]
        .iter()
        .map(|p| std::path::absolute(p).unwrap())
        .collect();
        assert!(check_out_dir(Path::new("/repo/data/build/x"), &forbidden).is_err());
        assert!(check_out_dir(Path::new("/repo"), &forbidden).is_err());
        assert!(
            check_out_dir(
                Path::new("/home/u/Library/Application Support/ikari-ime/x"),
                &forbidden
            )
            .is_err()
        );
        assert!(check_out_dir(Path::new("/tmp/out"), &forbidden).is_ok());
    }

    #[test]
    fn report_shows_worse_and_growth() {
        let total = |worse| Total {
            base_top1: 1,
            cand_top1: 2,
            worse,
            better: 1,
        };
        let make = |worse, lm: Option<(Total, &'static str)>| ReportInput {
            date: "20270101",
            seed: 7,
            lm: lm.as_ref().map(|_| Path::new("/m.gguf")),
            plain: total(worse),
            plain_table: "TABLE",
            with_lm: lm,
            rows: 100,
            base_bytes: 1024 * 1024,
            cand_bytes: 3 * 1024 * 1024,
            sample_count: 30,
            list_report: Path::new("/r.md"),
        };
        let ok = render_report(&make(0, None));
        assert!(ok.contains("悪化 0"), "{ok}");
        assert!(ok.contains("+2.0 MiB"), "{ok}");
        assert!(ok.contains("測っていない"), "{ok}");
        let bad = make(0, Some((total(3), "LMTABLE")));
        assert_eq!(worse_total(&bad), 3);
        let text = render_report(&bad);
        assert!(
            text.contains("悪化 3 件") && text.contains("LMTABLE"),
            "{text}"
        );
    }
}
