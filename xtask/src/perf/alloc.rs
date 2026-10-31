//! `cargo xtask perf alloc`: ヒープ確保の回数とバイト数（dhat。issue #149）。
//!
//! 流れ: 殻の測り手（`shell_perf`）を `dhat-heap` つきでビルド → `--metric alloc` で流す → 標準出力の JSON（`dispatch`
//! 1 回あたりの確保の分布）と、dhat が書く `dhat-heap.json`（確保の場所ごとの回数とバイト数）を読み、文字の表にする。
//! dhat はクレートなので、道具のインストールは要らない（feature を付けたビルドだけに入る）。
//!
//! 確保の場所は、確保の呼び出しの列のうち、いちばん内側の「ikari の関数」（`ikari_*` と測り手）にまとめる。
//! `Vec::push` や `String::clone` の中ではなく、それを呼んだ ikari のコードの行が分かる。
//! 注意: 場所の表は、`--within` に合う呼び出しの列を持つ確保だけ（既定は `Shell::dispatch` の中）。替え玉のエンジン
//! （台本の照合、返事の組み立て）の確保も、`dispatch` の中から呼ばれるので入る。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

use super::record::{err, repo_root};
use super::tools;
use crate::XtaskError;

const DEFAULT_REPEAT: usize = 200;
const DEFAULT_TOP: usize = 25;
/// 測り手の `steady_dispatch`／`startup_dispatch`（`shell_perf.rs`）。定常（打鍵ごと）と起動（`Activate`）を分ける
const STEADY_WITHIN: &str = "shell_perf::steady_dispatch";
const FIRST_WITHIN: &str = "shell_perf::first_dispatch";
/// 入力中の台本の区分（`shell_perf.rs` の `typing_*_dispatch`）。名前と見出し
const TYPING_GROUPS: [(&str, &str, &str); 3] = [
    (
        "pending",
        "入力中: ローマ字の途中（読みが変わらず、先読みを送らない）",
        "shell_perf::typing_pending_dispatch",
    ),
    (
        "reading",
        "入力中: 読み（かな）が変わって先読みを送る",
        "shell_perf::typing_reading_dispatch",
    ),
    (
        "edit",
        "入力中: Backspace、Esc",
        "shell_perf::typing_edit_dispatch",
    ),
];
const STARTUP_WITHIN: &str = "shell_perf::startup_dispatch";
const NAME_LIMIT: usize = 150;
const KEY_ROW_LIMIT: usize = 170;
const DHAT_FILE: &str = "dhat-heap.json";
const DHAT_FEATURE: &str = "dhat-heap";
/// 場所の名前に選ぶ関数の接頭辞（`<` を除いたもの）。標準ライブラリや prost の中ではなく、呼んだ ikari の行にする
const OWN_PREFIXES: [&str; 2] = ["ikari_", "shell_perf"];

#[derive(Debug, clap::Args)]
pub struct Args {
    /// 表に出す確保の場所の数
    #[arg(long, default_value_t = DEFAULT_TOP)]
    pub top: usize,
    /// シナリオ全部をくり返す回数（dhat は遅いので、P1 より少なくてよい）
    #[arg(long, default_value_t = DEFAULT_REPEAT)]
    pub repeat: usize,
    /// 呼び出しの列にこの文字列を含む確保だけを、場所の表 1 つに入れる（空にすると全部）。
    /// 指定しないときは、定常（打鍵ごと）と起動（`Activate`）の 2 つの表に分ける
    #[arg(long)]
    pub within: Option<String>,
    /// キーごとの確保の回数（多い順の上位）も出す
    #[arg(long)]
    pub detail: bool,
    /// 表を書き出すファイル（`eval/reports/` に置く。なければ標準出力だけ）
    #[arg(long)]
    pub out: Option<PathBuf>,
}

/// 確保の場所 1 つぶん。
#[derive(Debug, PartialEq)]
pub struct Site {
    pub place: String,
    pub blocks: u64,
    pub bytes: u64,
}

