//! `cargo xtask webwords`（メンテナー用。ADR 0055 §5.1）。Wikipedia 日本語版のダンプから外来語の一覧を作る手順。
//! 利用者の辞書のビルドは `cargo xtask dict` が一覧を使う（§5.3）。ここは一覧を作る側で、利用者は使わない。
//!
//! # 手順（実行はメンテナー。Release に置く操作はここに入れない）
//! 1. `cargo xtask webwords fetch --dump <YYYYMMDD> --work <空のディレクトリ>`
//!    dumps.wikimedia.org から pages-articles（分割の全ファイル）と SQL ダンプ 5 つ（page、redirect、page_props、categorylinks、
//!    linktarget）を、1 ファイルずつ、間に待ちを入れて取る。sha1 を dumpstatus.json と照合し、sha256 と合わせて
//!    `<work>/manifest.tsv` に記録する。`data/raw` には置かない。数十 GB になるので、空きと時間を確かめてから流す。
//! 2. `cargo xtask webwords build --dump <YYYYMMDD> --work <fetch の作業ディレクトリ> --out <出力先>`
//!    `dict-builder webwords collect`、`classify` を流し、`webwords-<日付>.tsv`、`webwords-<日付>-report.md`、
//!    `xz -9e` で圧縮した `webwords-<日付>.tsv.xz` を出力先に作る。最後に xz の sha256 を出す。
//! 3. メンテナーが、レポートを確かめ（理由ごとの件数、抜き取り。ADR 0055 §4.1・§6）、`webwords-<日付>.tsv.xz` を GitHub の Release
//!    （タグ `webwords-<日付>`。ADR 0033 の型）に資産として置く。
//! 4. `scripts/lib.sh` の `WEBWORDS_LIST_URL` に資産の URL を、`WEBWORDS_LIST_SHA256` に手順 2 が出した xz の sha256 を入れる。
//!    `THIRD_PARTY_NOTICES.md` のダンプの日付を直す。`scripts/fetch-data.sh --only webwords` で取れることを確かめる。
//! 5. 作業用のディレクトリを消す（本文と SQL ダンプは配らない。sha256 は一覧のヘッダとレポートに残る）。
//!
//! pages-articles の sha256: manifest の pages-articles の各ファイルの sha256（16 進）を、ファイル名の順につないだ文字列の sha256。

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use clap::Subcommand;
use serde_json::Value;

use crate::XtaskError;
use crate::data::consts::LibConsts;
use crate::data::fetch::tmp_path;
use crate::data::run::{Failure, capture, cargo, log, repo_root, run as exec};
use crate::data::sha256::{sha256_file, sha256_hex};

const DUMPS_BASE: &str = "https://dumps.wikimedia.org";
const WIKI: &str = "jawiki";
const DUMP_STATUS_FILE: &str = "dumpstatus.json";
const MANIFEST_FILE: &str = "manifest.tsv";
/// ダンプの 1 ファイルごとの待ち（Wikimedia のサーバーに礼儀正しく。続けざまに取らない）
const FETCH_INTERVAL: Duration = Duration::from_secs(10);
const USER_AGENT: &str =
    "ikari-ime-xtask (https://github.com/ikari-zen/ikari-ime; maintainer tool)";
const CURL_RETRY: &str = "3";
/// dumpstatus.json の本文の分割の job
const ARTICLES_JOB: &str = "articlesdump";
const ARTICLES_MARKER: &str = "pages-articles";
/// SQL ダンプの job と、ファイル名の `<wiki>-<日付>-<key>.sql.gz` の key（classify の引数の順）
const SQL_JOBS: [(&str, &str); 5] = [
    ("pagetable", "page"),
    ("redirecttable", "redirect"),
    ("pagepropstable", "page_props"),
    ("categorylinkstable", "categorylinks"),
    ("linktargettable", "linktarget"),
];
const BUILDER_PACKAGE: &str = "ikari-dict-builder";
const COLLECTED_HEADER_ARTICLES: &str = "articles=";
const XZ_LEVEL: &str = "-9e";
const DATE_LEN: usize = 8;

#[derive(Debug, clap::Args)]
pub struct Args {
    #[command(subcommand)]
    command: Action,
}

