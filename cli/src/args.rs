//! コマンドライン引数（ADR 0001 §5, ADR 0002 §14）。解釈の部分は純粋関数にしてテストする。
//!
//! `ikari <READING>`（フェーズ0の形）と `ikari convert <READING>` は同じ。サブコマンド名と同じ綴りの
//! 読み（`data` など）を変換したいときは `ikari convert data` と書く。

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand, ValueEnum};
use ikari_input::JisKey;

pub const DICT_DIR_ENV: &str = "IKARI_DICT_DIR";
pub const DEFAULT_DICT_DIR: &str = "./data/build/dict";
const DEFAULT_NUM: usize = 9;
const MAX_PAGE_SIZE: usize = 9;
const JIS_SHIFT_PREFIX: char = '+';
/// `--profile` でプロファイルを使わないことを表す値
pub const PROFILE_NONE: &str = "none";
const DEFAULT_LEARN_SHOW_LIMIT: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum InputMode {
    /// READING がすべて ASCII なら romaji、そうでなければ kana
    Auto,
    Romaji,
    Kana,
    /// JIS の無シフト刻印の列。'+' を前に付けると次のキーをシフト
    Jis,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    Text,
    Tsv,
}

/// 使う Ranker の構成
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum RankerSet {
    /// 単語コストと連接コストだけ（フェーズ0と同じ）
    Cost,
    /// 解析器と、使えるすべての Ranker（文書内キャッシュ、学習、プロファイル、話題）
    #[default]
    Full,
}

impl From<RankerSet> for ikari_stack::RankerSet {
    fn from(set: RankerSet) -> Self {
        match set {
            RankerSet::Cost => ikari_stack::RankerSet::Cost,
            RankerSet::Full => ikari_stack::RankerSet::Full,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum UserDictFormat {
    Ikari,
    Mozc,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum LayerArg {
    Usage,
    Initial,
    All,
}

/// `convert` と `session` が共有する、辞書と順位付けの指定。
#[derive(clap::Args, Debug, Clone, Default)]
pub struct CommonArgs {
    /// 辞書ディレクトリ（grammar.bin と system.dict）。省略時は $IKARI_DICT_DIR、なければ <ユーザーデータ>/dict、それもなければ ./data/build/dict
    #[arg(long, value_name = "DIR")]
    pub dict: Option<PathBuf>,

    /// ユーザーデータの置き場所（$IKARI_USER_DIR。既定は macOS が ~/Library/Application Support/ikari-ime、
    /// 他は $XDG_DATA_HOME/ikari-ime）
    #[arg(long, value_name = "DIR")]
    pub user_dir: Option<PathBuf>,

    /// 順位付けの構成
    #[arg(long, value_enum, value_name = "SET", default_value = "full")]
    pub ranker: RankerSet,

    /// 文脈の解析器を使わない（BOS を固定する。比較用）
    #[arg(long)]
    pub no_analyzer: bool,

    /// 同梱の語彙集（IT）を使わない（比較用。既定は使う）
    #[arg(long)]
    pub no_packs: bool,

    /// 「て」+ 郵便番号で〒つきの候補を出す（設定 postal.te_prefix を on にした状態。ADR 0032 §8）
    #[arg(long)]
    pub postal_te_prefix: bool,

    /// 1 文字のかなに U+3042 のような Unicode 表記の候補を出す（設定 unicode.notation を on にした状態）
    #[arg(long)]
    pub unicode_notation: bool,

    /// 曜日の読みに、直近の日付の候補を出す（設定 date.weekday_dates を on にした状態）
    #[arg(long)]
    pub date_weekday_dates: bool,

    /// vectors.bin（既定は <dict>/vectors.bin があれば使う）
    #[arg(long, value_name = "PATH")]
    pub vectors: Option<PathBuf>,

    /// profile.tsv の代わりに一時的に使うプロファイル（プリセットの id か none）
    #[arg(long, value_name = "NAME|none")]
    pub profile: Option<String>,

    /// Ranker の重みの上書き（key=value の行）
    #[arg(long, value_name = "PATH")]
    pub ranker_config: Option<PathBuf>,

    /// zenz の GGUF
    #[cfg(feature = "lm")]
    #[arg(long, value_name = "GGUF")]
    pub lm: Option<PathBuf>,

    /// zenz の待ち時間の上限（ミリ秒）。省略時は ZenzConfig の既定（300ms）。
    /// 実際の待ちは変換1回の締め切り（`ConvertOptions.lm_budget`: 読み1文字 6ms、30〜300ms）で決まる
    #[cfg(feature = "lm")]
    #[arg(long, value_name = "MS")]
    pub lm_budget_ms: Option<u64>,
}

/// `ikari [convert] [OPTIONS] <READING>`
#[derive(clap::Args, Debug, Clone)]
pub struct Args {
    /// 読み（入力モードに応じて、ローマ字、かな、JIS の刻印列）
    pub reading: String,

    #[command(flatten)]
    pub common: CommonArgs,

    /// 左文脈（確定済みのテキスト）
    #[arg(long, value_name = "TEXT", default_value = "")]
    pub context: String,

    /// パスワード欄として扱う（文脈も学習も使わない）
    #[arg(long)]
    pub secure: bool,

    /// 入力モード
    #[arg(long, value_enum, value_name = "MODE", default_value = "auto")]
    pub input: InputMode,

    /// ローマ字表の TSV（省略時は同梱の表。指定すると丸ごと置き換える）
    #[arg(long, value_name = "PATH")]
    pub romaji_table: Option<PathBuf>,

    /// 出力する候補の数。必要なページを順に取得する
    #[arg(short = 'n', long, value_name = "N", default_value_t = DEFAULT_NUM,
          value_parser = parse_num)]
    pub num: usize,

    /// 1ページの候補数（1..=9）
    #[arg(long, value_name = "N", default_value_t = MAX_PAGE_SIZE,
          value_parser = parse_page_size)]
    pub page_size: usize,