/// `dispatch` 1 回あたりの確保の分布（測り手の JSON の `alloc`）。
#[derive(Debug, PartialEq)]
pub struct Distribution {
    pub dispatches: u64,
    pub values: BTreeMap<String, u64>,
    /// 起動（`Activate`）の分（測り手の `startup`）。古い測り手では `None`
    pub startup: Option<Box<Distribution>>,
    /// 起動の直後の最初の 1 打の分（測り手の `first`）
    pub first: Option<Box<Distribution>>,
    /// 入力中の台本の区分ごとの分（測り手の `typing`。`TYPING_GROUPS` の名前）
    pub typing: Vec<(String, Box<Distribution>)>,
    /// 定常（最初の 1 打のあと。打鍵ごと）の分（測り手の `steady`）
    pub steady: Option<Box<Distribution>>,
}

pub fn run(args: Args) -> Result<(), XtaskError> {
    if args.top == 0 || args.repeat == 0 {
        return Err(err("--top と --repeat は 1 以上"));
    }
    let exe = tools::build_shell_bench(&[DHAT_FEATURE])?;
    let dhat_path = tools::output_dir()?.join(DHAT_FILE);
    let _ = std::fs::remove_file(&dhat_path);
    let environment = tools::environment_lines();
    eprintln!("perf alloc: dhat で数えます（repeat {}）", args.repeat);
    let mut command = Command::new(&exe);
    command
        .current_dir(repo_root())
        .args(["--metric", "alloc", "--repeat", &args.repeat.to_string()])
        .arg("--dhat-file")
        .arg(&dhat_path);
    if args.detail {
        command.arg("--detail");
    }
    let output = command
        .output()
        .map_err(|error| err(format!("shell_perf を起動できません: {error}")))?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        return Err(err(format!(
            "shell_perf --metric alloc: 終了コード {:?}\n{stderr}",
            output.status.code()
        )));
    }
    let distribution = parse_distribution(&String::from_utf8_lossy(&output.stdout))?;
    let key_ranking = extract_key_ranking(&stderr);
    let dhat = read_dhat(&dhat_path)?;
    let report = match (
        &args.within,
        &distribution.steady,
        &distribution.startup,
        &distribution.first,
    ) {
        (None, Some(steady), Some(startup), Some(first)) => {
            let steady_sites = aggregate_sites(&dhat, STEADY_WITHIN)?;
            let startup_sites = aggregate_sites(&dhat, STARTUP_WITHIN)?;
            let first_sites = aggregate_sites(&dhat, FIRST_WITHIN)?;
            let typing_sites: Vec<Vec<Site>> = TYPING_GROUPS
                .iter()
                .map(|(_, _, within)| aggregate_sites(&dhat, within))
                .collect::<Result<_, _>>()?;
            let mut groups: Vec<(&str, &Distribution, &[Site], &str)> = vec![
                (
                    "定常（打鍵ごと。最初の 1 打のあと）",
                    steady,
                    &steady_sites,
                    STEADY_WITHIN,
                ),
                (
                    "起動直後の最初の 1 打（`Activate` 以外の最初のイベント）",
                    first,
                    &first_sites,
                    FIRST_WITHIN,
                ),
                (
                    "起動（`Activate`。`Shell::new` の直後と、入力先の切り替え）",
                    startup,
                    &startup_sites,
                    STARTUP_WITHIN,
                ),
            ];
            for ((name, title, within), sites) in TYPING_GROUPS.iter().zip(&typing_sites) {
                if let Some((_, found)) = distribution.typing.iter().find(|(n, _)| n == name) {
                    groups.push((title, found, sites, within));
                }
            }
            render_split(
                &groups,
                key_ranking.as_deref(),
                &args,
                &environment,
                &dhat_path,
            )
        }
        _ => {
            let within = args.within.clone().unwrap_or_default();
            let sites = aggregate_sites(&dhat, &within)?;
            render(
                &distribution,
                &sites,
                key_ranking.as_deref(),
                &args,
                &environment,
                &dhat_path,
            )
        }
    };
    tools::emit(&report, args.out.as_deref())
}

