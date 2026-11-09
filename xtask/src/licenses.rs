//! `cargo xtask licenses`（issue #242）。配るバイナリが静的に含む Rust の依存クレートのライセンス一覧
//! （`THIRD_PARTY_LICENSES_RUST.md`）を、cargo-about で作る。
//!
//! - 配るもの: macOS の ikari.app に入る Rust の成果物（`ikari-engine-ffi`（`lm-metal`）と `ikari-shell-ffi`）。
//!   ビルドの引数は `macos/scripts/build.sh` と合わせる。xtask、評価、cli、dict-builder などは含めない
//! - 設定は `about.toml`（許可するライセンスを `accepted` に明記。想定外のライセンスが入ると cargo-about が失敗する。
//!   build.rs だけが使うもの、dev-dependencies は、成果物に入らないので除く）
//! - 道具は cargo-about（`cargo install cargo-about --locked --features cli`。自動では入れない）
//! - 出力は `--offline`（手元の `~/.cargo/registry` のライセンスのファイルだけを読む）で、誰が流しても同じになる
//! - 2 つの成果物の結果は、ライセンス本文ごとにまとめて 1 つの Markdown にする（テンプレートは使わず、ここで書く）
//! - `--check` は、書き出す内容が今のファイルと同じかを確かめる（`cargo xtask check` から呼ぶ）

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use serde_json::Value;

use crate::XtaskError;
use crate::data::run::repo_root;

/// 生成物（リポジトリ直下）
pub const OUTPUT_FILE: &str = "THIRD_PARTY_LICENSES_RUST.md";
/// 配る Rust の成果物（パッケージ、機能）。`macos/scripts/build.sh` の `cargo build -p …` と合わせる
const SHIPPED: [(&str, &str); 2] = [("engine-ffi", "lm-metal"), ("shell-ffi", "")];
/// 生成物の先頭の見出し
const TITLE: &str = "# Rust 依存クレートのライセンス";

/// 配るバイナリの Rust 依存のライセンス一覧を作る（cargo-about が要る）
#[derive(Debug, clap::Args)]
pub struct Args {
    /// 書き換えずに、今のファイルが最新かだけを確かめる（古ければ失敗）
    #[arg(long)]
    check: bool,
}

pub fn run(args: Args) -> Result<(), XtaskError> {
    let root = repo_root();
    if !available() {
        return Err(XtaskError(
            "cargo-about がありません（cargo install cargo-about --locked --features cli）".into(),
        ));
    }
    let markdown = generate(&root)?;
    let path = root.join(OUTPUT_FILE);
    if args.check {
        let current = std::fs::read_to_string(&path).unwrap_or_default();
        if current == markdown {
            return Ok(());
        }
        return Err(XtaskError(format!(
            "{OUTPUT_FILE} が古いか、ありません（cargo xtask licenses で作り直してください）"
        )));
    }
    std::fs::write(&path, markdown)
        .map_err(|e| XtaskError(format!("{} に書けません: {e}", path.display())))?;
    eprintln!("{OUTPUT_FILE} を書きました");
    Ok(())
}

/// cargo-about が使えるか
pub fn available() -> bool {
    Command::new("cargo")
        .args(["about", "--version"])
        .output()
        .is_ok_and(|o| o.status.success())
}

/// 同じライセンス本文を持つクレートの集まり
#[derive(Default)]
struct Group {
    id: String,
    /// （名前、版）→ リポジトリ
    crates: BTreeMap<(String, String), String>,
}

fn generate(root: &Path) -> Result<String, XtaskError> {
    // 本文 → 集まり。本文が同じなら（著作権表示も同じなので）まとめる
    let mut groups: BTreeMap<String, Group> = BTreeMap::new();
    for (package, features) in SHIPPED {
        let json = about_json(root, package, features)?;
        let licenses = json
            .get("licenses")
            .and_then(Value::as_array)
            .ok_or_else(|| XtaskError("cargo-about の出力に licenses がありません".into()))?;
        for license in licenses {
            let text = str_field(license, "text");
            let group = groups.entry(text).or_default();
            group.id = str_field(license, "id");
            for used in license
                .get("used_by")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let krate = &used["crate"];
                group.crates.insert(
                    (str_field(krate, "name"), str_field(krate, "version")),
                    str_field(krate, "repository"),
                );
            }
        }
    }
    Ok(render(&groups))
}

fn str_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn about_json(root: &Path, package: &str, features: &str) -> Result<Value, XtaskError> {
    let mut command = Command::new("cargo");
    command
        .current_dir(root)
        .args(["about", "generate", "--locked", "--offline", "--fail"])
        .args(["--format", "json", "--manifest-path"])
        .arg(format!("{package}/Cargo.toml"));
    if !features.is_empty() {
        command.args(["--features", features]);
    }
    let output = command
        .output()
        .map_err(|e| XtaskError(format!("cargo about を起動できません: {e}")))?;
    if !output.status.success() {
        return Err(XtaskError(format!(
            "cargo about が失敗しました（{package}。許可していないライセンスが入った可能性があります。about.toml の accepted を確かめてください）\n{}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|e| XtaskError(format!("cargo about の出力を読めません（{package}）: {e}")))
}

/// Markdown にする。順序は（ライセンスの種類、最初のクレートの名前）で決め、実行のたびに変わらないようにする
fn render(groups: &BTreeMap<String, Group>) -> String {
    let mut sorted: Vec<(&String, &Group)> = groups.iter().collect();
    sorted.sort_by(|a, b| {
        (&a.1.id, a.1.crates.keys().next()).cmp(&(&b.1.id, b.1.crates.keys().next()))
    });
    let mut all: Vec<&(String, String)> = groups.values().flat_map(|g| g.crates.keys()).collect();
    all.sort();
    all.dedup();
    let total = all.len();
    let mut out = String::new();
    out.push_str(TITLE);
    out.push_str("\n\n");
    out.push_str(&format!(
        "ikari.app に入る Rust の成果物（`ikari-engine-ffi`、`ikari-shell-ffi`）が静的に含む依存クレート（{total} 個）のライセンスです。\
         `cargo xtask licenses` で作ります（cargo-about。設定は `about.toml`）。手で書き換えないでください。\n\n\
         ビルドのときだけ使うもの、テストだけで使うもの、配らない道具（cli、dict-builder、評価、xtask）は含みません。\
         同じ本文のライセンスはまとめてあります。\n\n"
    ));
    for (text, group) in sorted {
        out.push_str(&format!("## {}（{} 個）\n\n", group.id, group.crates.len()));
        for ((name, version), repository) in &group.crates {
            if repository.is_empty() {
                out.push_str(&format!("- {name} {version}\n"));
            } else {
                out.push_str(&format!("- {name} {version}（{repository}）\n"));
            }
        }
        out.push_str("\n````text\n");
        out.push_str(text.trim_end());
        out.push_str("\n````\n\n");
    }
    out.truncate(out.trim_end().len());
    out.push('\n');
    out
}