#[derive(Debug, Subcommand)]
enum Action {
    /// dumps.wikimedia.org から pages-articles と SQL ダンプを空の作業用ディレクトリに取る（sha1 を照合し、sha256 を記録する）
    Fetch {
        /// ダンプの日付（例: 20261001）
        #[arg(long, value_name = "YYYYMMDD")]
        dump: String,
        /// 作業用ディレクトリ（空であること。data/raw には置かない）
        #[arg(long, value_name = "DIR")]
        work: PathBuf,
    },
    /// 取ったダンプから collect と classify を流し、`webwords-<日付>.tsv`・レポート・xz を作る
    Build {
        /// ダンプの日付
        #[arg(long, value_name = "YYYYMMDD")]
        dump: String,
        /// `fetch` の作業用ディレクトリ
        #[arg(long, value_name = "DIR")]
        work: PathBuf,
        /// 出力先ディレクトリ
        #[arg(long, value_name = "DIR")]
        out: PathBuf,
        /// data のディレクトリ（Sudachi の lex.csv を読む。既定はリポジトリの data/）
        #[arg(long, value_name = "DIR")]
        data: Option<PathBuf>,
    },
}

pub fn run(args: Args) -> Result<(), XtaskError> {
    let result = match args.command {
        Action::Fetch { dump, work } => run_fetch(&dump, &work),
        Action::Build {
            dump,
            work,
            out,
            data,
        } => run_build(&dump, &work, &out, data),
    };
    result.map_err(|failure| XtaskError(failure.message))
}

/// ダンプの日付（`YYYYMMDD`）の形。URL とファイル名に入るので、数字 8 桁だけを通す
fn validate_date(date: &str) -> Result<(), Failure> {
    if date.len() == DATE_LEN && date.chars().all(|c| c.is_ascii_digit()) {
        Ok(())
    } else {
        Err(Failure::new(format!(
            "--dump は YYYYMMDD（数字 8 桁）です: {date}"
        )))
    }
}

fn status_url(date: &str) -> String {
    format!("{DUMPS_BASE}/{WIKI}/{date}/{DUMP_STATUS_FILE}")
}

/// dumpstatus.json の `url`（`/jawiki/<日付>/<名前>` の形）から、取得先の URL を作る
fn file_url(relative: &str) -> String {
    format!("{DUMPS_BASE}/{}", relative.trim_start_matches('/'))
}

/// 取るファイル 1 つ
#[derive(Debug, Clone, PartialEq, Eq)]
struct DumpFile {
    name: String,
    url: String,
    sha1: String,
}

/// dumpstatus.json から、取るファイルを決める。pages-articles の分割の全ファイル（bz2）と、SQL ダンプ 5 つ。
/// job が完了（done）していない、ファイルがない、sha1 がないときは、取る前にエラーにする。
fn plan_files(status: &Value, date: &str) -> Result<Vec<DumpFile>, Failure> {
    let jobs = status
        .get("jobs")
        .ok_or_else(|| Failure::new("dumpstatus.json に jobs がありません"))?;
    let mut plan = Vec::new();
    let mut add = |job: &str, accept: &dyn Fn(&str) -> bool| -> Result<(), Failure> {
        let entry = jobs
            .get(job)
            .ok_or_else(|| Failure::new(format!("dumpstatus.json に job {job} がありません")))?;
        if entry.get("status").and_then(Value::as_str) != Some("done") {
            return Err(Failure::new(format!(
                "job {job} がまだ完了していません（ダンプ {date}）。別の日付を選んでください"
            )));
        }
        let files = entry
            .get("files")
            .and_then(Value::as_object)
            .ok_or_else(|| Failure::new(format!("job {job} にファイルがありません")))?;
        let mut found = 0;
        for (name, info) in files {
            if !accept(name) {
                continue;
            }
            let field = |key: &str| {
                info.get(key)
                    .and_then(Value::as_str)
                    .filter(|v| !v.is_empty())
                    .ok_or_else(|| Failure::new(format!("{name} に {key} がありません")))
            };
            plan.push(DumpFile {
                name: safe_file_name(name)?,
                url: file_url(field("url")?),
                sha1: field("sha1")?.to_owned(),
            });
            found += 1;
        }
        if found == 0 {
            return Err(Failure::new(format!(
                "job {job} に対象のファイルがありません"
            )));
        }
        Ok(())
    };
    add(ARTICLES_JOB, &|name| {
        name.contains(ARTICLES_MARKER) && name.ends_with(".bz2")
    })?;
    for (job, key) in SQL_JOBS {
        let suffix = format!("-{key}.sql.gz");
        add(job, &|name| name.ends_with(&suffix))?;
    }
    Ok(plan)
}