fn read_dhat(path: &Path) -> Result<Value, XtaskError> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| err(format!("{} を読めません: {error}", path.display())))?;
    serde_json::from_str(&text).map_err(|error| {
        err(format!(
            "{} が JSON ではありません: {error}",
            path.display()
        ))
    })
}

/// 測り手の標準出力（最後の JSON の行）から、確保の分布を取り出す。
pub fn parse_distribution(stdout: &str) -> Result<Distribution, XtaskError> {
    let line = stdout
        .lines()
        .rev()
        .find(|line| line.trim_start().starts_with('{'))
        .ok_or_else(|| err(format!("測り手の出力に JSON がありません: {stdout:?}")))?;
    let value: Value = serde_json::from_str(line).map_err(|error| {
        err(format!(
            "測り手の出力が JSON ではありません: {error}: {line}"
        ))
    })?;
    let alloc = value["alloc"]
        .as_object()
        .ok_or_else(|| err(format!("測り手の出力に alloc がありません: {line}")))?;
    let count = |key: &str| value[key].as_u64().unwrap_or(0);
    Ok(Distribution {
        dispatches: count("samples") + count("excluded_samples"),
        values: numbers(alloc),
        startup: group_distribution(&value["startup"]),
        first: group_distribution(&value["first"]),
        typing: TYPING_GROUPS
            .iter()
            .filter_map(|(name, _, _)| {
                Some((
                    (*name).to_owned(),
                    group_distribution(&value["typing"][name])?,
                ))
            })
            .collect(),
        steady: group_distribution(&value["steady"]),
    })
}

fn numbers(object: &serde_json::Map<String, Value>) -> BTreeMap<String, u64> {
    object
        .iter()
        .filter_map(|(key, number)| Some((key.clone(), number.as_u64()?)))
        .collect()
}

/// 測り手の `startup`／`steady`（`dispatches` と `alloc`）。なければ `None`
fn group_distribution(group: &Value) -> Option<Box<Distribution>> {
    let alloc = group["alloc"].as_object()?;
    Some(Box::new(Distribution {
        dispatches: group["dispatches"].as_u64()?,
        values: numbers(alloc),
        startup: None,
        first: None,
        typing: Vec::new(),
        steady: None,
    }))
}

/// `--detail` の標準エラーから、確保の多いキーの表（`# 確保の多いキー…` の行から、dhat の要約の行の手前まで）を取り出す。
pub fn extract_key_ranking(stderr: &str) -> Option<String> {
    let mut lines = stderr
        .lines()
        .skip_while(|line| !line.starts_with("# 確保の多いキー"));
    let header = lines.next()?;
    // キーの Debug 表示は長いので、1 行に収める
    let rows: Vec<String> = lines
        .take_while(|line| !line.starts_with("dhat:"))
        .map(|line| tools::shorten(line, KEY_ROW_LIMIT))
        .collect();
    Some(format!("{header}\n{}\n", rows.join("\n")))
}

/// dhat の 1 フレーム（`0x1004ae23c: 関数 (ファイル:行:列)`）を、関数と場所に分ける。
fn split_frame(frame: &str) -> (&str, &str) {
    let body = frame.split_once(": ").map_or(frame, |(_, rest)| rest);
    match body.rfind(" (") {
        Some(index) if body.ends_with(')') => (&body[..index], &body[index + 2..body.len() - 1]),
        _ => (body, ""),
    }
}

/// ikari の関数か（標準ライブラリや依存クレートの中ではない）。
fn is_own(function: &str) -> bool {
    let name = function.trim_start_matches('<');
    OWN_PREFIXES.iter().any(|prefix| name.starts_with(prefix))
}

