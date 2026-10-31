//! `cargo xtask perf profile`: CPU プロファイル（samply。issue #149）。
//!
//! 流れ: 測り手（既定は殻のキー 1 回の経路 `shell_perf --metric p1`）を関数名つきでビルド → `samply record --save-only`
//! で記録（sudo は要らない）→ 記録（Firefox Profiler 形式）と、samply が書く記号表（`--unstable-presymbolicate` の
//! `*.syms.json`）から、関数ごとの時間の割合を文字の表にする。ブラウザで見るときは `samply load <記録>`。
//!
//! samply の記録と記号表の形は samply の「不安定」な出力なので、読めなかったら誤りで止まる（黙って空にしない）。
//! 記号表の読み方は `summarize`（単体テストあり）。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

use super::record::{err, repo_root};
use super::tools;
use crate::XtaskError;

pub const SAMPLY_INSTALL_HINT: &str = "cargo install samply --locked";
const DEFAULT_REPEAT: usize = 3000;
/// 測り手の `steady_dispatch`（最初の 1 打のあとの打鍵ごと）。起動（`startup_dispatch`）、最初の 1 打（`first_dispatch`）、測り手の準備は数えない
const DEFAULT_SHELL_FOCUS: &str = "shell_perf::steady_dispatch";
const DEFAULT_TOP: usize = 25;
const DEFAULT_RATE_HZ: u32 = 1000;
const NAME_LIMIT: usize = 150;
const PROFILE_FILE: &str = "profile.json";
const SYMS_FILE: &str = "profile.syms.json";

#[derive(Debug, clap::Args)]
pub struct Args {
    /// 表に出す関数の数
    #[arg(long, default_value_t = DEFAULT_TOP)]
    pub top: usize,
    /// サンプリングの間隔（1 秒あたりの回数）
    #[arg(long, default_value_t = DEFAULT_RATE_HZ)]
    pub rate: u32,
    /// 既定の対象（殻のキー 1 回）をくり返す回数。短いとサンプルが足りない
    #[arg(long, default_value_t = DEFAULT_REPEAT)]
    pub repeat: usize,
    /// 表を書き出すファイル（`eval/reports/` に置く。なければ標準出力だけ）
    #[arg(long)]
    pub out: Option<PathBuf>,
    /// このコードを含むサンプルだけを数え、その関数より外側（呼び出し元）は数えない。既定は、殻のキー 1 回なら
    /// 既定は測り手の `steady_dispatch`（打鍵ごと。起動と準備を除く）。起動は `--focus shell_perf::startup_dispatch`、最初の 1 打は `shell_perf::first_dispatch`。入力中は `shell_perf::typing_pending_dispatch`、`typing_reading_dispatch`、`typing_edit_dispatch`（`--focus shell_perf::typing_` で 3 つまとめて）。`--` で対象を替えたときは全部。空文字で全部
    #[arg(long)]
    pub focus: Option<String>,
    /// 対象を差し替える。`--` のあとに実行ファイルと引数を書く（関数名が要るので、`debug = 1` つきでビルドしたもの）
    #[arg(last = true)]
    pub command: Vec<String>,
}

/// 集計の結果。
#[derive(Debug, Default, PartialEq)]
pub struct Summary {
    pub samples: u64,
    /// 関数ごとの、いちばん内側の関数としてサンプルに載った数
    pub self_counts: HashMap<String, u64>,
    /// 関数ごとの、呼び出しの途中も含めてサンプルに載った数（1 サンプルに 1 回まで）
    pub total_counts: HashMap<String, u64>,
}

