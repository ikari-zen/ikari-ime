//! `ikari-dict-builder` CLI（ADR 0001 §8）。

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use clap::{Parser, Subcommand};
use ikari_dict_builder::{
    BuildConfig, BuildOptions, ChainConfig, DEFAULT_CHAIN_COST, DEFAULT_CLUSTERS,
    DEFAULT_KMEANS_ITERS, DEFAULT_MUNICIPALITY_CHAIN_COST, DEFAULT_TOWN_COST, Policy, PostalConfig,
    TownConfig, VectorsConfig, build_emoji_file, build_town_lex, build_vectors, build_with,
    read_chive_vocab, verify, verify_vectors,
    webwords::{self, MIN_DF_FLOOR as WEBWORDS_MIN_DF_FLOOR, WebClass},
};
use ikari_dict_format::VECTORS_FILE_NAME;

const DEFAULT_UNK_POS: &str = "名詞,普通名詞,一般,*,*,*";
const DEFAULT_UNK_COST: &str = "10000";
const DEFAULT_LABEL: &str = "sudachi-core-20260723";

/// SudachiDict の CSV から ikari-ime の辞書（grammar.bin と system.dict）を作る。
#[derive(Parser, Debug)]
#[command(
    name = "ikari-dict-builder",
    version,
    args_conflicts_with_subcommands = true
)]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,

    /// lex.csv（複数指定できる。small と core を連結して1つの辞書にする）
    #[arg(
        long,
        value_name = "FILE",
        required_unless_present = "verify",
        conflicts_with = "verify"
    )]
    lex: Vec<PathBuf>,

    /// Full の notcore_lex.csv（`--lex` の後ろに連結する。辞書の行にはなるが、品詞の統計と活用規則には数えない。ADR 0012 §11.3）
    #[arg(long, value_name = "FILE", conflicts_with = "verify")]
    notcore_lex: Vec<PathBuf>,

    /// 住所の連なりの lex.csv（`town` の `--chain-out`。notcore と同じ扱いで、同じ（読み、表記、左右 ID、品詞）の Core の行にも勝つ。ADR 0030 §12）
    #[arg(long, value_name = "FILE", conflicts_with = "verify")]
    chain_lex: Vec<PathBuf>,

    /// matrix.def
    #[arg(
        long,
        value_name = "FILE",
        required_unless_present = "verify",
        conflicts_with = "verify"
    )]
    matrix: Option<PathBuf>,

    /// 出力ディレクトリ（grammar.bin と system.dict を書く）
    #[arg(
        long,
        value_name = "DIR",
        required_unless_present = "verify",
        conflicts_with = "verify"
    )]
    out: Option<PathBuf>,

    /// 未知語に使う品詞（品詞1〜活用形をカンマで結合したもの）
    #[arg(long, value_name = "POS", default_value = DEFAULT_UNK_POS)]
    unk_pos: String,

    /// 未知語1文字あたりのコスト
    #[arg(long, value_name = "COST", default_value = DEFAULT_UNK_COST, allow_negative_numbers = true)]
    unk_cost: i16,

    /// 辞書のメタ情報 `source` に入れるラベル
    #[arg(long, value_name = "LABEL", default_value = DEFAULT_LABEL)]
    label: String,

    /// コスト調整ポリシー（述語<TAB>値の TSV）。省略すると同梱の policy/default.tsv
    #[arg(long, value_name = "FILE")]
    policy: Option<PathBuf>,

    /// chiVe のテキスト。語彙を読み、ポリシーの lemma_rank の判定に使う
    #[arg(long, value_name = "FILE")]
    chive_vocab: Option<PathBuf>,

    /// ポリシーのレポート（Markdown）の出力先
    #[arg(long, value_name = "FILE")]
    policy_report: Option<PathBuf>,

    /// 連接表の補正の δ が閾値 T 以下でも失敗にしない（掃引だけに使う。ADR 0054 §5.1）
    #[arg(long, conflicts_with = "verify")]
    allow_weak_connection: bool,

    /// ビルドせず、辞書ディレクトリの全エントリを検証する（vectors.bin があればそれも）
    #[arg(long, value_name = "DIR")]
    verify: Option<PathBuf>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// emoji-data の emoji.json から emoji.tsv を作る（ADR 0027 §5.3）
    Emoji {
        /// emoji-data の emoji.json
        #[arg(long, value_name = "FILE")]
        json: PathBuf,
        /// emoji.json の取得元のコミット（emoji.tsv の source 行に書く）
        #[arg(long, value_name = "COMMIT")]
        commit: String,
        /// 出力する emoji.tsv のパス
        #[arg(long, value_name = "FILE")]
        out: PathBuf,
    },
    /// ABR の町字マスター（mt_town_all.csv）から、辞書に足す地名の lex.csv を作る（ADR 0030）。
    /// できたファイルは `--notcore-lex` に渡す
    Town {
        /// ABR の mt_town_all.csv
        #[arg(long, value_name = "FILE")]
        csv: PathBuf,
        /// Sudachi の lex.csv（small、core、notcore。同じ表記と読みの行を足さないために読む）
        #[arg(long, value_name = "FILE", required = true)]
        lex: Vec<PathBuf>,
        /// 出力する lex.csv のパス
        #[arg(long, value_name = "FILE")]
        out: PathBuf,
        /// 行のコスト（Sudachi の地名の最頻値が既定）
        #[arg(long, value_name = "COST", default_value_t = DEFAULT_TOWN_COST, allow_negative_numbers = true)]
        cost: i16,
        /// 住所の連なり（都道府県 + 市区町村 + 町名。読みがぴったり一致したときだけ出る）の lex.csv の出力先。
        /// 省略すると連なりを作らない。辞書のビルドには `--chain-lex` で渡す（ADR 0030 §12）
        #[arg(long, value_name = "FILE")]
        chain_out: Option<PathBuf>,
        /// 住所の連なりの行のコスト
        #[arg(long, value_name = "COST", default_value_t = DEFAULT_CHAIN_COST, allow_negative_numbers = true)]
        chain_cost: i16,
        /// 「都道府県 + 市区町村」だけの連なり（町名がない形）の行のコスト（ADR 0030 §13）
        #[arg(long, value_name = "COST", default_value_t = DEFAULT_MUNICIPALITY_CHAIN_COST, allow_negative_numbers = true)]
        chain_municipality_cost: i16,
        /// ABR「町字・郵便番号変換表」の CSV。`--postal-out` と一緒に指定すると `postal.tsv` を作る（ADR 0032 §3）
        #[arg(long, value_name = "FILE", requires = "postal_out")]
        postal_csv: Option<PathBuf>,
        /// 出力する postal.tsv のパス
        #[arg(long, value_name = "FILE", requires = "postal_csv")]
        postal_out: Option<PathBuf>,
    },
    /// Wikipedia 日本語版から集めた外来語の収集・判定・辞書の行の生成（ADR 0055。issue #113）
    Webwords {
        #[command(subcommand)]
        command: WebwordsCommand,
    },
    /// chiVe のテキストから vectors.bin を作る
    Vectors {
        /// chiVe の word2vec テキスト
        #[arg(long, value_name = "FILE")]
        chive: PathBuf,
        /// system.dict があるディレクトリ
        #[arg(long, value_name = "DIR")]
        dict: PathBuf,
        /// 辞書を作ったときと同じ lex.csv（同じ順。small、core、notcore）。chiVe にない lemma に、Sudachi の分割の
        /// 部分のベクトルの平均を付ける（ADR 0012 §12）。省略すると付けない
        #[arg(long, value_name = "FILE")]
        lex: Vec<PathBuf>,
        /// 出力する vectors.bin のパス
        #[arg(long, value_name = "FILE")]
        out: PathBuf,
        #[arg(long, default_value_t = DEFAULT_CLUSTERS)]
        clusters: u16,
        #[arg(long, default_value_t = DEFAULT_KMEANS_ITERS)]
        kmeans_iters: u32,
        /// メタ情報 `source` に入れるラベル
        #[arg(long, default_value = "chive-1.3-mc90")]
        label: String,
    },
}