/// 行の列は不要（同じ関数の別の行は別の場所だが、列の違いは同じ場所）。`ファイル:行:列` → `ファイル:行`
fn without_column(location: &str) -> String {
    let mut parts = location.rsplitn(3, ':');
    let last = parts.next();
    match (last, parts.next(), parts.next()) {
        (Some(column), Some(line), Some(file))
            if column.chars().all(|c| c.is_ascii_digit())
                && line.chars().all(|c| c.is_ascii_digit()) =>
        {
            format!("{file}:{line}")
        }
        _ => location.to_owned(),
    }
}

/// dhat-heap.json を、確保の場所ごとに合計する。`within` が空でなければ、呼び出しの列にそれを含むものだけ。
pub fn aggregate_sites(dhat: &Value, within: &str) -> Result<Vec<Site>, XtaskError> {
    let points = dhat["pps"]
        .as_array()
        .ok_or_else(|| err("dhat-heap.json に pps がありません（dhat の出力の形が違う）"))?;
    let table = dhat["ftbl"]
        .as_array()
        .ok_or_else(|| err("dhat-heap.json に ftbl がありません（dhat の出力の形が違う）"))?;
    let mut totals: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    for point in points {
        let frames: Vec<&str> = point["fs"]
            .as_array()
            .map(|indexes| {
                indexes
                    .iter()
                    .filter_map(|index| table.get(usize::try_from(index.as_u64()?).ok()?)?.as_str())
                    .collect()
            })
            .unwrap_or_default();
        if !within.is_empty() && !frames.iter().any(|frame| frame.contains(within)) {
            continue;
        }
        let own = frames
            .iter()
            .map(|frame| split_frame(frame))
            .find(|(function, _)| is_own(function))
            .or_else(|| frames.first().map(|frame| split_frame(frame)));
        let Some((function, location)) = own else {
            continue;
        };
        let place = format!("{function} ({})", without_column(location));
        let entry = totals.entry(place).or_default();
        entry.0 += point["tbk"].as_u64().unwrap_or(0);
        entry.1 += point["tb"].as_u64().unwrap_or(0);
    }
    let mut sites: Vec<Site> = totals
        .into_iter()
        .map(|(place, (blocks, bytes))| Site {
            place,
            blocks,
            bytes,
        })
        .collect();
    // 回数の多い順。同数は場所の名前順（BTreeMap から取り出した順を保つ安定ソート）
    sites.sort_by_key(|site| std::cmp::Reverse(site.blocks));
    Ok(sites)
}

fn render(
    distribution: &Distribution,
    sites: &[Site],
    key_ranking: Option<&str>,
    args: &Args,
    environment: &[String],
    dhat_path: &Path,
) -> String {
    let get = |key: &str| distribution.values.get(key).copied().unwrap_or(0);
    let mut text = String::from("# ヒープ確保（dhat）\n\n");
    text.push_str(&format!(
        "- 対象: 殻のキー（shell_perf --metric alloc。repeat {}、`dispatch` {} 回）\n",
        args.repeat, distribution.dispatches
    ));
    for line in environment {
        text.push_str(line);
        text.push('\n');
    }
    text.push_str(&format!(
        "- 記録: `{}`（dhat/dh_view.html で開ける）\n\n",
        tools::repo_relative(dhat_path)
    ));
    text.push_str("## `dispatch` 1 回あたり（同期のエンジン要求を送ったキーは除く）\n\n");
    text.push_str("| | p50 | p99 | max |\n|---|---:|---:|---:|\n");
    text.push_str(&format!(
        "| 確保の回数 | {} | {} | {} |\n| 確保のバイト数 | {} | {} | {} |\n\n",
        get("blocks_p50"),
        get("blocks_p99"),
        get("blocks_max"),
        get("bytes_p50"),
        get("bytes_p99"),
        get("bytes_max"),
    ));
    let within = args.within.clone().unwrap_or_default();
    let filter = if within.is_empty() {
        "全部".to_owned()
    } else {
        format!("`{within}` の中")
    };
    text.push_str(&format!(
        "## 確保の場所の上位（{filter}。ikari の関数と行にまとめた）\n\n\
         | 回数 | 1 回の dispatch あたり | バイト数 | 場所 |\n|---:|---:|---:|---|\n"
    ));
    for site in sites.iter().take(args.top) {
        let per_dispatch = site.blocks as f64 / distribution.dispatches.max(1) as f64;
        text.push_str(&format!(
            "| {} | {per_dispatch:.2} | {} | `{}` |\n",
            site.blocks,
            site.bytes,
            tools::shorten(&site.place, NAME_LIMIT)
        ));
    }
    text.push('\n');
    if let Some(ranking) = key_ranking {
        text.push_str("## キーごとの確保の回数（多い順）\n\n```\n");
        text.push_str(ranking);
        text.push_str("```\n\n");
    }
    text
}