pub fn run(args: Args) -> Result<(), XtaskError> {
    tools::require_tool("samply", SAMPLY_INSTALL_HINT)?;
    if args.top == 0 || args.rate == 0 || args.repeat == 0 {
        return Err(err("--top、--rate、--repeat は 1 以上"));
    }
    let (target, command) = if args.command.is_empty() {
        let exe = tools::build_shell_bench(&[])?;
        (
            "殻のキー 1 回（shell_perf --metric p1）".to_owned(),
            vec![
                exe.to_string_lossy().into_owned(),
                "--metric".into(),
                "p1".into(),
                "--repeat".into(),
                args.repeat.to_string(),
            ],
        )
    } else {
        (args.command.join(" "), args.command.clone())
    };

    let dir = tools::output_dir()?;
    let profile_path = dir.join(PROFILE_FILE);
    let syms_path = dir.join(SYMS_FILE);
    // 古い結果を読まないように消す
    for path in [&profile_path, &syms_path] {
        let _ = std::fs::remove_file(path);
    }
    let environment = tools::environment_lines();
    eprintln!("perf profile: samply で記録します（{target}）");
    // 対象の標準出力は samply の出力と混ざるので、標準エラーへ回さず捨てずに、そのまま流す
    let status = Command::new("samply")
        .current_dir(repo_root())
        .args(["record", "--save-only", "--unstable-presymbolicate"])
        .args(["--rate", &args.rate.to_string()])
        .arg("-o")
        .arg(&profile_path)
        .arg("--")
        .args(&command)
        .status()
        .map_err(|error| err(format!("samply を起動できません: {error}")))?;
    if !status.success() {
        return Err(err(format!(
            "samply が終了コード {:?} で終わりました",
            status.code()
        )));
    }

    let profile = read_json(&profile_path)?;
    let syms = read_json(&syms_path)?;
    let focus = args.focus.clone().unwrap_or_else(|| {
        if args.command.is_empty() {
            DEFAULT_SHELL_FOCUS.to_owned()
        } else {
            String::new()
        }
    });
    let summary = summarize(&profile, &syms, &focus)?;
    let report = render(
        &summary,
        &target,
        args.rate,
        args.top,
        &focus,
        &environment,
        &profile_path,
    );
    tools::emit(&report, args.out.as_deref())
}

fn read_json(path: &Path) -> Result<Value, XtaskError> {
    let text = std::fs::read_to_string(path).map_err(|error| {
        err(format!(
            "{} を読めません（samply の出力の形が変わった可能性）: {error}",
            path.display()
        ))
    })?;
    serde_json::from_str(&text).map_err(|error| {
        err(format!(
            "{} が JSON ではありません: {error}",
            path.display()
        ))
    })
}

// ---------------------------------------------------------------- 記号表と集計

/// ライブラリ名（debug_name）→ アドレス（rva）→ 関数名の並び（いちばん内側のインライン関数が先頭）。
type SymbolMap = HashMap<String, HashMap<u64, Vec<String>>>;