    /// 出力形式
    #[arg(long, value_enum, value_name = "FMT", default_value = "text")]
    pub format: OutputFormat,

    /// text 形式で分割と品詞も表示する
    #[arg(long)]
    pub segments: bool,

    /// 変換開始から1ページ目までの時間と、辞書を開く時間を stderr に出す
    #[arg(long)]
    pub timing: bool,

    /// その順位（1 始まり）の候補を確定して学習する（learning.tsv を保存する）。--secure のときは学習しない
    #[arg(long, value_name = "RANK", value_parser = parse_num)]
    pub commit: Option<usize>,

    /// エンジンが動いていても学習ファイルを書き換える（エンジンのメモリ上の学習で上書きされることがある）
    #[arg(long)]
    pub force: bool,
}

#[derive(clap::Args, Debug, Clone)]
pub struct SessionArgs {
    #[command(flatten)]
    pub common: CommonArgs,

    /// 「読み<TAB>選ぶ表記」の行を読んで非対話で実行する（選ぶ表記が空なら確定せず、1位を表示するだけ）
    #[arg(long, value_name = "TSV")]
    pub script: Option<PathBuf>,

    /// 入力モード
    #[arg(long, value_enum, value_name = "MODE", default_value = "auto")]
    pub input: InputMode,

    /// 1ページの候補数（1..=9）
    #[arg(long, value_name = "N", default_value_t = MAX_PAGE_SIZE,
          value_parser = parse_page_size)]
    pub page_size: usize,

    /// パスワード欄として扱う（文脈も学習も使わない）
    #[arg(long)]
    pub secure: bool,

    /// エンジンが動いていても学習ファイルを書き換える（エンジンのメモリ上の学習で上書きされることがある）
    #[arg(long)]
    pub force: bool,
}

#[derive(Subcommand, Debug, Clone)]
pub enum Command {
    /// 読みを変換する（サブコマンドなしの `ikari <READING>` と同じ）
    Convert(Args),
    /// 1行ずつ読みを入力し、番号で確定する。確定履歴を左文脈に積んでいく
    Session(SessionArgs),
    /// ユーザー辞書（Mozc 互換の4列 TSV）
    #[command(name = "user-dict")]
    UserDict(UserDictArgs),
    /// 学習データの確認とリセット
    Learn(LearnArgs),
    /// 属性プロファイル
    Profile(ProfileArgs),
    /// 文章の取り込み（初期学習）
    Ingest(IngestArgs),
    /// ユーザーデータのエクスポートとインポート
    Data(DataArgs),
}