/// 分布の表（p50・p99・max）と、場所の上位の表。
fn push_group(
    text: &mut String,
    title: &str,
    distribution: &Distribution,
    sites: &[Site],
    within: &str,
    top: usize,
) {
    let get = |key: &str| distribution.values.get(key).copied().unwrap_or(0);
    text.push_str(&format!(
        "## {title}: `dispatch` {} 回\n\n| | p50 | p99 | max |\n|---|---:|---:|---:|\n",
        distribution.dispatches
    ));
    text.push_str(&format!(
        "| 確保の回数 | {} | {} | {} |\n| 確保のバイト数 | {} | {} | {} |\n\n",
        get("blocks_p50"),
        get("blocks_p99"),
        get("blocks_max"),
        get("bytes_p50"),
        get("bytes_p99"),
        get("bytes_max"),
    ));
    text.push_str(&format!(
        "確保の場所の上位（`{within}` の中。ikari の関数と行にまとめた。同期のエンジン要求を送った dispatch の確保も入る）\n\n\
         | 回数 | 1 回の dispatch あたり | バイト数 | 場所 |\n|---:|---:|---:|---|\n"
    ));
    for site in sites.iter().take(top) {
        let per_dispatch = site.blocks as f64 / distribution.dispatches.max(1) as f64;
        text.push_str(&format!(
            "| {} | {per_dispatch:.2} | {} | `{}` |\n",
            site.blocks,
            site.bytes,
            tools::shorten(&site.place, NAME_LIMIT)
        ));
    }
    text.push('\n');
}

