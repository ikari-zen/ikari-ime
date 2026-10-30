//! `cargo xtask data fetch`（ADR 0043 段 X3）。辞書とモデルの元データを `data/raw/` に取得する。`scripts/fetch-data.sh` の移し先。
//!
//! 取得物（URL、版、sha256 はこのファイルの表に固定している。ダウンロード後に sha256 を確かめ、合わなければ捨てる）。
//! 通信するのは `--check` を付けないときだけ（`--check` は通信せず、あるべきファイルがあり sha256 が合っているかだけを見る）。
//! ライセンスの確認は人が行う（spec）。ここは確認済みの配布元だけを固定する。
//!
//! - sudachi: SudachiDict の small_lex / core_lex / notcore_lex / matrix.def（zip）。small + core が Core、notcore を足したものが Full
//!   （ADR 0012 §11.3）。配布元 sudachi.s3-website-ap-northeast-1.amazonaws.com。Apache License 2.0（WorksApplications）
//! - chive: chiVe 1.3 mc90（tar.gz）。Apache License 2.0（Works Applications）
//! - zenz: zenz-v3.2-small Q5_K_M（GGUF）。モデルカードでは Apache-2.0。派生物の扱いは THIRD_PARTY_NOTICES.md
//! - emoji: iamcal/emoji-data の emoji.json（コミット固定。画像は取らない。ADR 0027 §5.2）。MIT
//! - abr: デジタル庁 ABR の町字マスター mt_town_all.csv（zip。ADR 0030）。公共データ利用規約（PDL1.0）。
//!   配布元は日付で更新される（版番号はない）。sha256 が合わなければ捨てるので、更新されたら ADR 0030 の手順で測り直して下の表を直す
//! - postal: ABR の町字・郵便番号変換表 abr_post_code.csv（zip。ADR 0032 §7）。配布元は月 1 回ほど更新される。同上
//! - ajimee: AJIMEE-Bench の評価データ（コミット固定）。CC BY-SA 3.0。リポジトリには入れない
//!
//! ダウンロードは一時ファイルに書き、sha256 を確かめてから置く。すでに正しいファイルがあれば何もしない。
//! ダウンロードは `curl`、zip と tar.gz の展開は `unzip`（なければ `tar`。macOS と Windows 10 以降の tar は zip も展開できる）と `tar` を使う。

use std::path::{Path, PathBuf};
use std::process::Command;

use super::consts::LibConsts;
use super::non_empty;
use super::run::{Failure, log, repo_root, run as exec, succeeds_quietly};
use super::sha256::sha256_file;

const SUDACHI_RAW_URL: &str =
    "http://sudachi.s3-website-ap-northeast-1.amazonaws.com/sudachidict-raw";
const CHIVE_URL_BASE: &str = "https://sudachi.s3.ap-northeast-1.amazonaws.com/chive";
const EMOJI_URL_BASE: &str = "https://raw.githubusercontent.com/iamcal/emoji-data";
const ABR_URL: &str = "https://data.address-br.digital.go.jp/mt_town/mt_town_all.csv.zip";
const POSTAL_URL_BASE: &str = "https://www.arcgis.com/sharing/rest/content/items";
const ZENZ_URL: &str =
    "https://huggingface.co/Miwa-Keita/zenz-v3.2-small-gguf/resolve/main/ggml-model-Q5_K_M.gguf";

// sha256（ダウンロードしたファイルと、展開したあとのファイル。2026-10-12 に手元の data/raw/ から取った）
const SHA_SMALL_LEX_ZIP: &str = "b578ac9545899783d5d7e30d5d78d5d9dcf40b36965d4af0663ec2eb041c1093";
const SHA_CORE_LEX_ZIP: &str = "a2b39e1572adab08a649b1390b134517adc55f1d733c59358b113298788bf31c";
const SHA_NOTCORE_LEX_ZIP: &str =
    "a15edc5193b42acfebdc220825d8c7edf0059bf950169d09bcdf6c426a81ecd1";