#[derive(clap::Args, Debug, Clone)]
pub struct UserDictArgs {
    /// ユーザーデータの置き場所
    #[arg(long, value_name = "DIR", global = true)]
    pub user_dir: Option<PathBuf>,
    /// 辞書ディレクトリ（`check` で使う）
    #[arg(long, value_name = "DIR", global = true)]
    pub dict: Option<PathBuf>,
    #[command(subcommand)]
    pub command: UserDictCommand,
}

#[derive(Subcommand, Debug, Clone)]
pub enum UserDictCommand {
    /// 語を登録する（同じ読みと表記は上書き）
    Add {
        /// 読み（ひらがな。カタカナは変換する）
        reading: String,
        /// 表記
        surface: String,
        /// 品詞（名詞、サ変名詞、形状詞、固有名詞、組織、人名、姓、名、地名、副詞、感動詞、短縮よみ、形容詞、動詞:<活用型>）
        pos: String,
        #[arg(long, default_value = "")]
        comment: String,
    },
    /// 語を消す
    Remove { reading: String, surface: String },
    /// 登録語を一覧する
    List,
    /// ファイルから取り込む（マージ。同じ読みと表記は上書き）
    Import {
        #[arg(long, value_enum, default_value = "ikari")]
        format: UserDictFormat,
        file: PathBuf,
    },
    /// 品詞の対応表と登録語を、辞書の文法に対して検査する
    Check,
}

#[derive(clap::Args, Debug, Clone)]
pub struct LearnArgs {
    /// ユーザーデータの置き場所
    #[arg(long, value_name = "DIR", global = true)]
    pub user_dir: Option<PathBuf>,
    /// エンジンが動いていても学習ファイルを書き換える（エンジンのメモリ上の学習で上書きされることがある）
    #[arg(long, global = true)]
    pub force: bool,
    #[command(subcommand)]
    pub command: LearnCommand,
}

#[derive(Subcommand, Debug, Clone)]
pub enum LearnCommand {
    /// 学習の記録を表示する（特徴量はハッシュのまま）
    Show {
        /// この読みの記録だけ
        #[arg(long)]
        reading: Option<String>,
        /// 表示する最大行数
        #[arg(long, default_value_t = DEFAULT_LEARN_SHOW_LIMIT)]
        limit: usize,
    },
    /// 層ごとに学習を消す（initial は取り込みの層、usage は実際の使用の層）
    Reset {
        #[arg(long, value_enum)]
        layer: LayerArg,
    },
}

#[derive(clap::Args, Debug, Clone)]
pub struct ProfileArgs {
    /// ユーザーデータの置き場所
    #[arg(long, value_name = "DIR", global = true)]
    pub user_dir: Option<PathBuf>,
    #[command(subcommand)]
    pub command: ProfileCommand,
}

#[derive(Subcommand, Debug, Clone)]
pub enum ProfileCommand {
    /// 同梱のプリセットを一覧する
    Presets,
    /// 現在の設定を表示する
    Show,
    /// 分野を設定する（例: `it=often infra=sometimes`）。既存の設定は置き換える
    Set {
        #[arg(required = true, value_name = "DOMAIN=often|sometimes")]
        settings: Vec<String>,
    },
    /// 設定を消す
    Clear,
}

#[derive(clap::Args, Debug, Clone)]
pub struct IngestArgs {
    /// ユーザーデータの置き場所
    #[arg(long, value_name = "DIR", global = true)]
    pub user_dir: Option<PathBuf>,
    /// エンジンが動いていても学習ファイルを書き換える（エンジンのメモリ上の学習で上書きされることがある）
    #[arg(long, global = true)]
    pub force: bool,
    /// 辞書ディレクトリ
    #[arg(long, value_name = "DIR", global = true)]
    pub dict: Option<PathBuf>,
    #[command(subcommand)]
    pub command: IngestCommand,
}

#[derive(Subcommand, Debug, Clone)]
pub enum IngestCommand {
    /// 同梱のプロンプトを表示する
    Prompt,
    /// LLM が出力したファイル（JSONL）を取り込む
    Run {
        file: PathBuf,
        /// 検証と集計だけして、何も保存しない
        #[arg(long)]
        dry_run: bool,
        /// 推定した分野を profile.tsv に書く
        #[arg(long)]
        apply_profile: bool,
        /// vectors.bin（分野の推定に使う。既定は <dict>/vectors.bin があれば使う）
        #[arg(long, value_name = "PATH")]
        vectors: Option<PathBuf>,
    },
    /// 取り込み元の一覧
    List,
    /// 取り込み元を消す（その initial 記録と登録候補も消す。取り込みファイルは消さない）
    Remove { id: u32 },
    /// 取り込みをすべて消す（initial 層、台帳、登録候補。usage 層は残す）
    Reset,
    /// 辞書にない語の登録候補を一覧する
    Candidates,
    /// 登録候補をユーザー辞書に入れる（`candidates` で表示した番号）
    Accept {
        #[arg(required = true)]
        numbers: Vec<usize>,
        /// 品詞（簡略名）
        #[arg(long, default_value = "名詞")]
        pos: String,
    },
}

