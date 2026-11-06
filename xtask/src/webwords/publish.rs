//! `cargo xtask webwords publish`: Release の作成と、固定（`scripts/lib.sh`、`THIRD_PARTY_NOTICES.md`）の書き換え。
//!
//! 既定は dry-run（やる予定を表示して終了）。`--yes` のときだけ `gh release create` と書き換えを行う。
//! Release を作る前に、ユーザーに聞く（docs/webwords-update.md）。
//! 組み立て（`release_args`、`release_notes`、`rewrite_*`）は副作用のない関数にして、単体テストで確かめる（テストでは gh を実行しない）。

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::data::run::{Failure, log, repo_root, run as exec};
use crate::data::sha256::sha256_file;

use super::validate_date;

const REPO_SLUG: &str = "ikari-zen/ikari-ime";
const LIB_SH: &str = "scripts/lib.sh";
const NOTICES: &str = "THIRD_PARTY_NOTICES.md";
const URL_KEY: &str = "readonly WEBWORDS_LIST_URL=";
const SHA_KEY: &str = "readonly WEBWORDS_LIST_SHA256=";
/// THIRD_PARTY_NOTICES.md の中で、ダンプの日付を持つ行の目印
const NOTICE_DUMP_MARKER: &str = "データベースダンプ jawiki-";
const PLACEHOLDER_DATE: &str = "<日付>";
const DATE_LEN: usize = 8;
const LICENSE_URL: &str = "https://creativecommons.org/licenses/by-sa/4.0/deed.ja";
const SOURCE_URL: &str = "https://ja.wikipedia.org/";

pub(super) struct Options {
    pub date: String,
    /// `build` の出力先（`webwords-<日付>.tsv`、`.tsv.xz`、`-report.md` がある）
    pub out: PathBuf,
    pub yes: bool,
}

/// Release に置くものの情報
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Facts {
    pub date: String,
    pub sha256: String,
    pub rows: u64,
}

impl Facts {
    pub fn tag(&self) -> String {
        format!("webwords-{}", self.date)
    }
    pub fn asset_name(&self) -> String {
        format!("webwords-{}.tsv.xz", self.date)
    }
    pub fn url(&self) -> String {
        format!(
            "https://github.com/{REPO_SLUG}/releases/download/{}/{}",
            self.tag(),
            self.asset_name()
        )
    }
    pub fn title(&self) -> String {
        format!("外来語の一覧 {}", self.tag())
    }
}

fn group_thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Release のノート（webwords-20261001 と同じ形）
pub(super) fn release_notes(facts: &Facts) -> String {
    format!(
        "ウィキペディア日本語版（jawiki-{date}）から作った、外来語（片仮名の語）の語と出現頻度の一覧。ikari-ime の辞書のビルドが使う。\n\
\n\
- ライセンス: CC BY-SA 4.0（{LICENSE_URL}）\n\
- 出典: ウィキペディア日本語版の執筆者（{SOURCE_URL}）。各記事の執筆者は各記事の履歴を参照\n\
- 加工: 本文から片仮名の連なりを取り出し、語・出現回数・記事の数・区分だけにした。記事の本文、文、記事名、カテゴリ名は含まない\n\
- 行数: {rows} / xz の sha256: {sha}\n\
- 取得: scripts/fetch-data.sh --only webwords\n\
- ADR 0055。Refs #113\n",
        date = facts.date,
        rows = group_thousands(facts.rows),
        sha = facts.sha256
    )
}

/// `gh release create` の引数（`gh` は含まない）
pub(super) fn release_args(facts: &Facts, notes: &str, assets: &[PathBuf]) -> Vec<String> {
    let mut args = vec![
        "release".to_owned(),
        "create".to_owned(),
        facts.tag(),
        "--repo".to_owned(),
        REPO_SLUG.to_owned(),
        "--title".to_owned(),
        facts.title(),
        "--notes".to_owned(),
        notes.to_owned(),
    ];
    args.extend(assets.iter().map(|p| p.display().to_string()));
    args
}