const SHA_MATRIX_ZIP: &str = "3ccaed582481b31be170c7200cf1cb2856f2d14c2fa601b3692d97dbad5a3bd1";
const SHA_SMALL_LEX_CSV: &str = "6cc54808a2359b48fe15105814ca3352dceb4c25305f6956b7a104f7f6611f00";
const SHA_CORE_LEX_CSV: &str = "3a3c4b68cf0b64893dff6cc14ba690d79a99fefe1564dd751a418317db2e148d";
const SHA_NOTCORE_LEX_CSV: &str =
    "6237df5751fea79e638dba54d088520ec3e2386209971e737c6ecc5d69f24950";
const SHA_MATRIX_DEF: &str = "b20e0da1ed5472e0be2e5cb3c716672fd788e1701e08d54624b58ba52d73d927";
const SHA_CHIVE_TAR: &str = "885c7db3b8cd8ad1311ac32eafc874007f45010791b3c1f1e934a2aa0c7d22b0";
const SHA_CHIVE_TXT: &str = "bd1807016d6bf1813021bc9f12b32e71a2ff1121b1fefe0b1b55ddab345503e0";
const SHA_ZENZ_SMALL: &str = "29c223d4c23327b80fd13ebb5ab2555057a46317997d5da391584ffbef0db673";
/// 2026-10-18 に取得
const SHA_EMOJI_JSON: &str = "8736d721991e48c41c57acc46b65e5d32258a45e4bcdebcc6012f420f81e775d";
/// 2026-10-25 に取得（2026-10-22 の d613bb1c… から更新。辞書は同一: ADR 0030 §14）
const SHA_ABR_ZIP: &str = "3593307e88b1491cba8dc3b90c224d1c8d4ac54b02bb3532e15ea830ce84d10a";
const SHA_ABR_CSV: &str = "9238298a7dfb237e22207b89b3a25d63c4437f7d3ded8db3b509068c7f74ca9a";
/// 2026-10-23 に取得（郵便番号データ版 2609、更新日 2026-09-24）
const SHA_POSTAL_ZIP: &str = "45e96e61840f070e2185ae5a4f01c7b8a169d82fb6e1c71dfd25cc8f0fe4b8f8";
const SHA_POSTAL_CSV: &str = "7d9a97984b25110fb6fdcadd5239970f4d038877ec0121252f69770451a6ce92";
const SHA_AJIMEE_ITEMS: &str = "e9eb668fd6aa14b1e26436f429b5550108af0a1dfd443b8cea0bcb3ab3028fca";

/// 再現性のため、取得するコミットを固定する（2025-01-13 時点の main）
const AJIMEE_COMMIT: &str = "401666cd56d1a570c2021798b64b6da4396bfd45";
const AJIMEE_PATH: &str = "JWTD_v2/v1/evaluation_items.json";
const AJIMEE_RAW_URL: &str = "https://raw.githubusercontent.com/azooKey/AJIMEE-Bench";
const AJIMEE_DIR_NAME: &str = "ajimee";
const AJIMEE_ITEMS_FILE: &str = "evaluation_items.json";
const AJIMEE_README_FILE: &str = "README.md";
const AJIMEE_LICENSE_FILE: &str = "LICENSE.txt";
/// 評価データの各行に要るキー
const AJIMEE_REQUIRED_KEYS: [&str; 2] = ["input", "expected_output"];

const CURL_RETRY: &str = "3";
const GROUPS: [&str; 7] = [
    "sudachi", "chive", "zenz", "ajimee", "emoji", "abr", "postal",
];

/// 辞書とモデルの元データを data/raw/ に取得する
#[derive(Debug, clap::Args)]
pub struct Args {
    /// 出力先（既定: このリポジトリの data/）。data/raw/ の下に置く
    #[arg(long)]
    data: Option<PathBuf>,
    /// 指定したものだけ取得する（複数指定できる。既定はすべて）。sudachi / chive / zenz / ajimee / emoji / abr / postal
    #[arg(long)]
    only: Vec<String>,
    /// 通信せず、あるべきファイルがあり sha256 が合っているかだけを確かめる（足りなければ終了コード 1）
    #[arg(long)]
    check: bool,
    /// AJIMEE の出力先（既定: <data>/raw/ajimee。eval/scripts/fetch_ajimee.sh の IKARI_AJIMEE_DIR 用）
    #[arg(long)]
    ajimee_dir: Option<PathBuf>,
}