/// ファイル名は作業用ディレクトリの直下に置くので、区切りや `..` を含むものは通さない
fn safe_file_name(name: &str) -> Result<String, Failure> {
    let bad = name.is_empty()
        || name.contains('/')
        || name.contains('\\')
        || name.contains("..")
        || name.contains('\t')
        || name.contains('\n');
    if bad {
        Err(Failure::new(format!("ファイル名が不正です: {name}")))
    } else {
        Ok(name.to_owned())
    }
}

/// manifest の 1 行（`名前<TAB>sha1<TAB>sha256`）
#[derive(Debug, Clone, PartialEq, Eq)]
struct ManifestEntry {
    name: String,
    sha1: String,
    sha256: String,
}

fn manifest_text(entries: &[ManifestEntry]) -> String {
    entries
        .iter()
        .map(|e| format!("{}\t{}\t{}\n", e.name, e.sha1, e.sha256))
        .collect()
}

fn parse_manifest(text: &str) -> Result<Vec<ManifestEntry>, Failure> {
    text.lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let mut fields = line.split('\t');
            match (fields.next(), fields.next(), fields.next(), fields.next()) {
                (Some(name), Some(sha1), Some(sha256), None) => Ok(ManifestEntry {
                    name: name.to_owned(),
                    sha1: sha1.to_owned(),
                    sha256: sha256.to_owned(),
                }),
                _ => Err(Failure::new(format!(
                    "{MANIFEST_FILE} の行が不正です: {line}"
                ))),
            }
        })
        .collect()
}

/// 一覧のヘッダの sha256: pages-articles の各ファイルの sha256（16 進）を、ファイル名の順につないだ文字列の sha256
fn articles_sha256(entries: &[ManifestEntry]) -> String {
    let mut shas: Vec<(&str, &str)> = entries
        .iter()
        .filter(|e| e.name.contains(ARTICLES_MARKER) && e.name.ends_with(".bz2"))
        .map(|e| (e.name.as_str(), e.sha256.as_str()))
        .collect();
    shas.sort_unstable();
    let joined: String = shas.iter().map(|(_, sha)| *sha).collect();
    sha256_hex(joined.as_bytes())
}

/// ファイルの sha1（`shasum -a 1`、なければ `sha1sum`）
fn sha1_file(path: &Path) -> Result<String, Failure> {
    let mut shasum = Command::new("shasum");
    shasum.args(["-a", "1"]).arg(path);
    let mut sha1sum = Command::new("sha1sum");
    sha1sum.arg(path);
    let out = capture(&mut shasum)
        .or_else(|| capture(&mut sha1sum))
        .ok_or_else(|| Failure::new("shasum（または sha1sum）で sha1 を取れません"))?;
    out.split_whitespace()
        .next()
        .map(str::to_owned)
        .ok_or_else(|| Failure::new("sha1 の出力が空です"))
}

/// ダンプの取得の手段（URL と出力先を受け取る）
type Transfer<'a> = &'a dyn Fn(&str, &Path) -> Result<(), Failure>;

/// curl（Wikimedia に名乗る User-Agent つき）
fn dump_curl(url: &str, out: &Path) -> Result<(), Failure> {
    let mut command = Command::new("curl");
    command
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--retry",
            CURL_RETRY,
            "--user-agent",
            USER_AGENT,
            "--output",
        ])
        .arg(out)
        .arg(url);
    exec(&mut command)
}