#[derive(Subcommand, Debug)]
enum WebwordsCommand {
    /// pages-articles の本文から片仮名の連なりを数える（tf・df・cue）
    Collect {
        /// pages-articles の bz2（分割の全ファイル）
        #[arg(long, value_name = "FILE", required = true, num_args = 1..)]
        pages_articles: Vec<PathBuf>,
        /// 収集の一覧の出力先
        #[arg(long, value_name = "FILE")]
        out: PathBuf,
        /// 出力する df の下限
        #[arg(long, value_name = "N", default_value_t = WEBWORDS_MIN_DF_FLOOR)]
        min_df: u32,
    },
    /// 収集の一覧に区分（普通名詞・固有名詞・迷う語）を付け、schema=2 の一覧を作る
    Classify {
        /// `collect` の出力
        #[arg(long, value_name = "FILE")]
        collected: PathBuf,
        /// jawiki の page の SQL ダンプ
        #[arg(long, value_name = "FILE")]
        page_sql: PathBuf,
        /// jawiki の redirect の SQL ダンプ
        #[arg(long, value_name = "FILE")]
        redirect_sql: PathBuf,
        /// jawiki の page_props の SQL ダンプ
        #[arg(long, value_name = "FILE")]
        page_props_sql: PathBuf,
        /// jawiki の categorylinks の SQL ダンプ
        #[arg(long, value_name = "FILE")]
        categorylinks_sql: PathBuf,
        /// jawiki の linktarget の SQL ダンプ
        #[arg(long, value_name = "FILE")]
        linktarget_sql: PathBuf,
        /// カテゴリの規則表。省略すると同梱の webwords/categories.tsv
        #[arg(long, value_name = "FILE")]
        categories: Option<PathBuf>,
        /// Sudachi（small、core、notcore）と ABR の lex.csv（既知の語の照合。複数指定できる）
        #[arg(long, value_name = "FILE", required = true)]
        lex: Vec<PathBuf>,
        /// 一覧の 1 行目の source（`jawiki-<日付>`）
        #[arg(long, value_name = "LABEL")]
        source: String,
        /// 一覧の 1 行目の sha256（pages-articles の各ファイルの sha256 をつないだものの sha256）
        #[arg(long, value_name = "HEX")]
        sha256: String,
        /// 一覧の 1 行目の articles（数えた記事数）
        #[arg(long, value_name = "N")]
        articles: u64,
        /// 一覧の 1 行目の sudachi（F1 に使った版）
        #[arg(long, value_name = "LABEL")]
        sudachi: String,
        /// 出力する df の下限
        #[arg(long, value_name = "N", default_value_t = WEBWORDS_MIN_DF_FLOOR)]
        min_df: u32,
        /// schema=2 の一覧の出力先
        #[arg(long, value_name = "FILE")]
        out: PathBuf,
        /// レポート（Markdown）の出力先
        #[arg(long, value_name = "FILE")]
        report: Option<PathBuf>,
    },
    /// schema=2 の一覧から、辞書に足す webwords_lex.csv を作る。できたファイルは `--lex` と `--notcore-lex` に渡す
    Lex {
        /// schema=2 の一覧
        #[arg(long, value_name = "FILE")]
        list: PathBuf,
        /// Sudachi（small、core、notcore）と ABR の lex.csv（既知の語の照合と ID の数え方。複数指定できる）
        #[arg(long, value_name = "FILE", required = true)]
        lex: Vec<PathBuf>,
        /// 出力する lex.csv のパス
        #[arg(long, value_name = "FILE")]
        out: PathBuf,
        /// レポート（Markdown）の出力先
        #[arg(long, value_name = "FILE")]
        report: Option<PathBuf>,
        /// df の下限（掃引用。省略すると配る一覧の下限）
        #[arg(long, value_name = "N")]
        min_df: Option<u32>,
        /// 普通名詞のコスト（掃引用。省略すると既定）
        #[arg(long, value_name = "COST", allow_negative_numbers = true)]
        cost: Option<i16>,
        /// 区分ごとの出発点からの差（掃引用。`proper.person=2000` の形。複数指定できる）
        #[arg(long, value_name = "CLASS=VALUE", value_parser = parse_proper_offset, allow_negative_numbers = true)]
        proper_offset: Vec<(WebClass, i16)>,
        /// F5（連結を足すかの規則）を当てない（掃引用）
        #[arg(long)]
        no_f5: bool,
    },
}