pub fn run(args: Args) -> Result<(), Failure> {
    let root = repo_root();
    let consts = LibConsts::load(&root).map_err(|e| Failure::new(e.to_string()))?;
    let data = args.data.unwrap_or_else(|| root.join("data"));
    non_empty(&data, "--data")?;
    for name in &args.only {
        if !GROUPS.contains(&name.as_str()) {
            return Err(Failure::new(format!(
                "--only は {} のどれかです",
                GROUPS.join(" / ")
            )));
        }
    }
    let mut fetcher = Fetcher {
        raw: data.join("raw"),
        consts: &consts,
        only: args.only,
        check_only: args.check,
        ajimee_dir: args.ajimee_dir,
        missing: false,
    };
    println!("data: {}", data.display());
    fetcher.run_groups()?;
    if fetcher.missing {
        // シェルと同じ終了コード 1（メッセージなし。MISSING の行が出ている）
        std::process::exit(1);
    }
    Ok(())
}

struct Fetcher<'a> {
    raw: PathBuf,
    consts: &'a LibConsts,
    only: Vec<String>,
    check_only: bool,
    ajimee_dir: Option<PathBuf>,
    missing: bool,
}

/// 正しいファイルがあるか
fn has_valid(path: &Path, sha256: &str) -> bool {
    path.is_file() && sha256_file(path).is_ok_and(|got| got == sha256)
}

/// 一時ファイルに落とし、sha256 を確かめてから置く。
fn download(url: &str, dest: &Path, want: &str) -> Result<(), Failure> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| Failure::new(format!("{} を作れません: {e}", parent.display())))?;
    }
    let tmp = tmp_path(dest);
    log(&format!("ダウンロード: {url}"));
    let result = curl(url, &tmp).and_then(|()| {
        let got = sha256_file(&tmp)
            .map_err(|e| Failure::new(format!("{} を読めません: {e}", tmp.display())))?;
        if got == want {
            Ok(())
        } else {
            Err(Failure::new(format!(
                "sha256 が一致しません: {url}（期待 {want}、実際 {got}）。配布元が更新された可能性があります"
            )))
        }
    });
    if let Err(failure) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(failure);
    }
    std::fs::rename(&tmp, dest).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        Failure::new(format!("{} に置けません: {e}", dest.display()))
    })
}

fn tmp_path(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".tmp.{}", std::process::id()));
    dest.with_file_name(name)
}

fn curl(url: &str, out: &Path) -> Result<(), Failure> {
    let mut command = Command::new("curl");
    command
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--retry",
            CURL_RETRY,
            "--output",
        ])
        .arg(out)
        .arg(url);
    exec(&mut command).map_err(|failure| {
        Failure::new(format!(
            "ダウンロードに失敗しました: {url}（{}）",
            failure.message
        ))
    })
}

/// zip を展開する。`unzip` があればそれを、なければ `tar`（bsdtar は zip も展開できる）を使う。
fn extract_zip(zip: &Path, dir: &Path) -> Result<(), Failure> {
    if succeeds_quietly(Command::new("unzip").arg("-v")) {
        exec(
            Command::new("unzip")
                .args(["-o", "-q"])
                .arg(zip)
                .arg("-d")
                .arg(dir),
        )
    } else {
        exec(Command::new("tar").arg("-xf").arg(zip).arg("-C").arg(dir))
    }
}

fn extract_tar_gz(tar: &Path, dir: &Path) -> Result<(), Failure> {
    exec(Command::new("tar").arg("-xzf").arg(tar).arg("-C").arg(dir))
}