/// `key="..."` の行の値を差し替える。行がちょうど 1 つなければエラー
fn replace_assignment(text: &str, key: &str, value: &str) -> Result<String, Failure> {
    let matches = text.lines().filter(|l| l.starts_with(key)).count();
    if matches != 1 {
        return Err(Failure::new(format!(
            "{LIB_SH} に `{key}` の行が {matches} 個あります（1 個のはず）"
        )));
    }
    let mut out: Vec<String> = text
        .lines()
        .map(|l| {
            if l.starts_with(key) {
                format!("{key}\"{value}\"")
            } else {
                l.to_owned()
            }
        })
        .collect();
    if text.ends_with('\n') {
        out.push(String::new());
    }
    Ok(out.join("\n"))
}

/// `scripts/lib.sh` の URL と sha256 を入れ替える（今が空でも、前の版でも同じ）
pub(super) fn rewrite_lib_sh(text: &str, facts: &Facts) -> Result<String, Failure> {
    let text = replace_assignment(text, URL_KEY, &facts.url())?;
    replace_assignment(&text, SHA_KEY, &facts.sha256)
}

/// `prefix` の直後の日付（8 桁の数字、または `<日付>`）を `date` にする
fn replace_date_after(line: &str, prefix: &str, date: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    while let Some(pos) = rest.find(prefix) {
        let after = pos + prefix.len();
        out.push_str(&rest[..after]);
        out.push_str(date);
        let tail = &rest[after..];
        let skip = if tail.starts_with(PLACEHOLDER_DATE) {
            PLACEHOLDER_DATE.len()
        } else if tail.len() >= DATE_LEN
            && tail.as_bytes()[..DATE_LEN].iter().all(u8::is_ascii_digit)
        {
            DATE_LEN
        } else {
            0
        };
        rest = &tail[skip..];
    }
    out.push_str(rest);
    out
}

/// `THIRD_PARTY_NOTICES.md` の Wikipedia の出典の行のダンプの日付を入れ替える
pub(super) fn rewrite_notices(text: &str, date: &str) -> Result<String, Failure> {
    let matches = text
        .lines()
        .filter(|l| l.contains(NOTICE_DUMP_MARKER))
        .count();
    if matches != 1 {
        return Err(Failure::new(format!(
            "{NOTICES} に `{NOTICE_DUMP_MARKER}` を含む行が {matches} 個あります（1 個のはず）"
        )));
    }
    let mut out: Vec<String> = text
        .lines()
        .map(|l| {
            if l.contains(NOTICE_DUMP_MARKER) {
                let l = replace_date_after(l, "jawiki-", date);
                replace_date_after(&l, "dumps.wikimedia.org/jawiki/", date)
            } else {
                l.to_owned()
            }
        })
        .collect();
    if text.ends_with('\n') {
        out.push(String::new());
    }
    Ok(out.join("\n"))
}

/// 一覧の行数（1 行目のヘッダを除く）
fn count_rows(list: &str) -> u64 {
    list.lines().skip(1).filter(|l| !l.is_empty()).count() as u64
}

/// 一覧のヘッダの `source=jawiki-<日付>` が、指定の日付と合うか
fn check_header(list: &str, date: &str) -> Result<(), Failure> {
    let header = list.lines().next().unwrap_or_default();
    if header.starts_with("#ikari-webwords\tschema=2\t")
        && header.contains(&format!("\tsource=jawiki-{date}\t"))
    {
        Ok(())
    } else {
        Err(Failure::new(format!(
            "一覧のヘッダが jawiki-{date} の schema=2 ではありません: {header}"
        )))
    }
}

fn read(path: &Path) -> Result<String, Failure> {
    std::fs::read_to_string(path)
        .map_err(|e| Failure::new(format!("{} を読めません: {e}", path.display())))
}

