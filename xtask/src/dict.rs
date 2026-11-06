//! `cargo xtask dict`（ADR 0043 §3・段 X2。レーン L-D）。辞書のビルド。`scripts/build-dict.sh` の移し先。
//!
//! 手順: dict.new に作る -> vectors.bin を作る -> verify -> dict を dict.prev に、dict.new を dict に入れ替える（旧版を 1 世代残す）。
//! BUILD_INFO に入力の大きさと sha256 などを書き、入力が前回と同じならビルドを飛ばす（ビルドは決定的）。
//! 絵文字の表（emoji.tsv）と住所・郵便番号の表（postal.tsv）は、辞書の入力・BUILD_INFO には含めず、辞書が最新でも毎回作る。
//! 定数（ファイル名、バージョン）は `scripts/lib.sh` から読む（二重管理しない）。

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::XtaskError;
use crate::data::consts::LibConsts;
use crate::data::run::{Failure, cargo, git, log, repo_root, run as exec};
use crate::data::sha256::sha256_file;

const BUILD_INFO_FORMAT: &str = "format=1";
const OUTPUT_PREFIX: &str = "output.";
const BUILDER_PACKAGE: &str = "ikari-dict-builder";
const UNSET_COST: &str = "default";
/// 外来語の固有名詞の区分（`--webwords-proper-offset` の左辺。dict-builder の `webwords lex` と同じ）
const WEBWORDS_PROPER_CLASSES: [&str; 3] = ["proper.person", "proper.place", "proper.general"];
const WEBWORDS_REPORT_FILE: &str = "webwords-report.md";

/// 辞書（grammar.bin、system.dict、vectors.bin）を data/raw/ から data/build/dict/ に作る（SudachiDict Full: small + core + notcore。ADR 0012 §11.3）
#[derive(Debug, clap::Args)]
pub struct Args {
    /// data のディレクトリ（既定はリポジトリの data/）
    #[arg(long)]
    data: Option<PathBuf>,
    /// 入力が前回と同じでも作り直す
    #[arg(long)]
    rebuild: bool,
    /// ABR の地名の行のコスト（掃引用。省略すると dict-builder の既定。ADR 0030 §4）
    #[arg(long)]
    town_cost: Option<String>,
    /// 住所の連なり（都道府県 + 市区町村 + 町名）の行のコスト（掃引用。ADR 0030 §12）
    #[arg(long)]
    chain_cost: Option<String>,
    /// 「都道府県 + 市区町村」だけの連なりの行のコスト（掃引用。ADR 0030 §13）
    #[arg(long)]
    chain_municipality_cost: Option<String>,
    /// 住所の連なりを足さない（掃引・比較用）
    #[arg(long)]
    no_chain: bool,
    /// ポリシーのファイル（掃引用。省略すると dict-builder/policy/chive.tsv）
    #[arg(long)]
    policy: Option<PathBuf>,
    /// 外来語の一覧（ADR 0055）を使わない（掃引・比較用。一覧がないときと同じ辞書になる）
    #[arg(
        long,
        conflicts_with_all = ["webwords_min_df", "webwords_cost", "webwords_proper_offset"]
    )]
    no_webwords: bool,
    /// 外来語の df の下限（掃引用。ADR 0055 §5.3。省略すると配る一覧の下限）
    #[arg(long, value_name = "N")]
    webwords_min_df: Option<String>,
    /// 外来語（普通名詞）のコスト（掃引用。省略すると dict-builder の既定）
    #[arg(long, value_name = "COST", allow_hyphen_values = true)]
    webwords_cost: Option<String>,
    /// 外来語の固有名詞の出発点からの差（掃引用。`<区分>=<値>`。区分は proper.person / proper.place / proper.general。複数指定できる）
    #[arg(long, value_name = "CLASS=VALUE", allow_hyphen_values = true)]
    webwords_proper_offset: Vec<String>,
}

impl Args {
    /// 外来語の掃引用の引数の形を確かめる（dict-builder に渡す前に、ビルドの途中ではなく最初に止める）
    fn check_webwords_args(&self) -> Result<(), Failure> {
        if let Some(value) = &self.webwords_min_df {
            value.parse::<u32>().map_err(|e| {
                Failure::new(format!(
                    "--webwords-min-df が整数ではありません: {value} ({e})"
                ))
            })?;
        }
        if let Some(value) = &self.webwords_cost {
            value.parse::<i16>().map_err(|e| {
                Failure::new(format!(
                    "--webwords-cost が整数ではありません: {value} ({e})"
                ))
            })?;
        }
        for offset in &self.webwords_proper_offset {
            parse_proper_offset(offset)?;
        }
        Ok(())
    }