/// 定常と起動を分けた表。
fn render_split(
    groups: &[(&str, &Distribution, &[Site], &str)],
    key_ranking: Option<&str>,
    args: &Args,
    environment: &[String],
    dhat_path: &Path,
) -> String {
    let mut text = String::from("# ヒープ確保（dhat。定常と起動に分けた）\n\n");
    text.push_str(&format!(
        "- 対象: 殻のキー（shell_perf --metric alloc。repeat {}）。測り手は dispatch ごとに新しい `Shell` を作るので、起動の費用（`Activate`）を分けて数える\n",
        args.repeat
    ));
    for line in environment {
        text.push_str(line);
        text.push('\n');
    }
    text.push_str(&format!(
        "- 記録: `{}`（dhat/dh_view.html で開ける）\n\n",
        tools::repo_relative(dhat_path)
    ));
    for (title, distribution, sites, within) in groups {
        push_group(&mut text, title, distribution, sites, within, args.top);
    }
    if let Some(ranking) = key_ranking {
        text.push_str("## キーごとの確保の回数（多い順。起動のイベントも含む）\n\n```\n");
        text.push_str(ranking);
        text.push_str("```\n\n");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn dhat() -> Value {
        json!({
            "ftbl": [
                "[root]",
                "0x1: <alloc::alloc::Global as core::alloc::Allocator>::allocate (alloc/src/alloc.rs:541:14)",
                "0x2: <alloc::string::String>::clone (alloc/src/string.rs:100:5)",
                "0x3: ikari_shell_core::diag::DiagEpisode::put (src/diag/mod.rs:40:9)",
                "0x4: <ikari_shell_core::controller::Shell>::dispatch (src/controller/mod.rs:215:44)",
                "0x5: shell_perf::measure_keys (shell-core/benches/shell_perf.rs:328:45)",
                "0x6: shell_perf::setup (shell-core/benches/shell_perf.rs:10:1)",
            ],
            "pps": [
                {"tb": 100, "tbk": 5, "fs": [1, 2, 3, 4, 5]},
                {"tb": 50, "tbk": 3, "fs": [1, 2, 3, 4, 5]},
                {"tb": 999, "tbk": 99, "fs": [1, 6]},
            ],
        })
    }

    #[test]
    fn groups_by_own_frame_and_filters_within() {
        let sites =
            aggregate_sites(&dhat(), "ikari_shell_core::controller::Shell>::dispatch").unwrap();
        assert_eq!(
            sites,
            vec![Site {
                place: "ikari_shell_core::diag::DiagEpisode::put (src/diag/mod.rs:40)".into(),
                blocks: 8,
                bytes: 150,
            }]
        );
    }

    #[test]
    fn empty_within_keeps_everything_sorted_by_blocks() {
        let sites = aggregate_sites(&dhat(), "").unwrap();
        assert_eq!(sites.len(), 2);
        assert_eq!(sites[0].blocks, 99);
        assert!(sites[0].place.starts_with("shell_perf::setup"));
    }

    #[test]
    fn wrong_shape_is_an_error() {
        assert!(aggregate_sites(&json!({}), "").is_err());
    }

    #[test]
    fn extracts_the_key_ranking() {
        let stderr =
            "x\n# 確保の多いキー(上位)  キー\n      90  a #0.1 K\n      80  b\ndhat: Total: 1\n";
        assert_eq!(
            extract_key_ranking(stderr).as_deref(),
            Some("# 確保の多いキー(上位)  キー\n      90  a #0.1 K\n      80  b\n")
        );
        assert_eq!(extract_key_ranking("nothing"), None);
    }

    #[test]
    fn splits_frames() {
        assert_eq!(
            split_frame("0x1: a::b (src/x.rs:1:2)"),
            ("a::b", "src/x.rs:1:2")
        );
        assert_eq!(split_frame("[root]"), ("[root]", ""));
        assert_eq!(without_column("src/x.rs:1:2"), "src/x.rs:1");
        assert_eq!(without_column("src/x.rs"), "src/x.rs");
    }

    #[test]
    fn reads_the_steady_and_startup_groups() {
        let stdout = "{\"alloc\":{\"blocks_p50\":15},\"samples\":10,\"excluded_samples\":2,\"steady\":{\"dispatches\":8,\"alloc\":{\"blocks_p50\":7}},\"startup\":{\"dispatches\":4,\"alloc\":{\"blocks_p50\":90}}}\n";
        let found = parse_distribution(stdout).unwrap();
        assert_eq!(found.steady.as_ref().unwrap().dispatches, 8);
        assert_eq!(found.steady.unwrap().values["blocks_p50"], 7);
        assert_eq!(found.startup.unwrap().values["blocks_p50"], 90);
    }

    #[test]
    fn reads_the_bench_distribution() {
        let stdout = "noise\n{\"alloc\":{\"blocks_p50\":15,\"blocks_max\":194},\"samples\":10,\"excluded_samples\":2}\n";
        let found = parse_distribution(stdout).unwrap();
        assert_eq!(found.dispatches, 12);
        assert_eq!(found.values["blocks_p50"], 15);
        assert!(parse_distribution("{\"samples\":1}").is_err());
        assert!(parse_distribution("none").is_err());
    }
}