/// 計画のファイルを 1 つずつ取る。間に `pause` を入れる（最初の前には入れない）。
/// 各ファイルは一時ファイルに落とし、sha1 が dumpstatus.json と合ったものだけ置き、manifest に追記する。
/// 合わなければそこで止める（置いたファイルと manifest はそのまま残るので、作業用のディレクトリごと消してやり直す）。
fn download_files(
    plan: &[DumpFile],
    work: &Path,
    transfer: Transfer<'_>,
    sha1_of: &dyn Fn(&Path) -> Result<String, Failure>,
    pause: &dyn Fn(),
) -> Result<Vec<ManifestEntry>, Failure> {
    let manifest_path = work.join(MANIFEST_FILE);
    let mut entries = Vec::new();
    for (index, file) in plan.iter().enumerate() {
        if index > 0 {
            pause();
        }
        let dest = work.join(&file.name);
        let tmp = tmp_path(&dest);
        log(&format!("[{}/{}] {}", index + 1, plan.len(), file.url));
        let verified = transfer(&file.url, &tmp).and_then(|()| {
            let got = sha1_of(&tmp)?;
            if got == file.sha1 {
                Ok(())
            } else {
                Err(Failure::new(format!(
                    "sha1 が dumpstatus.json と一致しません: {}（期待 {}、実際 {got}）",
                    file.name, file.sha1
                )))
            }
        });
        if let Err(failure) = verified {
            let _ = std::fs::remove_file(&tmp);
            return Err(failure);
        }
        std::fs::rename(&tmp, &dest)
            .map_err(|e| Failure::new(format!("{} に置けません: {e}", dest.display())))?;
        let sha256 =
            sha256_file(&dest).map_err(|e| Failure::new(format!("{}: {e}", dest.display())))?;
        entries.push(ManifestEntry {
            name: file.name.clone(),
            sha1: file.sha1.clone(),
            sha256,
        });
        std::fs::write(&manifest_path, manifest_text(&entries))
            .map_err(|e| Failure::new(format!("{} を書けません: {e}", manifest_path.display())))?;
    }
    Ok(entries)
}

/// 作業用のディレクトリが、なければ作り、あれば空であること
fn prepare_work_dir(work: &Path) -> Result<(), Failure> {
    if work.as_os_str().is_empty() {
        return Err(Failure::new("--work が空です"));
    }
    if work.exists() {
        let mut entries = std::fs::read_dir(work)
            .map_err(|e| Failure::new(format!("{} を読めません: {e}", work.display())))?;
        if entries.next().is_some() {
            return Err(Failure::new(format!(
                "{} は空ではありません。空のディレクトリを指定してください",
                work.display()
            )));
        }
        Ok(())
    } else {
        std::fs::create_dir_all(work)
            .map_err(|e| Failure::new(format!("{} を作れません: {e}", work.display())))
    }
}

fn run_fetch(date: &str, work: &Path) -> Result<(), Failure> {
    validate_date(date)?;
    prepare_work_dir(work)?;
    let status_path = work.join(DUMP_STATUS_FILE);
    log(&format!("dumpstatus.json を取る（{date}）"));
    dump_curl(&status_url(date), &status_path)?;
    let text = std::fs::read_to_string(&status_path)
        .map_err(|e| Failure::new(format!("{} を読めません: {e}", status_path.display())))?;
    let status: Value = serde_json::from_str(&text)
        .map_err(|e| Failure::new(format!("dumpstatus.json が JSON ではありません: {e}")))?;
    let plan = plan_files(&status, date)?;
    log(&format!(
        "{} ファイルを 1 つずつ取る（間に {} 秒待つ）",
        plan.len(),
        FETCH_INTERVAL.as_secs()
    ));
    let entries = download_files(&plan, work, &dump_curl, &sha1_file, &|| {
        std::thread::sleep(FETCH_INTERVAL);
    })?;
    log(&format!(
        "完了: {}（{} ファイル。記録 {}）",
        work.display(),
        entries.len(),
        work.join(MANIFEST_FILE).display()
    ));
    Ok(())
}

/// `collect` / `classify` に渡すファイルの一式（manifest と作業用ディレクトリから決める）
#[derive(Debug, PartialEq, Eq)]
struct BuildInputs {
    pages_articles: Vec<PathBuf>,
    /// `SQL_JOBS` の順（page、redirect、page_props、categorylinks、linktarget）
    sql: Vec<PathBuf>,
    articles_sha256: String,
}