impl Fetcher<'_> {
    fn wanted(&self, name: &str) -> bool {
        self.only.is_empty() || self.only.iter().any(|o| o == name)
    }

    fn get(&self, name: &str) -> Result<&str, Failure> {
        self.consts
            .get(name)
            .map_err(|e| Failure::new(e.to_string()))
    }

    fn run_groups(&mut self) -> Result<(), Failure> {
        if !self.check_only && !succeeds_quietly(Command::new("curl").arg("--version")) {
            return Err(Failure::new("curl がありません"));
        }
        if self.wanted("sudachi") {
            self.fetch_sudachi()?;
        }
        if self.wanted("chive") {
            self.fetch_chive()?;
        }
        if self.wanted("zenz") {
            self.fetch_zenz()?;
        }
        if self.wanted("emoji") {
            self.fetch_emoji()?;
        }
        if self.wanted("abr") {
            self.fetch_abr()?;
        }
        if self.wanted("postal") {
            self.fetch_postal()?;
        }
        if self.wanted("ajimee") {
            self.fetch_ajimee()?;
        }
        Ok(())
    }

    fn ok(path: &Path) {
        println!("ok         {}", path.display());
    }

    fn report_missing(&mut self, path: &Path) {
        println!("MISSING    {}", path.display());
        self.missing = true;
    }

    /// 足りなければ取得する（`--check` のときは足りないことを報告するだけ）
    fn ensure(&mut self, path: &Path, sha256: &str, url: &str) -> Result<(), Failure> {
        if has_valid(path, sha256) {
            Self::ok(path);
        } else if self.check_only {
            self.report_missing(path);
        } else {
            download(url, path, sha256)?;
            println!("downloaded {}", path.display());
        }
        Ok(())
    }

    /// 展開後のファイルが正しければ飛ばし、なければ（zip を取って）展開する。
    fn ensure_extracted(
        &mut self,
        out: &Path,
        out_sha: &str,
        archive: &Path,
        archive_sha: &str,
        url: &str,
        extract: fn(&Path, &Path) -> Result<(), Failure>,
    ) -> Result<(), Failure> {
        if has_valid(out, out_sha) {
            Self::ok(out);
            return Ok(());
        }
        if self.check_only {
            self.report_missing(out);
            return Ok(());
        }
        self.ensure(archive, archive_sha, url)?;
        log(&format!("展開: {}", archive.display()));
        let dir = archive.parent().unwrap_or(Path::new("."));
        extract(archive, dir)?;
        let name = out.file_name().unwrap_or_default().to_string_lossy();
        if !has_valid(out, out_sha) {
            return Err(Failure::new(format!(
                "展開した {name} の sha256 が一致しません"
            )));
        }
        println!("extracted  {}", out.display());
        Ok(())
    }

    fn fetch_sudachi(&mut self) -> Result<(), Failure> {
        let version = self.get("SUDACHI_VERSION")?.to_owned();
        let dir = self.raw.join(self.get("SUDACHI_DIR_NAME")?);
        // lex は版のディレクトリの下、matrix.def.zip は版に依らず sudachidict-raw/ の直下にある（matrix.def は 2019 年から変わらない）
        let rows = [
            (
                "small_lex",
                SHA_SMALL_LEX_ZIP,
                "small_lex.csv",
                SHA_SMALL_LEX_CSV,
            ),
            (
                "core_lex",
                SHA_CORE_LEX_ZIP,
                "core_lex.csv",
                SHA_CORE_LEX_CSV,
            ),
            (
                "notcore_lex",
                SHA_NOTCORE_LEX_ZIP,
                "notcore_lex.csv",
                SHA_NOTCORE_LEX_CSV,
            ),
            ("matrix.def", SHA_MATRIX_ZIP, "matrix.def", SHA_MATRIX_DEF),
        ];
        for (name, zip_sha, csv, csv_sha) in rows {
            let url = if name == "matrix.def" {
                format!("{SUDACHI_RAW_URL}/matrix.def.zip")
            } else {
                format!("{SUDACHI_RAW_URL}/{version}/{name}.zip")
            };
            self.ensure_extracted(
                &dir.join(csv),
                csv_sha,
                &dir.join(format!("{name}.zip")),
                zip_sha,
                &url,
                extract_zip,
            )?;
        }
        Ok(())
    }

    fn fetch_chive(&mut self) -> Result<(), Failure> {
        let name = self.get("CHIVE_NAME")?.to_owned();
        let dir = self.raw.join("chive");
        let txt = dir.join(&name).join(format!("{name}.txt"));
        self.ensure_extracted(
            &txt,
            SHA_CHIVE_TXT,
            &dir.join(format!("{name}.tar.gz")),
            SHA_CHIVE_TAR,
            &format!("{CHIVE_URL_BASE}/{name}.tar.gz"),
            extract_tar_gz,
        )
    }

    fn fetch_zenz(&mut self) -> Result<(), Failure> {
        let path = self.raw.join("zenz").join(self.get("ZENZ_MODEL_FILE")?);
        self.ensure(&path, SHA_ZENZ_SMALL, ZENZ_URL)
    }

    fn fetch_emoji(&mut self) -> Result<(), Failure> {
        let path = self.raw.join(self.get("EMOJI_DATA_JSON")?);
        let url = format!(
            "{EMOJI_URL_BASE}/{}/emoji.json",
            self.get("EMOJI_DATA_COMMIT")?
        );
        self.ensure(&path, SHA_EMOJI_JSON, &url)
    }

    fn fetch_abr(&mut self) -> Result<(), Failure> {
        let dir = self.raw.join(self.get("ABR_DIR_NAME")?);
        let csv = self.raw.join(self.get("ABR_TOWN_CSV")?);
        let zip = dir.join(format!(
            "{}.zip",
            csv.file_name().unwrap_or_default().to_string_lossy()
        ));
        self.ensure_extracted(&csv, SHA_ABR_CSV, &zip, SHA_ABR_ZIP, ABR_URL, extract_zip)
    }

    fn fetch_postal(&mut self) -> Result<(), Failure> {
        let dir = self.raw.join(self.get("ABR_DIR_NAME")?);
        let csv = self.raw.join(self.get("ABR_POSTAL_CSV")?);
        let url = format!("{POSTAL_URL_BASE}/{}/data", self.get("ABR_POSTAL_ITEM_ID")?);
        self.ensure_extracted(
            &csv,
            SHA_POSTAL_CSV,
            &dir.join("abr_post_code.zip"),
            SHA_POSTAL_ZIP,
            &url,
            extract_zip,
        )
    }

    fn fetch_ajimee(&mut self) -> Result<(), Failure> {
        let dir = self
            .ajimee_dir
            .clone()
            .unwrap_or_else(|| self.raw.join(AJIMEE_DIR_NAME));
        let items = dir.join(AJIMEE_ITEMS_FILE);
        if has_valid(&items, SHA_AJIMEE_ITEMS) {
            Self::ok(&items);
        } else if self.check_only {
            self.report_missing(&items);
        } else {
            fetch_ajimee_files(&dir)?;
            if !has_valid(&items, SHA_AJIMEE_ITEMS) {
                return Err(Failure::new(
                    "AJIMEE の evaluation_items.json の sha256 が一致しません",
                ));
            }
        }
        Ok(())
    }
}