    /// 外来語の掃引用の引数が 1 つでも指定されたか
    fn webwords_overrides_given(&self) -> bool {
        self.webwords_min_df.is_some()
            || self.webwords_cost.is_some()
            || !self.webwords_proper_offset.is_empty()
    }
}

/// `proper.person=2000` を (区分, 値) にする。区分は固有名詞の 3 つだけ（普通名詞の差は `--webwords-cost`）
fn parse_proper_offset(text: &str) -> Result<(&'static str, i16), Failure> {
    let (class, value) = text.split_once('=').ok_or_else(|| {
        Failure::new(format!(
            "--webwords-proper-offset は `区分=値` の形です: {text}"
        ))
    })?;
    let class = WEBWORDS_PROPER_CLASSES
        .iter()
        .find(|known| **known == class)
        .ok_or_else(|| {
            Failure::new(format!(
                "--webwords-proper-offset の区分は {} のどれかです: {class}",
                WEBWORDS_PROPER_CLASSES.join(" / ")
            ))
        })?;
    let value = value.parse::<i16>().map_err(|e| {
        Failure::new(format!(
            "--webwords-proper-offset の値が整数ではありません: {value} ({e})"
        ))
    })?;
    Ok((class, value))
}

/// BUILD_INFO に書く外来語の値（一覧を組み込むときだけ書く。一覧がなければ 1 行も増えない）
fn webwords_info_lines(args: &Args) -> Vec<String> {
    let value = |v: &Option<String>| v.clone().unwrap_or_else(|| UNSET_COST.to_owned());
    let mut lines = vec![
        format!("webwords.min-df={}", value(&args.webwords_min_df)),
        format!("webwords.cost={}", value(&args.webwords_cost)),
    ];
    for offset in &args.webwords_proper_offset {
        if let Ok((class, value)) = parse_proper_offset(offset) {
            lines.push(format!("webwords.proper-offset.{class}={value}"));
        }
    }
    lines
}

/// `ikari-dict-builder webwords lex` の引数（ADR 0055 §5.3 の 2）。`lex` は Sudachi の 3 つと ABR の lex.csv
fn webwords_lex_args(
    list: &Path,
    lex: &[PathBuf],
    out: &Path,
    report: &Path,
    args: &Args,
) -> Vec<OsString> {
    let mut v: Vec<OsString> = vec![
        "webwords".into(),
        "lex".into(),
        "--list".into(),
        list.into(),
    ];
    for path in lex {
        v.push("--lex".into());
        v.push(path.into());
    }
    v.push("--out".into());
    v.push(out.into());
    v.push("--report".into());
    v.push(report.into());
    if let Some(min_df) = &args.webwords_min_df {
        v.extend(["--min-df".into(), min_df.into()]);
    }
    if let Some(cost) = &args.webwords_cost {
        v.extend(["--cost".into(), cost.into()]);
    }
    for offset in &args.webwords_proper_offset {
        v.extend(["--proper-offset".into(), offset.into()]);
    }
    v
}

pub fn run_command(args: Args) -> Result<(), Failure> {
    args.check_webwords_args()?;
    let root = repo_root();
    let consts = LibConsts::load(&root).map_err(|e| Failure::new(e.to_string()))?;
    Builder::new(&root, &consts, args)?.run()
}

pub fn run(args: Args) -> Result<(), XtaskError> {
    run_command(args).map_err(|failure| XtaskError(failure.message))
}

/// 辞書ビルドの文脈（パスと定数）。
struct Builder<'a> {
    root: &'a Path,
    consts: &'a LibConsts,
    args: Args,
    raw: PathBuf,
    build: PathBuf,
    sudachi: PathBuf,
    chive: PathBuf,
    policy: PathBuf,
    builder: PathBuf,
    /// 入力（sha256 を取る順。BUILD_INFO の並びでもある）
    inputs: Vec<PathBuf>,
    abr_csv: PathBuf,
    use_abr: bool,
    /// 組み込む外来語の一覧（なければ、または `--no-webwords` なら None。今と同じ辞書を作る）
    webwords_list: Option<PathBuf>,
}