fn build_inputs(entries: &[ManifestEntry], work: &Path) -> Result<BuildInputs, Failure> {
    let mut pages_articles: Vec<PathBuf> = entries
        .iter()
        .filter(|e| e.name.contains(ARTICLES_MARKER) && e.name.ends_with(".bz2"))
        .map(|e| work.join(&e.name))
        .collect();
    pages_articles.sort();
    if pages_articles.is_empty() {
        return Err(Failure::new("manifest に pages-articles がありません"));
    }
    let mut sql = Vec::new();
    for (_, key) in SQL_JOBS {
        let suffix = format!("-{key}.sql.gz");
        let entry = entries
            .iter()
            .find(|e| e.name.ends_with(&suffix))
            .ok_or_else(|| Failure::new(format!("manifest に {key} の SQL ダンプがありません")))?;
        sql.push(work.join(&entry.name));
    }
    Ok(BuildInputs {
        pages_articles,
        sql,
        articles_sha256: articles_sha256(entries),
    })
}

fn collect_args(inputs: &BuildInputs, out: &Path) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec![
        "webwords".into(),
        "collect".into(),
        "--pages-articles".into(),
    ];
    args.extend(inputs.pages_articles.iter().map(OsString::from));
    args.extend(["--out".into(), out.into()]);
    args
}

/// collect の出力の 1 行目（`#ikari-webwords-collect\tfiles=..\tarticles=N\t…`）から記事数を取る
fn collected_articles(text: &str) -> Result<u64, Failure> {
    let header = text.lines().next().unwrap_or_default();
    header
        .split('\t')
        .find_map(|field| field.strip_prefix(COLLECTED_HEADER_ARTICLES))
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| {
            Failure::new(format!(
                "collect の出力の 1 行目に articles= がありません: {header}"
            ))
        })
}

struct ClassifyPlan<'a> {
    collected: &'a Path,
    inputs: &'a BuildInputs,
    lex: &'a [PathBuf],
    date: &'a str,
    sudachi: &'a str,
    articles: u64,
    out: &'a Path,
    report: &'a Path,
}

fn classify_args(plan: &ClassifyPlan<'_>) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec![
        "webwords".into(),
        "classify".into(),
        "--collected".into(),
        plan.collected.into(),
    ];
    let flags = [
        "--page-sql",
        "--redirect-sql",
        "--page-props-sql",
        "--categorylinks-sql",
        "--linktarget-sql",
    ];
    for (flag, path) in flags.iter().zip(&plan.inputs.sql) {
        args.extend([(*flag).into(), path.into()]);
    }
    for path in plan.lex {
        args.extend(["--lex".into(), path.into()]);
    }
    args.extend([
        "--source".into(),
        format!("jawiki-{}", plan.date).into(),
        "--sha256".into(),
        plan.inputs.articles_sha256.clone().into(),
        "--articles".into(),
        plan.articles.to_string().into(),
        "--sudachi".into(),
        plan.sudachi.into(),
        "--out".into(),
        plan.out.into(),
        "--report".into(),
        plan.report.into(),
    ]);
    args
}