/// AJIMEE-Bench の評価データを取得する（ADR 0002 §15.2, §21.1 #4）。`eval/scripts/fetch_ajimee.sh` の移し先。
/// データ（JWTD_v2/v1/evaluation_items.json）は CC BY-SA 3.0。リポジトリには入れない。
/// 取得後の sha256 の確認は呼び出し側（`fetch_ajimee`）が行う。
fn fetch_ajimee_files(dir: &Path) -> Result<(), Failure> {
    std::fs::create_dir_all(dir)
        .map_err(|e| Failure::new(format!("{} を作れません: {e}", dir.display())))?;
    let items = dir.join(AJIMEE_ITEMS_FILE);
    let url = |path: &str| format!("{AJIMEE_RAW_URL}/{AJIMEE_COMMIT}/{path}");
    // 途中で切れたファイルを残さないよう、一時ファイルに落としてから移す
    for (path, dest) in [
        (AJIMEE_PATH, items.clone()),
        ("README.md", dir.join(AJIMEE_README_FILE)),
    ] {
        let tmp = tmp_path(&dest);
        let result = curl(&url(path), &tmp).and_then(|()| {
            std::fs::rename(&tmp, &dest)
                .map_err(|e| Failure::new(format!("{} に置けません: {e}", dest.display())))
        });
        if let Err(failure) = result {
            let _ = std::fs::remove_file(&tmp);
            return Err(failure);
        }
    }
    let text = std::fs::read_to_string(&items)
        .map_err(|e| Failure::new(format!("{} を読めません: {e}", items.display())))?;
    let count = validate_ajimee_items(&text).map_err(Failure::new)?;
    println!("ok: {count} items");
    let license = format!(
        "evaluation_items.json: AJIMEE-Bench (https://github.com/azooKey/AJIMEE-Bench), commit {AJIMEE_COMMIT}.\n\
         The data is based on the Japanese Wikipedia Typo Dataset v2 and is licensed under CC BY-SA 3.0.\n\
         Do not commit this directory to the ikari-ime repository.\n"
    );
    std::fs::write(dir.join(AJIMEE_LICENSE_FILE), license)
        .map_err(|e| Failure::new(format!("{AJIMEE_LICENSE_FILE} を書けません: {e}")))?;
    println!("saved: {}", items.display());
    Ok(())
}