/// 失敗したら途中の dict.new を消す。
struct RemoveOnDrop(Option<PathBuf>);

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = std::fs::remove_dir_all(path);
        }
    }
}

impl<'a> Builder<'a> {
    fn new(root: &'a Path, consts: &'a LibConsts, args: Args) -> Result<Self, Failure> {
        let data = args.data.clone().unwrap_or_else(|| root.join("data"));
        if data.as_os_str().is_empty() {
            return Err(Failure::new("--data が空です"));
        }
        let get = |name: &str| consts.get(name).map_err(|e| Failure::new(e.to_string()));
        let raw = data.join("raw");
        let build = data.join("build");
        let sudachi = raw.join(get("SUDACHI_DIR_NAME")?);
        let chive_name = get("CHIVE_NAME")?;
        let chive = raw
            .join("chive")
            .join(chive_name)
            .join(format!("{chive_name}.txt"));
        let policy = args
            .policy
            .clone()
            .unwrap_or_else(|| root.join(get("DICT_POLICY").unwrap_or_default()));
        let builder = root.join("target").join("release").join(format!(
            "ikari-dict-builder{}",
            std::env::consts::EXE_SUFFIX
        ));
        let mut inputs: Vec<PathBuf> = [
            "small_lex.csv",
            "core_lex.csv",
            "notcore_lex.csv",
            "matrix.def",
        ]
        .iter()
        .map(|name| sudachi.join(name))
        .collect();
        inputs.push(chive.clone());
        let abr_csv = raw.join(get("ABR_TOWN_CSV")?);
        let use_abr = abr_csv.is_file();
        if use_abr {
            inputs.push(abr_csv.clone());
        } else {
            log(&format!(
                "ABR の町字マスターがないので、大字・町名は足しません（{}。scripts/fetch-data.sh --only abr で取得できます）",
                abr_csv.display()
            ));
        }
        let list = raw.join(get("WEBWORDS_LIST_PATH")?);
        let webwords_list = if args.no_webwords {
            log("--no-webwords なので、外来語の一覧は足しません");
            None
        } else if list.is_file() {
            inputs.push(list.clone());
            Some(list)
        } else {
            if args.webwords_overrides_given() {
                return Err(Failure::new(format!(
                    "--webwords-* を指定しましたが、外来語の一覧 {} がありません（scripts/fetch-data.sh --only webwords）",
                    list.display()
                )));
            }
            log(&format!(
                "外来語の一覧がないので、外来語は足しません（{}。scripts/fetch-data.sh --only webwords で取得できます）",
                list.display()
            ));
            None
        };
        for input in &inputs {
            if !input.is_file() {
                return Err(Failure::new(format!(
                    "{} がありません。scripts/fetch-data.sh で取得してください（--data を使うときは同じ値を渡す）",
                    input.display()
                )));
            }
        }
        if !policy.is_file() {
            return Err(Failure::new(format!("{} がありません", policy.display())));
        }
        Ok(Self {
            root,
            consts,
            args,
            raw,
            build,
            sudachi,
            chive,
            policy,
            builder,
            inputs,
            abr_csv,
            use_abr,
            webwords_list,
        })
    }

    fn get(&self, name: &str) -> Result<&str, Failure> {
        self.consts
            .get(name)
            .map_err(|e| Failure::new(e.to_string()))
    }