/// samply の記号表（`*.syms.json`）を読む。
fn load_symbols(syms: &Value) -> Result<SymbolMap, XtaskError> {
    let data = syms["data"]
        .as_array()
        .ok_or_else(|| err("記号表に data がありません（samply の出力の形が違う）"))?;
    // 文字列の表は、記号表の全体で 1 つ
    let strings: Vec<&str> = syms["string_table"]
        .as_array()
        .map(|array| array.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let mut map = SymbolMap::new();
    for library in data {
        let Some(name) = library["debug_name"].as_str() else {
            continue;
        };
        let symbols = library["symbol_table"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let known = library["known_addresses"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let entry = map.entry(name.to_owned()).or_default();
        for pair in known {
            let (Some(rva), Some(index)) = (pair[0].as_u64(), pair[1].as_u64()) else {
                continue;
            };
            let Some(symbol) = usize::try_from(index).ok().and_then(|i| symbols.get(i)) else {
                continue;
            };
            let names: Vec<String> = symbol["frames"]
                .as_array()
                .map(|frames| {
                    frames
                        .iter()
                        .filter_map(|frame| frame["function"].as_u64())
                        .filter_map(|index| strings.get(usize::try_from(index).ok()?))
                        .map(|name| (*name).to_owned())
                        .collect()
                })
                .unwrap_or_default();
            if !names.is_empty() {
                entry.insert(rva, names);
            }
        }
    }
    Ok(map)
}

/// 記号が引けない関数の名前。`0x…` のアドレスだけのものは、ライブラリごとに 1 つにまとめる（システムのライブラリは記号がない）。
fn unresolved_name(library: Option<&str>, raw: &str) -> String {
    let library = library.unwrap_or("?");
    if raw.starts_with("0x") {
        format!("{library}!（記号なし）")
    } else {
        format!("{library}!{raw}")
    }
}

/// 記録（Firefox Profiler 形式）と記号表から、関数ごとの時間を数える。
pub fn summarize(profile: &Value, syms: &Value, focus: &str) -> Result<Summary, XtaskError> {
    let symbols = load_symbols(syms)?;
    let libs = profile["libs"]
        .as_array()
        .ok_or_else(|| err("記録に libs がありません（samply の出力の形が違う）"))?;
    let threads = profile["threads"]
        .as_array()
        .ok_or_else(|| err("記録に threads がありません（samply の出力の形が違う）"))?;
    let mut summary = Summary::default();
    for thread in threads {
        summarize_thread(thread, libs, &symbols, focus, &mut summary)?;
    }
    if summary.samples == 0 {
        return Err(err(
            "サンプルが 1 つもありません。対象が短すぎる（--repeat を増やす）",
        ));
    }
    Ok(summary)
}

fn column(table: &Value, name: &str) -> Vec<Option<u64>> {
    table[name]
        .as_array()
        .map(|array| array.iter().map(Value::as_u64).collect())
        .unwrap_or_default()
}

fn summarize_thread(
    thread: &Value,
    libs: &[Value],
    symbols: &SymbolMap,
    focus: &str,
    summary: &mut Summary,
) -> Result<(), XtaskError> {
    let sample_stacks = column(&thread["samples"], "stack");
    if sample_stacks.is_empty() {
        return Ok(());
    }
    let strings: Vec<&str> = thread["stringArray"]
        .as_array()
        .map(|array| array.iter().map(|s| s.as_str().unwrap_or("")).collect())
        .unwrap_or_default();
    let stack_prefix = column(&thread["stackTable"], "prefix");
    let stack_frame = column(&thread["stackTable"], "frame");
    let frame_address = column(&thread["frameTable"], "address");
    let frame_func = column(&thread["frameTable"], "func");
    let func_name = column(&thread["funcTable"], "name");
    let func_resource = column(&thread["funcTable"], "resource");
    let resource_lib = column(&thread["resourceTable"], "lib");
    if stack_frame.len() != stack_prefix.len() || frame_address.len() != frame_func.len() {
        return Err(err("記録の stackTable か frameTable の形が違います"));
    }

    // フレーム → 関数名の並び（インラインがあれば複数）。同じフレームは 1 度だけ引く
    let mut resolved: HashMap<usize, Vec<String>> = HashMap::new();
    let mut resolve = |frame: usize| -> Vec<String> {
        resolved
            .entry(frame)
            .or_insert_with(|| {
                let func = frame_func.get(frame).copied().flatten().map(|v| v as usize);
                let library = func
                    .and_then(|f| func_resource.get(f).copied().flatten())
                    .and_then(|r| resource_lib.get(r as usize).copied().flatten())
                    .and_then(|l| libs.get(l as usize))
                    .and_then(|lib| lib["debugName"].as_str());
                let address = frame_address.get(frame).copied().flatten();
                if let (Some(library), Some(address)) = (library, address)
                    && let Some(names) = symbols.get(library).and_then(|table| table.get(&address))
                {
                    return names.clone();
                }
                // 記号が引けない: スレッドの関数名（0x… のアドレス）にライブラリ名を添える
                let raw = func
                    .and_then(|f| func_name.get(f).copied().flatten())
                    .and_then(|s| strings.get(s as usize).copied())
                    .unwrap_or("?");
                vec![unresolved_name(library, raw)]
            })
            .clone()
    };

    for stack in sample_stacks {
        let Some(mut current) = stack else { continue };
        // 葉から根へ、関数名を集める（インラインは内側が先）
        let mut names: Vec<String> = Vec::new();
        loop {
            let index = current as usize;
            let frame = stack_frame
                .get(index)
                .copied()
                .flatten()
                .map(|v| v as usize);
            if let Some(frame) = frame {
                names.extend(resolve(frame));
            }
            match stack_prefix.get(index).copied().flatten() {
                Some(parent) => current = parent,
                None => break,
            }
        }
        // focus があれば、それを含む関数までを数える（それより外側の呼び出し元は数えない）。含まないサンプルは捨てる
        if !focus.is_empty() {
            let Some(position) = names.iter().position(|name| name.contains(focus)) else {
                continue;
            };
            names.truncate(position + 1);
        }
        let Some(leaf) = names.first() else { continue };
        summary.samples += 1;
        *summary.self_counts.entry(leaf.clone()).or_default() += 1;
        let seen: HashSet<&String> = names.iter().collect();
        for name in seen {
            *summary.total_counts.entry(name.clone()).or_default() += 1;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- 出力

fn top_of(counts: &HashMap<String, u64>, top: usize) -> Vec<(&String, u64)> {
    // 同数のときは名前順にして、出力を安定させる
    let ordered: BTreeMap<(std::cmp::Reverse<u64>, &String), ()> = counts
        .iter()
        .map(|(name, count)| ((std::cmp::Reverse(*count), name), ()))
        .collect();
    ordered
        .keys()
        .take(top)
        .map(|(std::cmp::Reverse(count), name)| (*name, *count))
        .collect()
}

fn table(title: &str, counts: &HashMap<String, u64>, samples: u64, top: usize) -> String {
    let mut text = format!("## {title}\n\n| 割合 | サンプル | 関数 |\n|---:|---:|---|\n");
    for (name, count) in top_of(counts, top) {
        let percent = count as f64 * 100.0 / samples as f64;
        text.push_str(&format!(
            "| {percent:.1}% | {count} | `{}` |\n",
            tools::shorten(name, NAME_LIMIT)
        ));
    }
    text.push('\n');
    text
}

pub fn render(
    summary: &Summary,
    target: &str,
    rate: u32,
    top: usize,
    focus: &str,
    environment: &[String],
    profile_path: &Path,
) -> String {
    let mut text = String::from("# CPU プロファイル（samply）\n\n");
    text.push_str(&format!("- 対象: {target}\n"));
    text.push_str(&format!(
        "- サンプル数: {}（{rate} Hz。全スレッドの合計）\n",
        summary.samples
    ));
    if !focus.is_empty() {
        text.push_str(&format!(
            "- 数える範囲: `{focus}` を含むサンプルだけ（それより外側は数えない）\n"
        ));
    }
    for line in environment {
        text.push_str(line);
        text.push('\n');
    }
    text.push_str(&format!(
        "- 記録: `{}`（`samply load` でブラウザに開ける）\n\n",
        tools::repo_relative(profile_path)
    ));
    text.push_str(&table(
        "自分の時間（いちばん内側の関数）の上位",
        &summary.self_counts,
        summary.samples,
        top,
    ));
    text.push_str(&table(
        "呼び出しの途中も含めた時間の上位",
        &summary.total_counts,
        summary.samples,
        top,
    ));
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 関数 2 つ（outer がインラインで inner を含む。もう 1 つは leaf）の小さな記録
    fn fixture() -> (Value, Value) {
        let profile = json!({
            "libs": [{"debugName": "app"}],
            "threads": [{
                "stringArray": ["0x10", "0x20"],
                "samples": {"stack": [1, 1, 2, null]},
                "stackTable": {"prefix": [null, 0, 0], "frame": [0, 1, 1]},
                "frameTable": {"address": [4096, 8192], "func": [0, 1]},
                "funcTable": {"name": [0, 1], "resource": [0, 0]},
                "resourceTable": {"lib": [0]},
            }],
        });
        let syms = json!({
            "string_table": ["UNKNOWN", "main", "outer", "inner"],
            "data": [{
                "debug_name": "app",
                "symbol_table": [
                    {"rva": 4096, "size": 16, "symbol": 1, "frames": [{"function": 1}]},
                    {"rva": 8192, "size": 16, "symbol": 2, "frames": [{"function": 3}, {"function": 2}]},
                ],
                "known_addresses": [[4096, 0], [8192, 1]],
            }],
        });
        (profile, syms)
    }

    #[test]
    fn counts_self_and_total_with_inlined_frames() {
        let (profile, syms) = fixture();
        let summary = summarize(&profile, &syms, "").unwrap();
        // stack 1 = main -> (inner/outer)。stack 2 は同じ frame 1 を指す別の stack。null のサンプルは数えない
        assert_eq!(summary.samples, 3);
        assert_eq!(summary.self_counts["inner"], 3);
        assert!(!summary.self_counts.contains_key("outer"));
        assert_eq!(summary.total_counts["inner"], 3);
        assert_eq!(summary.total_counts["outer"], 3);
        assert_eq!(summary.total_counts["main"], 3);
    }

    #[test]
    fn unresolved_frames_keep_the_raw_address() {
        let (profile, mut syms) = fixture();
        syms["data"][0]["known_addresses"] = json!([[4096, 0]]);
        let summary = summarize(&profile, &syms, "").unwrap();
        assert_eq!(summary.self_counts["app!（記号なし）"], 3);
    }

    #[test]
    fn focus_keeps_only_matching_samples_and_cuts_the_callers() {
        let (profile, syms) = fixture();
        // main -> outer/inner。focus = outer: main（呼び出し元）は数えない
        let summary = summarize(&profile, &syms, "outer").unwrap();
        assert_eq!(summary.samples, 3);
        assert!(!summary.total_counts.contains_key("main"));
        assert_eq!(summary.total_counts["outer"], 3);
        assert!(summarize(&profile, &syms, "nothing-matches").is_err());
    }

    #[test]
    fn empty_profile_is_an_error() {
        let (mut profile, syms) = fixture();
        profile["threads"][0]["samples"]["stack"] = json!([null]);
        assert!(summarize(&profile, &syms, "").is_err());
    }

    #[test]
    fn wrong_shape_is_an_error() {
        assert!(summarize(&json!({}), &json!({}), "").is_err());
    }

    #[test]
    fn renders_percentages_in_order() {
        let (profile, syms) = fixture();
        let summary = summarize(&profile, &syms, "").unwrap();
        let text = render(
            &summary,
            "t",
            1000,
            2,
            "",
            &["- 負荷平均: 1".into()],
            Path::new("p.json"),
        );
        assert!(text.contains("| 100.0% | 3 | `inner` |"));
        assert!(text.contains("- 負荷平均: 1"));
        assert!(text.contains("サンプル数: 3"));
    }
}