/// 形だけ確かめる（JSON の配列で、空でなく、各要素に必要なキーがあること）。件数を返す。
fn validate_ajimee_items(text: &str) -> Result<usize, String> {
    let value: serde_json::Value = serde_json::from_str(text)
        .map_err(|e| format!("evaluation_items.json が JSON ではありません: {e}"))?;
    let items = value
        .as_array()
        .filter(|items| !items.is_empty())
        .ok_or("evaluation_items.json is not a non-empty array")?;
    for key in AJIMEE_REQUIRED_KEYS {
        if !items.iter().all(|item| item.get(key).is_some()) {
            return Err(format!("missing key: {key}"));
        }
    }
    Ok(items.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_sha256_constants_are_64_hex() {
        for sha in [
            SHA_SMALL_LEX_ZIP,
            SHA_CORE_LEX_ZIP,
            SHA_NOTCORE_LEX_ZIP,
            SHA_MATRIX_ZIP,
            SHA_SMALL_LEX_CSV,
            SHA_CORE_LEX_CSV,
            SHA_NOTCORE_LEX_CSV,
            SHA_MATRIX_DEF,
            SHA_CHIVE_TAR,
            SHA_CHIVE_TXT,
            SHA_ZENZ_SMALL,
            SHA_EMOJI_JSON,
            SHA_ABR_ZIP,
            SHA_ABR_CSV,
            SHA_POSTAL_ZIP,
            SHA_POSTAL_CSV,
            SHA_AJIMEE_ITEMS,
        ] {
            assert_eq!(sha.len(), 64, "{sha}");
            assert!(sha.chars().all(|c| c.is_ascii_hexdigit()), "{sha}");
        }
    }

    #[test]
    fn ajimee_validation() {
        assert_eq!(
            validate_ajimee_items(r#"[{"input":"a","expected_output":["b"]}]"#),
            Ok(1)
        );
        assert!(validate_ajimee_items("[]").is_err());
        assert!(validate_ajimee_items("{}").is_err());
        assert!(validate_ajimee_items(r#"[{"input":"a"}]"#).is_err());
        assert!(validate_ajimee_items("not json").is_err());
    }

    #[test]
    fn has_valid_checks_content() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, "abc").unwrap();
        // sha256("abc")
        let abc = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert!(has_valid(&file, abc));
        assert!(!has_valid(&file, SHA_ZENZ_SMALL));
        assert!(!has_valid(&dir.path().join("none"), abc));
    }

    #[test]
    fn check_mode_reports_missing_without_network() {
        let dir = tempfile::tempdir().unwrap();
        let consts = LibConsts::load(&repo_root()).unwrap();
        let mut fetcher = Fetcher {
            raw: dir.path().join("raw"),
            consts: &consts,
            only: vec!["zenz".to_owned(), "emoji".to_owned()],
            check_only: true,
            ajimee_dir: None,
            missing: false,
        };
        fetcher.run_groups().unwrap();
        assert!(fetcher.missing);
        assert!(!dir.path().join("raw").exists());
    }

    #[test]
    fn tmp_path_is_next_to_dest() {
        let tmp = tmp_path(Path::new("/x/y/z.gguf"));
        assert_eq!(tmp.parent(), Some(Path::new("/x/y")));
        assert!(
            tmp.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("z.gguf.tmp.")
        );
    }
}