fn run_build(
    date: &str,
    work: &Path,
    out_dir: &Path,
    data: Option<PathBuf>,
) -> Result<(), Failure> {
    validate_date(date)?;
    let root = repo_root();
    let consts = LibConsts::load(&root).map_err(|e| Failure::new(e.to_string()))?;
    let manifest_path = work.join(MANIFEST_FILE);
    let manifest = std::fs::read_to_string(&manifest_path).map_err(|e| {
        Failure::new(format!(
            "{} を読めません（先に fetch を流す）: {e}",
            manifest_path.display()
        ))
    })?;
    let inputs = build_inputs(&parse_manifest(&manifest)?, work)?;

    let data = data.unwrap_or_else(|| root.join("data"));
    let get = |name: &str| consts.get(name).map_err(|e| Failure::new(e.to_string()));
    let sudachi_version = get("SUDACHI_VERSION")?;
    let sudachi_dir = data.join("raw").join(get("SUDACHI_DIR_NAME")?);
    let mut lex: Vec<PathBuf> = ["small_lex.csv", "core_lex.csv", "notcore_lex.csv"]
        .iter()
        .map(|name| sudachi_dir.join(name))
        .collect();
    // ABR の町名の lex があれば、辞書のビルドと同じく既知の語の照合に使う（なくても作れる）
    let town_lex = data.join("build").join(get("ABR_TOWN_LEX_FILE")?);
    if town_lex.is_file() {
        lex.push(town_lex);
    }
    for path in &lex {
        if !path.is_file() {
            return Err(Failure::new(format!(
                "{} がありません（scripts/fetch-data.sh で取得する）",
                path.display()
            )));
        }
    }

    std::fs::create_dir_all(out_dir)
        .map_err(|e| Failure::new(format!("{} を作れません: {e}", out_dir.display())))?;
    let collected = out_dir.join(format!("collected-{date}.tsv"));
    let list = out_dir.join(format!("webwords-{date}.tsv"));
    let report = out_dir.join(format!("webwords-{date}-report.md"));

    log("ikari-dict-builder をビルド");
    exec(cargo(&root).args(["build", "--release", "-p", BUILDER_PACKAGE]))?;
    let builder = root.join("target").join("release").join(format!(
        "ikari-dict-builder{}",
        std::env::consts::EXE_SUFFIX
    ));

    log("collect（本文から片仮名の連なりを数える。時間がかかる）");
    exec(Command::new(&builder).args(collect_args(&inputs, &collected)))?;
    let collected_text = std::fs::read_to_string(&collected)
        .map_err(|e| Failure::new(format!("{} を読めません: {e}", collected.display())))?;
    let articles = collected_articles(&collected_text)?;
    drop(collected_text);

    log("classify（区分を付けて schema=2 の一覧を作る）");
    exec(Command::new(&builder).args(classify_args(&ClassifyPlan {
        collected: &collected,
        inputs: &inputs,
        lex: &lex,
        date,
        sudachi: sudachi_version,
        articles,
        out: &list,
        report: &report,
    })))?;

    log("xz で圧縮する");
    exec(
        Command::new("xz")
            .args([XZ_LEVEL, "--keep", "--force"])
            .arg(&list),
    )?;
    let mut xz_name = list.clone().into_os_string();
    xz_name.push(".xz");
    let xz = PathBuf::from(xz_name);
    let xz_sha = sha256_file(&xz).map_err(|e| Failure::new(format!("{}: {e}", xz.display())))?;
    println!("一覧: {}", list.display());
    println!("レポート: {}", report.display());
    println!("資産（Release に置く）: {}", xz.display());
    println!("WEBWORDS_LIST_SHA256={xz_sha}");
    println!(
        "次の手順は xtask/src/webwords.rs の冒頭の 3〜5（Release に置く操作はメンテナーが行う）"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use clap::Parser;
    use serde_json::json;

    use super::*;

    #[derive(Debug, Parser)]
    struct Cli {
        #[command(flatten)]
        args: Args,
    }

    fn status() -> Value {
        let file = |name: &str, sha1: &str| json!({ "size": 1, "url": format!("/jawiki/20261001/{name}"), "sha1": sha1, "md5": "m" });
        json!({ "jobs": {
            "articlesdump": { "status": "done", "files": {
                "jawiki-20261001-pages-articles1.xml-p1p2.bz2": file("jawiki-20261001-pages-articles1.xml-p1p2.bz2", "a1"),
                "jawiki-20261001-pages-articles2.xml-p3p4.bz2": file("jawiki-20261001-pages-articles2.xml-p3p4.bz2", "a2"),
            }},
            "pagetable": { "status": "done", "files": { "jawiki-20261001-page.sql.gz": file("jawiki-20261001-page.sql.gz", "s1") }},
            "redirecttable": { "status": "done", "files": { "jawiki-20261001-redirect.sql.gz": file("jawiki-20261001-redirect.sql.gz", "s2") }},
            "pagepropstable": { "status": "done", "files": { "jawiki-20261001-page_props.sql.gz": file("jawiki-20261001-page_props.sql.gz", "s3") }},
            "categorylinkstable": { "status": "done", "files": { "jawiki-20261001-categorylinks.sql.gz": file("jawiki-20261001-categorylinks.sql.gz", "s4") }},
            "linktargettable": { "status": "done", "files": { "jawiki-20261001-linktarget.sql.gz": file("jawiki-20261001-linktarget.sql.gz", "s5") }},
        }})
    }

    #[test]
    fn subcommands_parse() {
        let fetch =
            Cli::try_parse_from(["x", "fetch", "--dump", "20261001", "--work", "w"]).unwrap();
        assert!(matches!(fetch.args.command, Action::Fetch { .. }));
        let build = Cli::try_parse_from([
            "x", "build", "--dump", "20261001", "--work", "w", "--out", "o",
        ])
        .unwrap();
        assert!(matches!(build.args.command, Action::Build { .. }));
    }

    #[test]
    fn date_is_eight_digits() {
        assert!(validate_date("20261001").is_ok());
        for bad in ["2026100", "202610011", "2026-10-1", "../etc/x", ""] {
            assert!(validate_date(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn urls_are_built_from_the_dump_status() {
        assert_eq!(
            status_url("20261001"),
            "https://dumps.wikimedia.org/jawiki/20261001/dumpstatus.json"
        );
        assert_eq!(
            file_url("/jawiki/20261001/x.bz2"),
            "https://dumps.wikimedia.org/jawiki/20261001/x.bz2"
        );
    }

    #[test]
    fn plan_has_all_split_articles_and_five_sql_dumps() {
        let plan = plan_files(&status(), "20261001").unwrap();
        assert_eq!(plan.len(), 7);
        assert_eq!(plan[0].name, "jawiki-20261001-pages-articles1.xml-p1p2.bz2");
        assert_eq!(
            plan[0].url,
            "https://dumps.wikimedia.org/jawiki/20261001/jawiki-20261001-pages-articles1.xml-p1p2.bz2"
        );
        assert_eq!(plan[0].sha1, "a1");
        assert_eq!(plan[2].name, "jawiki-20261001-page.sql.gz");
        assert_eq!(plan[6].name, "jawiki-20261001-linktarget.sql.gz");
    }

    #[test]
    fn plan_rejects_unfinished_missing_and_unsafe() {
        let mut unfinished = status();
        unfinished["jobs"]["pagetable"]["status"] = json!("in-progress");
        assert!(plan_files(&unfinished, "20261001").is_err());
        let mut missing = status();
        missing["jobs"]
            .as_object_mut()
            .unwrap()
            .remove("linktargettable");
        assert!(plan_files(&missing, "20261001").is_err());
        let mut unsafe_name = status();
        unsafe_name["jobs"]["pagetable"]["files"] = json!({
            "../evil-page.sql.gz": { "url": "/x", "sha1": "s" }
        });
        assert!(plan_files(&unsafe_name, "20261001").is_err());
        assert!(plan_files(&json!({}), "20261001").is_err());
    }

    fn entries() -> Vec<ManifestEntry> {
        let e = |name: &str, sha256: &str| ManifestEntry {
            name: name.to_owned(),
            sha1: "x".to_owned(),
            sha256: sha256.to_owned(),
        };
        vec![
            e("jawiki-20261001-pages-articles2.xml-p3p4.bz2", "bb"),
            e("jawiki-20261001-pages-articles1.xml-p1p2.bz2", "aa"),
            e("jawiki-20261001-page.sql.gz", "p"),
            e("jawiki-20261001-redirect.sql.gz", "r"),
            e("jawiki-20261001-page_props.sql.gz", "pp"),
            e("jawiki-20261001-categorylinks.sql.gz", "c"),
            e("jawiki-20261001-linktarget.sql.gz", "l"),
        ]
    }

    #[test]
    fn manifest_roundtrips_and_rejects_bad_lines() {
        let entries = entries();
        assert_eq!(parse_manifest(&manifest_text(&entries)).unwrap(), entries);
        assert!(parse_manifest("only-name\n").is_err());
    }

    #[test]
    fn articles_sha256_is_sha256_of_hashes_in_name_order() {
        // ファイル名の順（articles1 -> articles2）に "aa" "bb" をつなぐ
        assert_eq!(articles_sha256(&entries()), sha256_hex(b"aabb"));
    }

    #[test]
    fn collect_and_classify_args() {
        let work = Path::new("w");
        let inputs = build_inputs(&entries(), work).unwrap();
        assert_eq!(inputs.sql.len(), 5);
        assert!(inputs.sql[0].ends_with("jawiki-20261001-page.sql.gz"));
        let collect: Vec<String> = collect_args(&inputs, Path::new("c.tsv"))
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(&collect[..3], ["webwords", "collect", "--pages-articles"]);
        assert!(collect[3].ends_with("pages-articles1.xml-p1p2.bz2"));
        assert!(collect[4].ends_with("pages-articles2.xml-p3p4.bz2"));
        assert_eq!(&collect[5..], ["--out", "c.tsv"]);

        let lex = [PathBuf::from("small.csv")];
        let classify: Vec<String> = classify_args(&ClassifyPlan {
            collected: Path::new("c.tsv"),
            inputs: &inputs,
            lex: &lex,
            date: "20261001",
            sudachi: "20260723",
            articles: 930000,
            out: Path::new("o.tsv"),
            report: Path::new("r.md"),
        })
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
        let at = |flag: &str| {
            let i = classify.iter().position(|a| a == flag).unwrap();
            classify[i + 1].clone()
        };
        assert!(at("--page-sql").ends_with("-page.sql.gz"));
        assert!(at("--redirect-sql").ends_with("-redirect.sql.gz"));
        assert!(at("--page-props-sql").ends_with("-page_props.sql.gz"));
        assert!(at("--categorylinks-sql").ends_with("-categorylinks.sql.gz"));
        assert!(at("--linktarget-sql").ends_with("-linktarget.sql.gz"));
        assert_eq!(at("--source"), "jawiki-20261001");
        assert_eq!(at("--sha256"), sha256_hex(b"aabb"));
        assert_eq!(at("--articles"), "930000");
        assert_eq!(at("--sudachi"), "20260723");
        assert_eq!(at("--lex"), "small.csv");
    }

    #[test]
    fn build_inputs_needs_every_file() {
        let mut partial = entries();
        partial.retain(|e| !e.name.ends_with("-linktarget.sql.gz"));
        assert!(build_inputs(&partial, Path::new("w")).is_err());
        assert!(build_inputs(&[], Path::new("w")).is_err());
    }

    #[test]
    fn collected_header_gives_article_count() {
        assert_eq!(
            collected_articles("#ikari-webwords-collect\tfiles=2\tarticles=930123\tskipped=4\tmin_df=5\nx\t1\t1\t0\n")
                .unwrap(),
            930123
        );
        assert!(collected_articles("#other\n").is_err());
    }

    #[test]
    fn work_dir_must_be_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(prepare_work_dir(dir.path()).is_ok());
        assert!(prepare_work_dir(&dir.path().join("new")).is_ok());
        std::fs::write(dir.path().join("a"), "x").unwrap();
        assert!(prepare_work_dir(dir.path()).is_err());
        assert!(prepare_work_dir(Path::new("")).is_err());
    }

    fn plan2() -> Vec<DumpFile> {
        ["a.bz2", "b.bz2", "c.bz2"]
            .iter()
            .map(|name| DumpFile {
                name: (*name).to_owned(),
                url: format!("https://x/{name}"),
                sha1: "good".to_owned(),
            })
            .collect()
    }

    #[test]
    fn download_is_one_at_a_time_with_pauses_and_records_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let log = RefCell::new(Vec::new());
        let pauses = Cell::new(0);
        let transfer = |url: &str, out: &Path| -> Result<(), Failure> {
            log.borrow_mut().push(url.to_owned());
            std::fs::write(out, url).map_err(|e| Failure::new(e.to_string()))
        };
        let sha1_of = |_: &Path| -> Result<String, Failure> { Ok("good".to_owned()) };
        let entries = download_files(&plan2(), dir.path(), &transfer, &sha1_of, &|| {
            pauses.set(pauses.get() + 1);
        })
        .unwrap();
        assert_eq!(
            *log.borrow(),
            ["https://x/a.bz2", "https://x/b.bz2", "https://x/c.bz2"]
        );
        // 最初の前には待たない
        assert_eq!(pauses.get(), 2);
        assert_eq!(entries.len(), 3);
        let manifest = std::fs::read_to_string(dir.path().join(MANIFEST_FILE)).unwrap();
        assert_eq!(parse_manifest(&manifest).unwrap(), entries);
        assert!(dir.path().join("c.bz2").is_file());
    }

    #[test]
    fn sha1_mismatch_stops_and_leaves_no_partial_file() {
        let dir = tempfile::tempdir().unwrap();
        let transfer = |_: &str, out: &Path| -> Result<(), Failure> {
            std::fs::write(out, "x").map_err(|e| Failure::new(e.to_string()))
        };
        let sha1_of = |_: &Path| -> Result<String, Failure> { Ok("bad".to_owned()) };
        let err = download_files(&plan2(), dir.path(), &transfer, &sha1_of, &|| {}).unwrap_err();
        assert!(
            err.message
                .contains("sha1 が dumpstatus.json と一致しません"),
            "{err}"
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}