#[derive(clap::Args, Debug, Clone)]
pub struct DataArgs {
    /// ユーザーデータの置き場所
    #[arg(long, value_name = "DIR", global = true)]
    pub user_dir: Option<PathBuf>,
    /// エンジンが動いていても学習ファイルを書き換える（エンジンのメモリ上の学習で上書きされることがある）
    #[arg(long, global = true)]
    pub force: bool,
    #[command(subcommand)]
    pub command: DataCommand,
}

#[derive(Subcommand, Debug, Clone)]
pub enum DataCommand {
    /// ユーザー辞書、学習、プロファイル、取り込み元の台帳を1つのファイルに書き出す
    Export { file: PathBuf },
    /// エクスポートしたファイルで上書きする（マージしない）。不正なファイルでは何も変えない
    Import { file: PathBuf },
}

/// サブコマンドの名前。最初の引数がこれなら、サブコマンドとして解釈する。
pub const SUBCOMMAND_NAMES: [&str; 7] = [
    "convert",
    "session",
    "user-dict",
    "learn",
    "profile",
    "ingest",
    "data",
];

/// `ikari [OPTIONS] <READING>`（フェーズ0の形）。`--help` はこちらの説明を出す。
#[derive(Parser, Debug)]
#[command(
    name = "ikari",
    version,
    about = "ikari-ime の変換を試す CLI",
    after_help = "サブコマンド:\n  convert    読みを変換する（`ikari <READING>` と同じ。読みがサブコマンド名と同じ綴りのとき用）\n  session    1行ずつ読みを入力し、番号で確定する\n  user-dict  ユーザー辞書（add / remove / list / import / check）\n  learn      学習データの確認とリセット（show / reset）\n  profile    属性プロファイル（presets / show / set / clear）\n  ingest     文章の取り込み（prompt / run / list / remove / reset / candidates / accept）\n  data       ユーザーデータのエクスポートとインポート（export / import）\n各サブコマンドの詳しい説明は `ikari <サブコマンド> --help`。"
)]
struct ConvertCli {
    #[command(flatten)]
    args: Args,
}

#[derive(Parser, Debug)]
#[command(name = "ikari", version)]
struct SubCli {
    #[command(subcommand)]
    command: Command,
}

/// コマンドラインを解釈する。最初の引数がサブコマンド名ならサブコマンド、そうでなければ変換。
pub fn parse_command<I, T>(args: I) -> Result<Command, clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let args: Vec<OsString> = args.into_iter().map(Into::into).collect();
    let is_subcommand = args
        .get(1)
        .and_then(|a| a.to_str())
        .is_some_and(|a| SUBCOMMAND_NAMES.contains(&a));
    if is_subcommand {
        SubCli::try_parse_from(args).map(|c| c.command)
    } else {
        ConvertCli::try_parse_from(args).map(|c| Command::Convert(c.args))
    }
}

fn parse_bounded(s: &str, min: usize, max: usize) -> Result<usize, String> {
    let n: usize = s
        .parse()
        .map_err(|_| format!("`{s}` は数値として解釈できません"))?;
    if (min..=max).contains(&n) {
        Ok(n)
    } else {
        Err(format!("{min}..={max} の範囲で指定してください"))
    }
}

fn parse_num(s: &str) -> Result<usize, String> {
    parse_bounded(s, 1, usize::MAX)
}

fn parse_page_size(s: &str) -> Result<usize, String> {
    parse_bounded(s, 1, MAX_PAGE_SIZE)
}

/// auto を romaji / kana に解決する。それ以外はそのまま返す。
pub fn resolve_input_mode(mode: InputMode, reading: &str) -> InputMode {
    match mode {
        InputMode::Auto if reading.is_ascii() => InputMode::Romaji,
        InputMode::Auto => InputMode::Kana,
        other => other,
    }
}