fn write(path: &Path, text: &str) -> Result<(), Failure> {
    std::fs::write(path, text)
        .map_err(|e| Failure::new(format!("{} に書けません: {e}", path.display())))
}

pub(super) fn run_publish(opts: Options) -> Result<(), Failure> {
    validate_date(&opts.date)?;
    let date = &opts.date;
    let root = repo_root();
    let xz = opts.out.join(format!("webwords-{date}.tsv.xz"));
    let list_path = opts.out.join(format!("webwords-{date}.tsv"));
    let report = opts.out.join(format!("webwords-{date}-report.md"));
    for path in [&xz, &list_path, &report] {
        if !path.is_file() {
            return Err(Failure::new(format!(
                "{} がありません（先に build / measure を流す）",
                path.display()
            )));
        }
    }
    let list = read(&list_path)?;
    check_header(&list, date)?;
    let facts = Facts {
        date: date.clone(),
        sha256: sha256_file(&xz).map_err(|e| Failure::new(format!("{}: {e}", xz.display())))?,
        rows: count_rows(&list),
    };
    let notes = release_notes(&facts);
    let assets = [xz.clone(), report.clone()];
    let args = release_args(&facts, &notes, &assets);

    // 書き換えは先にメモリの上で作る（失敗するなら Release を作る前に止める）
    let lib_path = root.join(LIB_SH);
    let notices_path = root.join(NOTICES);
    let new_lib = rewrite_lib_sh(&read(&lib_path)?, &facts)?;
    let new_notices = rewrite_notices(&read(&notices_path)?, date)?;

    println!("タグ: {}", facts.tag());
    println!("タイトル: {}", facts.title());
    println!("資産: {} / {}", xz.display(), report.display());
    println!("xz の sha256: {} / 行数: {}", facts.sha256, facts.rows);
    println!("書き換える行:");
    println!("  {LIB_SH}: {URL_KEY}\"{}\"", facts.url());
    println!("  {LIB_SH}: {SHA_KEY}\"{}\"", facts.sha256);
    println!("  {NOTICES}: ダンプの日付を jawiki-{date} にする");
    println!("--- Release のノート ---\n{notes}---");
    if !opts.yes {
        println!(
            "dry-run です。何も変えていません。作るなら --yes（Release の前に、ユーザーに聞く）。"
        );
        return Ok(());
    }

    log("gh release create");
    exec(Command::new("gh").args(&args))?;
    write(&lib_path, &new_lib)?;
    write(&notices_path, &new_notices)?;
    println!(
        "Release を作り、{LIB_SH} と {NOTICES} を書き換えました。次: scripts/fetch-data.sh --only webwords で取れることを確かめ、PR にする。"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> Facts {
        Facts {
            date: "20270201".to_owned(),
            sha256: "ab".repeat(32),
            rows: 1_234_567,
        }
    }

    const LIB: &str = "a=1\nreadonly WEBWORDS_LIST_PATH=\"webwords/webwords.tsv\"\nreadonly WEBWORDS_LIST_URL=\"\"\nreadonly WEBWORDS_LIST_SHA256=\"\"\nz=2\n";

    #[test]
    fn names_and_url() {
        let f = facts();
        assert_eq!(f.tag(), "webwords-20270201");
        assert_eq!(
            f.url(),
            "https://github.com/ikari-zen/ikari-ime/releases/download/webwords-20270201/webwords-20270201.tsv.xz"
        );
    }

    #[test]
    fn thousands_separator() {
        assert_eq!(group_thousands(193_979), "193,979");
        assert_eq!(group_thousands(999), "999");
        assert_eq!(group_thousands(1_000), "1,000");
        assert_eq!(group_thousands(0), "0");
    }

    #[test]
    fn notes_have_license_and_counts() {
        let notes = release_notes(&facts());
        assert!(notes.contains("jawiki-20270201"));
        assert!(notes.contains("CC BY-SA 4.0"));
        assert!(notes.contains("行数: 1,234,567 / xz の sha256: abab"));
        assert!(notes.contains("Refs #113"));
    }

    #[test]
    fn gh_args_are_assembled_without_running() {
        let args = release_args(
            &facts(),
            "N",
            &[PathBuf::from("/o/a.xz"), PathBuf::from("/o/r.md")],
        );
        assert_eq!(
            args,
            [
                "release",
                "create",
                "webwords-20270201",
                "--repo",
                "ikari-zen/ikari-ime",
                "--title",
                "外来語の一覧 webwords-20270201",
                "--notes",
                "N",
                "/o/a.xz",
                "/o/r.md"
            ]
        );
    }

    #[test]
    fn lib_sh_is_rewritten_from_empty_and_from_previous() {
        let f = facts();
        let once = rewrite_lib_sh(LIB, &f).unwrap();
        assert!(once.contains(&format!("readonly WEBWORDS_LIST_URL=\"{}\"\n", f.url())));
        assert!(once.contains(&format!("readonly WEBWORDS_LIST_SHA256=\"{}\"\n", f.sha256)));
        assert!(once.starts_with("a=1\n") && once.ends_with("z=2\n"));
        assert!(once.contains("WEBWORDS_LIST_PATH=\"webwords/webwords.tsv\""));
        let next = Facts {
            date: "20270601".to_owned(),
            ..f
        };
        let twice = rewrite_lib_sh(&once, &next).unwrap();
        assert!(twice.contains("webwords-20270601/webwords-20270601.tsv.xz"));
        assert!(!twice.contains("20270201"));
        assert_eq!(twice.lines().count(), LIB.lines().count());
    }

    #[test]
    fn lib_sh_needs_exactly_one_line_each() {
        assert!(rewrite_lib_sh("a=1\n", &facts()).is_err());
        let dup = format!("{LIB}readonly WEBWORDS_LIST_URL=\"\"\n");
        assert!(rewrite_lib_sh(&dup, &facts()).is_err());
    }

    #[test]
    fn notices_date_is_replaced_from_placeholder_and_from_previous() {
        let text = "x\n- **出典**: ウィキペディア（https://ja.wikipedia.org/）。データベースダンプ jawiki-<日付>（pages-articles。https://dumps.wikimedia.org/jawiki/<日付>/）。各記事\n- 加工: webwords-<日付>.tsv\n";
        let once = rewrite_notices(text, "20270201").unwrap();
        assert!(once.contains("jawiki-20270201（"), "{once}");
        assert!(
            once.contains("dumps.wikimedia.org/jawiki/20270201/）"),
            "{once}"
        );
        assert!(once.contains("webwords-<日付>.tsv"), "他の行は変えない");
        let twice = rewrite_notices(&once, "20270601").unwrap();
        assert!(twice.contains("jawiki-20270601（") && twice.contains("jawiki/20270601/"));
        assert!(!twice.contains("20270201"));
        assert!(rewrite_notices("none\n", "20270201").is_err());
    }

    #[test]
    fn real_files_can_be_rewritten() {
        let root = repo_root();
        let lib = std::fs::read_to_string(root.join(LIB_SH)).unwrap();
        rewrite_lib_sh(&lib, &facts()).unwrap();
        let notices = std::fs::read_to_string(root.join(NOTICES)).unwrap();
        let out = rewrite_notices(&notices, "20270201").unwrap();
        assert!(out.contains("jawiki-20270201"));
    }

    #[test]
    fn header_and_rows() {
        let list = "#ikari-webwords\tschema=2\tsource=jawiki-20270201\tsha256=x\n語\t1\t5\tcommon\n\n語2\t1\t5\tcommon\n";
        check_header(list, "20270201").unwrap();
        assert!(check_header(list, "20270301").is_err());
        assert!(check_header("x\n", "20270201").is_err());
        assert_eq!(count_rows(list), 2);
    }
}