    fn file_name(path: &Path) -> String {
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    /// ツリーの識別子（コミット済みの内容）。作業ツリーに未コミットの変更があれば +dirty を付ける。
    fn tree_id(&self, dir: &str) -> String {
        let spec = format!("HEAD:{dir}");
        let Some(out) = crate::data::run::capture(git(self.root).args(["rev-parse", &spec])) else {
            return "unknown".to_owned();
        };
        let mut id = out.trim().to_owned();
        let dirty =
            crate::data::run::capture(git(self.root).args(["status", "--porcelain", "--", dir]))
                .is_some_and(|s| !s.trim().is_empty());
        if dirty {
            id.push_str("+dirty");
        }
        id
    }

    /// 入力の部分（BUILD_INFO の output. 以外の行）。
    fn input_info(&self) -> Result<String, Failure> {
        let cost = |value: &Option<String>| value.clone().unwrap_or_else(|| UNSET_COST.to_owned());
        let mut lines = vec![BUILD_INFO_FORMAT.to_owned()];
        for input in &self.inputs {
            let size = std::fs::metadata(input)
                .map_err(|e| Failure::new(format!("{}: {e}", input.display())))?
                .len();
            let sha = sha256_file(input)
                .map_err(|e| Failure::new(format!("{}: {e}", input.display())))?;
            lines.push(format!("input.{}={size} {sha}", Self::file_name(input)));
        }
        if self.use_abr {
            lines.push(format!("town.cost={}", cost(&self.args.town_cost)));
            if self.args.no_chain {
                lines.push("town.chain=off".to_owned());
            } else {
                lines.push(format!("town.chain-cost={}", cost(&self.args.chain_cost)));
                lines.push(format!(
                    "town.chain-municipality-cost={}",
                    cost(&self.args.chain_municipality_cost)
                ));
            }
        }
        if self.webwords_list.is_some() {
            lines.extend(webwords_info_lines(&self.args));
        }
        let policy_sha = sha256_file(&self.policy)
            .map_err(|e| Failure::new(format!("{}: {e}", self.policy.display())))?;
        lines.push(format!(
            "policy.{}={policy_sha}",
            Self::file_name(&self.policy)
        ));
        lines.push(format!(
            "builder.dict-builder={}",
            self.tree_id("dict-builder")
        ));
        lines.push(format!(
            "builder.dict-format={}",
            self.tree_id("dict-format")
        ));
        Ok(lines.join("\n"))
    }

    fn ensure_builder(&self) -> Result<(), Failure> {
        if self.builder.is_file() {
            return Ok(());
        }
        self.build_builder()
    }

    fn build_builder(&self) -> Result<(), Failure> {
        exec(cargo(self.root).args(["build", "--release", "-p", BUILDER_PACKAGE]))?;
        if self.builder.is_file() {
            Ok(())
        } else {
            Err(Failure::new(format!(
                "{} がありません",
                self.builder.display()
            )))
        }
    }

    fn builder_cmd(&self) -> Command {
        Command::new(&self.builder)
    }

    fn lex_args(&self) -> Vec<PathBuf> {
        ["small_lex.csv", "core_lex.csv", "notcore_lex.csv"]
            .iter()
            .map(|name| self.sudachi.join(name))
            .collect()
    }

    /// 絵文字の表を作る（なければ飛ばす）。
    fn build_emoji(&self) -> Result<(), Failure> {
        let table = self.get("EMOJI_TABLE_FILE")?;
        let json = self.raw.join(self.get("EMOJI_DATA_JSON")?);
        if !json.is_file() {
            log(&format!(
                "{table} は作りません（{} がありません。scripts/fetch-data.sh --only emoji で取得できます）",
                json.display()
            ));
            return Ok(());
        }
        self.ensure_builder()?;
        log(&format!("{table} を作る"));
        let dict = self.build.join("dict");
        mk(&dict)?;
        exec(
            self.builder_cmd()
                .arg("emoji")
                .arg("--json")
                .arg(&json)
                .args(["--commit", self.get("EMOJI_DATA_COMMIT")?])
                .arg("--out")
                .arg(dict.join(table)),
        )
    }

    /// 住所と郵便番号の表を作る（変換表と町字マスターがなければ飛ばす）。
    /// town は辞書のためのファイルも同時に書くので、出力先は postal 専用の一時ディレクトリにして捨てる。
    fn build_postal(&self) -> Result<(), Failure> {
        let table = self.get("POSTAL_TABLE_FILE")?;
        let csv = self.raw.join(self.get("ABR_POSTAL_CSV")?);
        if !csv.is_file() {
            log(&format!(
                "{table} は作りません（{} がありません。scripts/fetch-data.sh --only postal で取得できます）",
                csv.display()
            ));
            return Ok(());
        }
        if !self.use_abr {
            log(&format!(
                "{table} は作りません（町字マスター {} がありません。scripts/fetch-data.sh --only abr で取得できます）",
                self.abr_csv.display()
            ));
            return Ok(());
        }
        self.ensure_builder()?;
        log(&format!("{table} を作る"));
        let dict = self.build.join("dict");
        mk(&dict)?;
        let tmp = self.build.join(format!("postal.tmp{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        mk(&tmp)?;
        let _cleanup = RemoveOnDrop(Some(tmp.clone()));
        let mut command = self.builder_cmd();
        command.arg("town").arg("--csv").arg(&self.abr_csv);
        for lex in self.lex_args() {
            command.arg("--lex").arg(lex);
        }
        command
            .arg("--out")
            .arg(tmp.join("town_lex.csv"))
            .arg("--postal-csv")
            .arg(&csv)
            .arg("--postal-out")
            .arg(tmp.join(table));
        exec(&mut command).map_err(|_| Failure::new(format!("{table} を作れませんでした")))?;
        std::fs::rename(tmp.join(table), dict.join(table))
            .map_err(|e| Failure::new(format!("{table} を置けません: {e}")))
    }

    fn run(&self) -> Result<(), Failure> {
        let dict_files = self
            .consts
            .array("DICT_FILES")
            .map_err(|e| Failure::new(e.to_string()))?;
        let dict = self.build.join("dict");
        let info_file = dict.join("BUILD_INFO");
        log("入力を確かめる（sha256）");
        let new_input_info = self.input_info()?;

        if !self.args.rebuild && info_file.is_file() {
            let complete = dict_files.iter().all(|f| dict.join(f).is_file());
            let current = std::fs::read_to_string(&info_file).unwrap_or_default();
            let current_inputs = current
                .lines()
                .filter(|line| !line.starts_with(OUTPUT_PREFIX))
                .collect::<Vec<_>>()
                .join("\n");
            if complete && current_inputs.trim_end_matches('\n') == new_input_info {
                log(&format!(
                    "辞書は最新です（入力が BUILD_INFO と同じ）。--rebuild で作り直せます: {}",
                    dict.display()
                ));
                self.build_emoji()?;
                self.build_postal()?;
                return Ok(());
            }
        }

        log("ikari-dict-builder をビルド");
        self.build_builder()?;

        let new = self.build.join("dict.new");
        let _ = std::fs::remove_dir_all(&new);
        mk(&self.build)?;
        // 失敗したら途中のものを残さない
        let mut guard = RemoveOnDrop(Some(new.clone()));

        let mut lex: Vec<PathBuf> = self.lex_args();
        let mut notcore: Vec<(&str, PathBuf)> =
            vec![("--notcore-lex", self.sudachi.join("notcore_lex.csv"))];
        if self.use_abr {
            let town_lex = self.build.join(self.get("ABR_TOWN_LEX_FILE")?);
            let chain_lex = self.build.join(self.get("ABR_CHAIN_LEX_FILE")?);
            log(&format!(
                "ABR の町字マスターから地名の lex.csv を作る -> {}",
                town_lex.display()
            ));
            let mut command = self.builder_cmd();
            command.arg("town").arg("--csv").arg(&self.abr_csv);
            for path in &lex {
                command.arg("--lex").arg(path);
            }
            if let Some(cost) = &self.args.town_cost {
                command.args(["--cost", cost]);
            }
            if !self.args.no_chain {
                command.arg("--chain-out").arg(&chain_lex);
                if let Some(cost) = &self.args.chain_cost {
                    command.args(["--chain-cost", cost]);
                }
                if let Some(cost) = &self.args.chain_municipality_cost {
                    command.args(["--chain-municipality-cost", cost]);
                }
            }
            exec(command.arg("--out").arg(&town_lex))?;
            lex.push(town_lex.clone());
            notcore.push(("--notcore-lex", town_lex));
            // 住所の連なり: 同じ行の Core（Sudachi の郵便番号の住所）より優先する（ADR 0030 §12）
            if !self.args.no_chain {
                lex.push(chain_lex.clone());
                notcore.push(("--chain-lex", chain_lex));
            }
        }

        if let Some(list) = &self.webwords_list {
            // 一覧を F1 などでもう一度当てるので、Sudachi と ABR の lex.csv を渡す（ABR の町名と同じ位置に足す）
            let webwords_lex = self.build.join(self.get("WEBWORDS_LEX_FILE")?);
            log(&format!(
                "外来語の一覧から lex.csv を作る -> {}",
                webwords_lex.display()
            ));
            let mut command = self.builder_cmd();
            command.args(webwords_lex_args(
                list,
                &lex,
                &webwords_lex,
                &self.build.join(WEBWORDS_REPORT_FILE),
                &self.args,
            ));
            exec(&mut command)?;
            lex.push(webwords_lex.clone());
            notcore.push(("--notcore-lex", webwords_lex));
        }

        log(&format!("辞書を作る -> {}", new.display()));
        let mut command = self.builder_cmd();
        command
            .arg("--policy")
            .arg(&self.policy)
            .arg("--chive-vocab")
            .arg(&self.chive);
        for path in [
            self.sudachi.join("small_lex.csv"),
            self.sudachi.join("core_lex.csv"),
        ] {
            command.arg("--lex").arg(path);
        }
        for (flag, path) in &notcore {
            command.arg(flag).arg(path);
        }
        command
            .arg("--matrix")
            .arg(self.sudachi.join("matrix.def"))
            .args([
                "--label",
                &format!("sudachi-full-{}", self.get("SUDACHI_VERSION")?),
            ])
            .arg("--out")
            .arg(&new)
            .arg("--policy-report")
            .arg(self.build.join("dict-policy-report.md"));
        exec(&mut command)?;

        log("vectors.bin を作る");
        let mut command = self.builder_cmd();
        command
            .arg("vectors")
            .arg("--chive")
            .arg(&self.chive)
            .arg("--dict")
            .arg(&new)
            .arg("--out")
            .arg(new.join("vectors.bin"));
        for path in &lex {
            command.arg("--lex").arg(path);
        }
        exec(&mut command)?;

        log("検証");
        exec(self.builder_cmd().arg("--verify").arg(&new))?;

        let mut info = format!("{new_input_info}\n");
        for file in dict_files {
            let path = new.join(file);
            if !path.is_file() {
                return Err(Failure::new(format!("{} ができていません", path.display())));
            }
            let sha =
                sha256_file(&path).map_err(|e| Failure::new(format!("{}: {e}", path.display())))?;
            info.push_str(&format!("{OUTPUT_PREFIX}{file}={sha}\n"));
        }
        std::fs::write(new.join("BUILD_INFO"), info)
            .map_err(|e| Failure::new(format!("BUILD_INFO を書けません: {e}")))?;

        log("入れ替え: dict -> dict.prev、dict.new -> dict");
        let prev = self.build.join("dict.prev");
        let _ = std::fs::remove_dir_all(&prev);
        if dict.is_dir() {
            std::fs::rename(&dict, &prev)
                .map_err(|e| Failure::new(format!("dict を dict.prev にできません: {e}")))?;
        }
        std::fs::rename(&new, &dict)
            .map_err(|e| Failure::new(format!("dict.new を dict にできません: {e}")))?;
        guard.0 = None;
        self.build_emoji()?;
        self.build_postal()?;

        log(&format!("完了: {}", dict.display()));
        let text = std::fs::read_to_string(&info_file).unwrap_or_default();
        for line in text.lines().filter(|l| l.starts_with(OUTPUT_PREFIX)) {
            println!("{line}");
        }
        Ok(())
    }
}

fn mk(path: &Path) -> Result<(), Failure> {
    std::fs::create_dir_all(path)
        .map_err(|e| Failure::new(format!("{} を作れません: {e}", path.display())))
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Debug, Parser)]
    struct Cli {
        #[command(flatten)]
        args: Args,
    }

    fn parse(extra: &[&str]) -> Args {
        let mut argv = vec!["x"];
        argv.extend_from_slice(extra);
        Cli::try_parse_from(argv).unwrap().args
    }

    fn strings(args: Vec<OsString>) -> Vec<String> {
        args.into_iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    /// 入力がそろった data のディレクトリ（中身はダミー。dict-builder は呼ばない）
    fn fake_data(with_list: bool) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let consts = LibConsts::load(&repo_root()).unwrap();
        let raw = dir.path().join("raw");
        let sudachi = raw.join(consts.get("SUDACHI_DIR_NAME").unwrap());
        let chive_name = consts.get("CHIVE_NAME").unwrap();
        let chive = raw.join("chive").join(chive_name);
        std::fs::create_dir_all(&sudachi).unwrap();
        std::fs::create_dir_all(&chive).unwrap();
        for name in [
            "small_lex.csv",
            "core_lex.csv",
            "notcore_lex.csv",
            "matrix.def",
        ] {
            std::fs::write(sudachi.join(name), name).unwrap();
        }
        std::fs::write(chive.join(format!("{chive_name}.txt")), "chive").unwrap();
        if with_list {
            let list = raw.join(consts.get("WEBWORDS_LIST_PATH").unwrap());
            std::fs::create_dir_all(list.parent().unwrap()).unwrap();
            std::fs::write(list, "#ikari-webwords\tschema=2\n").unwrap();
        }
        dir
    }

    fn input_info(data: &Path, extra: &[&str]) -> Result<String, Failure> {
        let root = repo_root();
        let consts = LibConsts::load(&root).unwrap();
        let data_arg = data.to_string_lossy().into_owned();
        let mut argv = vec!["--data", data_arg.as_str()];
        argv.extend_from_slice(extra);
        Builder::new(&root, &consts, parse(&argv))?.input_info()
    }

    #[test]
    fn without_list_build_info_has_no_webwords_lines() {
        let data = fake_data(false);
        let info = input_info(data.path(), &[]).unwrap();
        assert!(!info.contains("webwords"), "{info}");
    }

    #[test]
    fn no_webwords_gives_the_same_build_info_as_no_list() {
        let without = input_info(fake_data(false).path(), &[]).unwrap();
        let data = fake_data(true);
        let disabled = input_info(data.path(), &["--no-webwords"]).unwrap();
        assert_eq!(without, disabled);
        let enabled = input_info(data.path(), &[]).unwrap();
        assert_ne!(without, enabled);
    }

    #[test]
    fn with_list_build_info_has_sha256_and_values() {
        let data = fake_data(true);
        let info = input_info(
            data.path(),
            &[
                "--webwords-min-df",
                "10",
                "--webwords-cost",
                "13000",
                "--webwords-proper-offset",
                "proper.person=-500",
            ],
        )
        .unwrap();
        assert!(info.contains("input.webwords.tsv="), "{info}");
        assert!(info.contains("webwords.min-df=10"), "{info}");
        assert!(info.contains("webwords.cost=13000"), "{info}");
        assert!(
            info.contains("webwords.proper-offset.proper.person=-500"),
            "{info}"
        );
        let defaults = input_info(data.path(), &[]).unwrap();
        assert!(defaults.contains("webwords.min-df=default"), "{defaults}");
        assert!(defaults.contains("webwords.cost=default"), "{defaults}");
    }

    #[test]
    fn overrides_without_list_are_an_error() {
        let data = fake_data(false);
        let err = input_info(data.path(), &["--webwords-cost", "13000"]).unwrap_err();
        assert!(err.message.contains("外来語の一覧"), "{err}");
    }

    #[test]
    fn no_webwords_conflicts_with_overrides() {
        let result = Cli::try_parse_from(["x", "--no-webwords", "--webwords-cost", "1"]);
        assert!(result.is_err());
    }

    #[test]
    fn lex_args_follow_the_dict_builder_contract() {
        let args = parse(&[]);
        let got = strings(webwords_lex_args(
            Path::new("l.tsv"),
            &[PathBuf::from("a.csv"), PathBuf::from("town.csv")],
            Path::new("o.csv"),
            Path::new("r.md"),
            &args,
        ));
        assert_eq!(
            got,
            [
                "webwords", "lex", "--list", "l.tsv", "--lex", "a.csv", "--lex", "town.csv",
                "--out", "o.csv", "--report", "r.md"
            ]
        );
    }

    #[test]
    fn lex_args_pass_sweep_overrides() {
        let args = parse(&[
            "--webwords-min-df",
            "10",
            "--webwords-cost",
            "-100",
            "--webwords-proper-offset",
            "proper.place=2000",
            "--webwords-proper-offset",
            "proper.general=-2000",
        ]);
        args.check_webwords_args().unwrap();
        let got = strings(webwords_lex_args(
            Path::new("l"),
            &[],
            Path::new("o"),
            Path::new("r"),
            &args,
        ));
        assert_eq!(
            got[8..],
            [
                "--min-df",
                "10",
                "--cost",
                "-100",
                "--proper-offset",
                "proper.place=2000",
                "--proper-offset",
                "proper.general=-2000"
            ]
        );
    }

    #[test]
    fn invalid_sweep_values_are_rejected() {
        for bad in [
            ["--webwords-min-df", "x"],
            ["--webwords-cost", "1.5"],
            ["--webwords-proper-offset", "common=1"],
            ["--webwords-proper-offset", "proper.place"],
            ["--webwords-proper-offset", "proper.place=x"],
        ] {
            assert!(parse(&bad).check_webwords_args().is_err(), "{bad:?}");
        }
    }
}