/// 辞書ディレクトリの決定（ADR 0004 §8.1）。空の環境変数は未設定として扱う。
/// 引数、環境変数、`user_dict_dir`、`./data/build/dict` の順。`user_dict_dir` は存在するときだけ渡す。
pub fn resolve_dict_dir_with(
    arg: Option<&Path>,
    env: Option<OsString>,
    user_dict_dir: Option<PathBuf>,
) -> PathBuf {
    if let Some(p) = arg {
        return p.to_path_buf();
    }
    match env {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => user_dict_dir.unwrap_or_else(|| PathBuf::from(DEFAULT_DICT_DIR)),
    }
}

/// `domain=often|sometimes` の列を `(分野, 重み)` にする（`ikari profile set`）。
pub fn parse_profile_settings(
    items: &[String],
) -> Result<Vec<(String, ikari_core::ProfileWeight)>, String> {
    items
        .iter()
        .map(|item| {
            let (domain, weight) = item.split_once('=').ok_or_else(|| {
                format!("`{item}` は `分野=often|sometimes` の形で指定してください")
            })?;
            let weight = ikari_core::ProfileWeight::parse(weight)
                .ok_or_else(|| format!("重みは often か sometimes です: {weight}"))?;
            Ok((domain.to_string(), weight))
        })
        .collect()
}