/// `proper.person=2000` を (区分, 値) にする。普通名詞（common）の差は `--cost` で指定するので受けない。
fn parse_proper_offset(text: &str) -> Result<(WebClass, i16), String> {
    let (class, value) = text
        .split_once('=')
        .ok_or_else(|| format!("`区分=値` の形ではありません: {text}"))?;
    let class = WebClass::parse(class)
        .filter(|c| c.is_proper())
        .ok_or_else(|| format!("区分は proper.person / proper.place / proper.general: {class}"))?;
    let value = value
        .parse::<i16>()
        .map_err(|e| format!("値が整数ではありません: {value} ({e})"))?;
    Ok((class, value))
}

fn run_webwords(command: WebwordsCommand) -> Result<(), Box<dyn std::error::Error>> {
    let rows = match command {
        WebwordsCommand::Collect {
            pages_articles,
            out,
            min_df,
        } => webwords::collect(&webwords::CollectConfig {
            pages_articles,
            out,
            min_df,
        })?,
        WebwordsCommand::Classify {
            collected,
            page_sql,
            redirect_sql,
            page_props_sql,
            categorylinks_sql,
            linktarget_sql,
            categories,
            lex,
            source,
            sha256,
            articles,
            sudachi,
            min_df,
            out,
            report,
        } => webwords::classify(&webwords::ClassifyConfig {
            collected,
            page_sql,
            redirect_sql,
            page_props_sql,
            categorylinks_sql,
            linktarget_sql,
            categories,
            lex,
            header: webwords::ListHeader {
                source,
                sha256,
                articles,
                sudachi,
                // 規則表の sha256 は classify が埋める
                rules: String::new(),
            },
            min_df,
            out,
            report,
        })?,
        WebwordsCommand::Lex {
            list,
            lex,
            out,
            report,
            min_df,
            cost,
            proper_offset,
            no_f5,
        } => webwords::lex(&webwords::LexConfig {
            list,
            lex,
            out,
            report,
            min_df,
            cost,
            proper_offsets: proper_offset,
            no_f5,
        })?,
    };
    println!("{rows} rows");
    Ok(())
}

fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    let started = Instant::now();
    if let Some(Command::Webwords { command }) = args.command {
        return run_webwords(command);
    }
    if let Some(Command::Emoji { json, commit, out }) = &args.command {
        let report = build_emoji_file(json, commit, out)?;
        println!("{report}");
        return Ok(());
    }
    if let Some(Command::Town {
        csv,
        lex,
        out,
        cost,
        chain_out,
        chain_cost,
        chain_municipality_cost,
        postal_csv,
        postal_out,
    }) = &args.command
    {
        let report = build_town_lex(&TownConfig {
            csv: csv.clone(),
            sudachi_lex: lex.clone(),
            out: out.clone(),
            cost: *cost,
            chain: chain_out.clone().map(|out| ChainConfig {
                out,
                cost: *chain_cost,
                municipality_cost: *chain_municipality_cost,
            }),
            postal: postal_csv
                .clone()
                .zip(postal_out.clone())
                .map(|(csv, out)| PostalConfig { csv, out }),
        })?;
        println!("{report}");
        eprintln!("built in {:.2?}", started.elapsed());
        return Ok(());
    }
    if let Some(Command::Vectors {
        chive,
        dict,
        lex,
        out,
        clusters,
        kmeans_iters,
        label,
    }) = args.command
    {
        let mut cfg = VectorsConfig::new(chive, dict, out);
        cfg.lex = lex;
        cfg.clusters = clusters;
        cfg.kmeans_iters = kmeans_iters;
        cfg.label = label;
        let report = build_vectors(&cfg)?;
        println!("{report}");
        eprintln!("built in {:.2?}", started.elapsed());
        return Ok(());
    }
    if let Some(dir) = args.verify {
        let report = verify(&dir)?;
        println!(
            "OK: grammar_id={:#018x} entries={} readings={} pos_count={} lemmas={} surfaces={} conj_rules={}",
            report.grammar_id,
            report.entries,
            report.readings,
            report.pos_count,
            report.lemmas,
            report.surfaces,
            report.conj_rules
        );
        if dir.join(VECTORS_FILE_NAME).exists() {
            let v = verify_vectors(&dir)?;
            println!(
                "OK: vectors dict_id={:#018x} dims={} rows={} clusters={} lemmas_with_vector={}",
                v.dict_id, v.dims, v.rows, v.clusters, v.lemmas_with_vector
            );
        }
        eprintln!("verified in {:.2?}", started.elapsed());
        return Ok(());
    }

    // clap の required_unless_present で保証されているが、unwrap は使わない
    let (Some(matrix), Some(out_dir)) = (args.matrix, args.out) else {
        return Err("--matrix and --out are required".into());
    };
    let cfg = BuildConfig {
        lex: args.lex.clone(),
        matrix,
        out_dir,
        unk_pos: args.unk_pos.clone(),
        unk_cost: args.unk_cost,
        label: args.label.clone(),
    };
    let mut opts = BuildOptions {
        notcore_lex: args.notcore_lex.clone(),
        chain_lex: args.chain_lex.clone(),
        allow_weak_connection: args.allow_weak_connection,
        ..BuildOptions::default()
    };
    if let Some(path) = &args.policy {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        opts.policy = Policy::parse(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    if let Some(path) = &args.chive_vocab {
        eprintln!("reading chiVe vocabulary from {} ...", path.display());
        opts.chive_vocab = Some(read_chive_vocab(path)?);
        opts.chive_vocab_label = Some(
            path.file_stem()
                .map_or_else(|| "chive".into(), |n| n.to_string_lossy().into_owned()),
        );
    }
    let report = build_with(&cfg, &opts)?;
    println!("{report}");
    if let Some(path) = &args.policy_report {
        let mut md = report
            .policy
            .to_markdown(&opts.policy, opts.chive_vocab.as_ref().map(|v| v.len()));
        md.push_str(&report.notcore_markdown());
        md.push_str(&report.connection_markdown());
        std::fs::write(path, md).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    }
    eprintln!("built in {:.2?}", started.elapsed());
    Ok(())
}

fn main() -> ExitCode {
    match run(Args::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    #[test]
    fn cli_definition_is_consistent() {
        Args::command().debug_assert();
    }

    #[test]
    fn proper_offset_accepts_only_proper_classes() {
        assert_eq!(
            parse_proper_offset("proper.person=-2000"),
            Ok((WebClass::ProperPerson, -2000))
        );
        assert!(parse_proper_offset("common=1").is_err());
        assert!(parse_proper_offset("proper.place").is_err());
        assert!(parse_proper_offset("proper.place=x").is_err());
    }
}