/// jis モードの入力列を (キー, シフト) の列にする。'+' は次の1キーだけをシフトする。
pub fn parse_jis_keys(s: &str) -> Result<Vec<(JisKey, bool)>, String> {
    let mut keys = Vec::new();
    let mut shift = false;
    for c in s.chars() {
        if c == JIS_SHIFT_PREFIX {
            if shift {
                return Err(format!("'{JIS_SHIFT_PREFIX}' が連続しています"));
            }
            shift = true;
            continue;
        }
        let key = JisKey::from_legend(c)
            .ok_or_else(|| format!("JIS キーの刻印として解釈できない文字です: {c:?}"))?;
        keys.push((key, shift));
        shift = false;
    }
    if shift {
        return Err(format!(
            "末尾の '{JIS_SHIFT_PREFIX}' の後にキーがありません"
        ));
    }
    Ok(keys)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn try_cli(args: &[&str]) -> Result<Command, clap::Error> {
        let mut v = vec!["ikari"];
        v.extend_from_slice(args);
        parse_command(v)
    }

    /// `convert` の引数として解釈する（サブコマンドなしでも `convert` 付きでも同じ）
    fn parse(args: &[&str]) -> Result<Args, clap::Error> {
        match try_cli(args)? {
            Command::Convert(a) => Ok(a),
            other => panic!("not a convert command: {other:?}"),
        }
    }

    fn sub(args: &[&str]) -> Command {
        try_cli(args).unwrap()
    }

    #[test]
    fn defaults() {
        let a = parse(&["ひんし"]).unwrap();
        assert_eq!(a.reading, "ひんし");
        assert_eq!(a.common.dict, None);
        assert_eq!(a.common.user_dir, None);
        assert_eq!(a.common.ranker, RankerSet::Full);
        assert!(!a.common.no_analyzer);
        assert!(!a.common.no_packs);
        assert_eq!(a.common.vectors, None);
        assert_eq!(a.common.profile, None);
        assert_eq!(a.common.ranker_config, None);
        assert_eq!(a.context, "");
        assert!(!a.secure);
        assert_eq!(a.commit, None);
        assert_eq!(a.input, InputMode::Auto);
        assert_eq!(a.num, 9);
        assert_eq!(a.page_size, 9);
        assert_eq!(a.format, OutputFormat::Text);
        assert!(!a.segments && !a.timing);
    }

    #[test]
    fn all_options() {
        let a = parse(&[
            "--dict",
            "d",
            "--user-dir",
            "u",
            "--ranker",
            "cost",
            "--no-analyzer",
            "--vectors",
            "v.bin",
            "--profile",
            "it",
            "--ranker-config",
            "r.conf",
            "--context",
            "ユーザー辞書に",
            "--secure",
            "--commit",
            "2",
            "--input",
            "jis",
            "--romaji-table",
            "t.tsv",
            "-n",
            "20",
            "--page-size",
            "5",
            "--format",
            "tsv",
            "--segments",
            "--timing",
            "hinshi",
        ])
        .unwrap();
        assert_eq!(a.common.dict, Some(PathBuf::from("d")));
        assert_eq!(a.common.user_dir, Some(PathBuf::from("u")));
        assert_eq!(a.common.ranker, RankerSet::Cost);
        assert!(a.common.no_analyzer);
        assert_eq!(a.common.vectors, Some(PathBuf::from("v.bin")));
        assert_eq!(a.common.profile.as_deref(), Some("it"));
        assert_eq!(a.common.ranker_config, Some(PathBuf::from("r.conf")));
        assert_eq!(a.context, "ユーザー辞書に");
        assert!(a.secure && a.segments && a.timing);
        assert_eq!(a.commit, Some(2));
        assert_eq!(a.input, InputMode::Jis);
        assert_eq!(a.romaji_table, Some(PathBuf::from("t.tsv")));
        assert_eq!(a.num, 20);
        assert_eq!(a.page_size, 5);
        assert_eq!(a.format, OutputFormat::Tsv);
    }

    #[test]
    fn explicit_convert_subcommand_is_the_same() {
        let a = parse(&["convert", "--ranker", "cost", "ひんし"]).unwrap();
        assert_eq!(a.reading, "ひんし");
        assert_eq!(a.common.ranker, RankerSet::Cost);
        // サブコマンド名と同じ綴りの読み
        assert_eq!(parse(&["convert", "data"]).unwrap().reading, "data");
    }

    #[test]
    fn invalid_arguments_are_rejected() {
        assert!(try_cli(&[]).is_err());
        assert!(try_cli(&["--page-size", "0", "a"]).is_err());
        assert!(try_cli(&["--page-size", "10", "a"]).is_err());
        assert!(try_cli(&["-n", "0", "a"]).is_err());
        assert!(try_cli(&["--input", "bogus", "a"]).is_err());
        assert!(try_cli(&["--format", "json", "a"]).is_err());
        assert!(try_cli(&["--ranker", "bogus", "a"]).is_err());
        assert!(try_cli(&["--commit", "0", "a"]).is_err());
        // 読みの引数とサブコマンドは同時に使えない
        assert!(try_cli(&["--secure", "data", "export", "f"]).is_err());
    }

    #[test]
    fn subcommands_parse() {
        assert!(matches!(
            sub(&[
                "user-dict",
                "add",
                "いかり",
                "ikari-ime",
                "短縮よみ",
                "--comment",
                "c"
            ]),
            Command::UserDict(UserDictArgs {
                command: UserDictCommand::Add { .. },
                ..
            })
        ));
        // 親のオプションは、サブコマンドの後ろにも書ける
        let Command::UserDict(u) = sub(&["user-dict", "list", "--user-dir", "u"]) else {
            panic!()
        };
        assert_eq!(u.user_dir, Some(PathBuf::from("u")));
        let Command::UserDict(u) = sub(&[
            "user-dict",
            "import",
            "--format",
            "mozc",
            "f.txt",
            "--user-dir",
            "u",
        ]) else {
            panic!()
        };
        assert!(matches!(
            u.command,
            UserDictCommand::Import {
                format: UserDictFormat::Mozc,
                ..
            }
        ));
        assert!(matches!(
            sub(&["user-dict", "check", "--dict", "d"]),
            Command::UserDict(_)
        ));
        assert!(matches!(
            sub(&["learn", "reset", "--layer", "initial"]),
            Command::Learn(LearnArgs {
                command: LearnCommand::Reset {
                    layer: LayerArg::Initial
                },
                ..
            })
        ));
        assert!(matches!(
            sub(&["learn", "show", "--reading", "ひんし"]),
            Command::Learn(_)
        ));
        assert!(matches!(sub(&["profile", "presets"]), Command::Profile(_)));
        assert!(matches!(
            sub(&["profile", "set", "it=often", "infra=sometimes"]),
            Command::Profile(_)
        ));
        assert!(matches!(sub(&["ingest", "prompt"]), Command::Ingest(_)));
        assert!(matches!(
            sub(&["ingest", "run", "f.jsonl", "--dry-run", "--apply-profile"]),
            Command::Ingest(_)
        ));
        assert!(matches!(
            sub(&["ingest", "accept", "1", "3", "--pos", "人名"]),
            Command::Ingest(_)
        ));
        assert!(matches!(
            sub(&["data", "export", "out.txt"]),
            Command::Data(_)
        ));
        assert!(matches!(
            sub(&["session", "--script", "s.tsv", "--ranker", "cost"]),
            Command::Session(_)
        ));
    }

    #[test]
    fn incomplete_subcommands_are_rejected() {
        for args in [
            vec!["user-dict"],
            vec!["user-dict", "add", "a", "b"],
            vec!["learn", "reset"],
            vec!["learn", "reset", "--layer", "bogus"],
            vec!["profile", "set"],
            vec!["ingest", "remove"],
            vec!["ingest", "remove", "x"],
            vec!["ingest", "accept"],
            vec!["data", "export"],
            vec!["data", "bogus", "f"],
        ] {
            assert!(try_cli(&args).is_err(), "{args:?}");
        }
    }

    #[test]
    fn auto_detection() {
        assert_eq!(
            resolve_input_mode(InputMode::Auto, "hinshi"),
            InputMode::Romaji
        );
        assert_eq!(
            resolve_input_mode(InputMode::Auto, "ひんし"),
            InputMode::Kana
        );
        // ASCII が混ざっていても、非 ASCII を含めば kana
        assert_eq!(
            resolve_input_mode(InputMode::Auto, "へんk"),
            InputMode::Kana
        );
        assert_eq!(resolve_input_mode(InputMode::Kana, "abc"), InputMode::Kana);
        assert_eq!(
            resolve_input_mode(InputMode::Romaji, "ひ"),
            InputMode::Romaji
        );
        assert_eq!(resolve_input_mode(InputMode::Jis, "tg"), InputMode::Jis);
    }

    #[test]
    fn dict_dir_precedence() {
        let arg = PathBuf::from("a");
        assert_eq!(
            resolve_dict_dir_with(Some(&arg), Some("env".into()), None),
            PathBuf::from("a")
        );
        assert_eq!(
            resolve_dict_dir_with(None, Some("env".into()), None),
            PathBuf::from("env")
        );
        assert_eq!(
            resolve_dict_dir_with(None, Some("".into()), None),
            PathBuf::from(DEFAULT_DICT_DIR)
        );
        assert_eq!(
            resolve_dict_dir_with(None, None, None),
            PathBuf::from(DEFAULT_DICT_DIR)
        );
    }

    #[test]
    fn user_data_dict_sits_between_the_env_and_the_default() {
        let user = Some(PathBuf::from("/u/dict"));
        let arg = PathBuf::from("a");
        assert_eq!(
            resolve_dict_dir_with(Some(&arg), Some("env".into()), user.clone()),
            arg
        );
        assert_eq!(
            resolve_dict_dir_with(None, Some("env".into()), user.clone()),
            PathBuf::from("env")
        );
        assert_eq!(
            resolve_dict_dir_with(None, Some("".into()), user.clone()),
            PathBuf::from("/u/dict")
        );
        assert_eq!(
            resolve_dict_dir_with(None, None, user),
            PathBuf::from("/u/dict")
        );
        assert_eq!(
            resolve_dict_dir_with(None, None, None),
            PathBuf::from(DEFAULT_DICT_DIR)
        );
    }

    #[test]
    fn profile_settings_parse() {
        let s = parse_profile_settings(&["it=often".to_string(), "novel=sometimes".to_string()])
            .unwrap();
        assert_eq!(s[0], ("it".to_string(), ikari_core::ProfileWeight::OFTEN));
        assert_eq!(
            s[1],
            ("novel".to_string(), ikari_core::ProfileWeight::SOMETIMES)
        );
        assert!(parse_profile_settings(&["it".to_string()]).is_err());
        assert!(parse_profile_settings(&["it=always".to_string()]).is_err());
    }

    #[test]
    fn jis_shift_notation() {
        assert_eq!(
            parse_jis_keys("t+e").unwrap(),
            vec![(JisKey::T, false), (JisKey::E, true)]
        );
        // '+' は次の1キーだけに効く
        assert_eq!(
            parse_jis_keys("+ee").unwrap(),
            vec![(JisKey::E, true), (JisKey::E, false)]
        );
        assert_eq!(parse_jis_keys("").unwrap(), vec![]);
    }

    #[test]
    fn jis_errors() {
        assert!(parse_jis_keys("t+").is_err());
        assert!(parse_jis_keys("++e").is_err());
        assert!(parse_jis_keys("あ").is_err());
        assert!(parse_jis_keys("T").is_err());
    }
}
