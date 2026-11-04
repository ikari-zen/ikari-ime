//! `ikari-eval` のコマンドラインの定義（clap）。

use std::path::PathBuf;

use clap::builder::{PossibleValuesParser, TypedValueParser};
use clap::{Parser, Subcommand, ValueEnum};
use ikari_eval::EvalConfig;
use ikari_stack::RankerSet;

#[derive(Parser, Debug)]
#[command(
    name = "ikari-eval",
    about = "同音異義語の評価セットを実行してレポートを出す",
    args_conflicts_with_subcommands = true
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Option<Command>,
    #[command(flatten)]
    pub(crate) run: RunArgs,
}

#[derive(Subcommand, Debug)]
pub(crate) enum Command {
    /// 2つの結果（--out-cases の TSV）のケースごとの順位の差分を出す
    Compare { a: PathBuf, b: PathBuf },
    /// 期待値が候補に出ないケースを調べ、--write で tags に `unreachable` を書く
    Reach(ReachArgs),
    /// 文節の区切りの適合率・再現率・F1 と完全一致率（ADR 0006 §7.1-2。segments.tsv）
    Segments(SegmentsArgs),
    /// 目標の表記に届くまでの打鍵数を、今の UI と文節の UI で数える（ADR 0006 §7.1-3）
    Ops(OpsArgs),
    /// 再変換で推定した読みの一致率（既存のセットの表記と読みの組。ADR 0006 §7.1-5）
    Reading(ReadingArgs),
}

#[derive(clap::Args, Debug)]
pub(crate) struct ReadingArgs {
    #[command(flatten)]
    pub(crate) engine: EngineArgs,
    /// 評価セット（TSV、複数可）。省略時は既存の 4 セット
    #[arg(long = "set")]
    pub(crate) sets: Vec<PathBuf>,
    /// split 列で絞る。split 列のないケースはどれにも含める
    #[arg(long, value_enum, default_value = "all")]
    pub(crate) split: SplitArg,
    /// 一致しなかった組を先頭からこの件数まで表に出す
    #[arg(long, default_value_t = 30)]
    pub(crate) list: usize,
}

#[derive(clap::Args, Debug)]
pub(crate) struct OpsArgs {
    #[command(flatten)]
    pub(crate) engine: EngineArgs,
    /// 評価セット（TSV）。expected_segments 列があれば segments.tsv の形式（target を目標にする）、
    /// なければ既存のセット（expected の別解のどれかを目標にする）
    #[arg(long, default_value = "eval/sets/segments.tsv")]
    pub(crate) set: PathBuf,
    /// split 列で絞る。split 列のないケースはどれにも含める
    #[arg(long, value_enum, default_value = "all")]
    pub(crate) split: SplitArg,
    /// 各文節でこの順位までの候補を使う
    #[arg(long, default_value_t = 50)]
    pub(crate) max_rank: usize,
    /// 候補ウィンドウの 1 ページの候補数（番号キーの数）
    #[arg(long, default_value_t = 9)]
    pub(crate) page_size: usize,
    /// 文節の UI の探索の打鍵数の上限
    #[arg(long, default_value_t = 40)]
    pub(crate) max_keys: usize,
    /// 文節の UI のほうが打鍵が多い（または届かない）ケースがあれば終了コード 1
    #[arg(long)]
    pub(crate) fail_on_worse: bool,
}

/// 構成を選ぶフラグ（`RunArgs` と同じ意味。文節の評価の subcommand が共有する）
#[derive(clap::Args, Debug)]
pub(crate) struct EngineArgs {
    /// 辞書ディレクトリ。省略時は $IKARI_DICT_DIR、なければ ./data/build/dict
    #[arg(long)]
    pub(crate) dict: Option<PathBuf>,
    /// 再現性のため、既定は cost（LM なし）
    #[arg(long, value_parser = ranker_value_parser(), default_value = RankerSet::Cost.as_str())]
    pub(crate) ranker: RankerSet,
    /// 文脈の解析をしない（BOS を固定する）。full のとき
    #[arg(long)]
    pub(crate) no_analyzer: bool,
    /// vectors.bin（省略時は <dict>/vectors.bin があれば使う）
    #[arg(long)]
    pub(crate) vectors: Option<PathBuf>,
    /// Ranker の重みの上書き（key=value の行）
    #[arg(long)]
    pub(crate) ranker_config: Option<PathBuf>,
    /// zenz の GGUF（feature lm が要る）。付けると LM つきの数値になる（参考）
    #[arg(long)]
    pub(crate) lm: Option<PathBuf>,
    /// LM の待ち時間の上限（ms）
    #[arg(long)]
    pub(crate) lm_budget_ms: Option<u64>,
    /// 変換 1 回の LM の締め切り（ms）。省略時は `ConvertOptions.lm_budget`
    #[arg(long)]
    pub(crate) lm_deadline_ms: Option<u64>,
    /// 学習ストアの置き場所。省略時は一時ディレクトリ（利用者のデータを評価に混ぜない）。
    /// 指定したディレクトリに user_dict.tsv があれば、ユーザー辞書として重ねる
    #[arg(long)]
    pub(crate) user_dir: Option<PathBuf>,
    /// 同梱の語彙集（IT）を重ねない（比較用。既定は重ねる。エンジンと同じ構成）
    #[arg(long)]
    pub(crate) no_packs: bool,
    /// 住所 ↔ 郵便番号の表（postal.tsv）。既定は `<dict>/postal.tsv`（なければ郵便番号の候補は出ない。ADR 0032）
    #[arg(long)]
    pub(crate) postal: Option<PathBuf>,
}

#[derive(clap::Args, Debug)]
pub(crate) struct SegmentsArgs {
    #[command(flatten)]
    pub(crate) engine: EngineArgs,
    /// 文節の評価セット（TSV）
    #[arg(long, default_value = "eval/sets/segments.tsv")]
    pub(crate) set: PathBuf,
    /// split 列で絞る
    #[arg(long, value_enum, default_value = "all")]
    pub(crate) split: SplitArg,
}

#[derive(clap::Args, Debug)]
pub(crate) struct ReachArgs {
    /// 評価セット（TSV。tags 列が必要）
    #[arg(long)]
    pub(crate) set: PathBuf,
    #[arg(long)]
    pub(crate) dict: Option<PathBuf>,
    /// この順位までに出なければ候補にないとみなす
    #[arg(long, default_value_t = EvalConfig::default().max_rank)]
    pub(crate) max_rank: usize,
    /// 評価セットのファイルに `unreachable` の印を書き込む
    #[arg(long)]
    pub(crate) write: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum SetFormat {
    /// ikari-eval の TSV
    Ikari,
    /// AJIMEE-Bench の JSON（eval/scripts/fetch_ajimee.sh で取得）
    Ajimee,
}

/// `--carry-joint` の値（`ikari_core::CarryJoint` に対応）
#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum CarryJointArg {
    None,
    Period,
    Newline,
}

impl From<ikari_core::CarryJoint> for CarryJointArg {
    fn from(j: ikari_core::CarryJoint) -> Self {
        match j {
            ikari_core::CarryJoint::None => Self::None,
            ikari_core::CarryJoint::Period => Self::Period,
            ikari_core::CarryJoint::Newline => Self::Newline,
        }
    }
}

impl RunArgs {
    /// `--carry-fill`・`--carry-joint` から作る組み立ての値
    pub(crate) fn carry_params(&self) -> ikari_core::CarryParams {
        ikari_core::CarryParams {
            fill_chars: self.carry_fill,
            joint: match self.carry_joint {
                CarryJointArg::None => ikari_core::CarryJoint::None,
                CarryJointArg::Period => ikari_core::CarryJoint::Period,
                CarryJointArg::Newline => ikari_core::CarryJoint::Newline,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum SplitArg {
    Dev,
    Test,
    All,
}

/// `--ranker` の値の解釈（候補の表示は `RankerSet::ALL` から作る）
fn ranker_value_parser() -> impl TypedValueParser<Value = RankerSet> {
    PossibleValuesParser::new(RankerSet::ALL.map(RankerSet::as_str)).map(|s| {
        RankerSet::parse(&s).expect("PossibleValuesParser only passes names of RankerSet::ALL")
    })
}

#[derive(clap::Args, Debug)]
pub(crate) struct RunArgs {
    /// 辞書ディレクトリ（grammar.bin と system.dict）。省略時は $IKARI_DICT_DIR、なければ ./data/build/dict
    #[arg(long)]
    pub(crate) dict: Option<PathBuf>,
    /// 評価セット。省略時は eval/sets/homophones.tsv（--set-format ajimee なら data/raw/ajimee/evaluation_items.json）
    #[arg(long)]
    pub(crate) set: Option<PathBuf>,
    /// 評価セットの形式
    #[arg(long, value_enum, default_value = "ikari")]
    pub(crate) set_format: SetFormat,
    /// split 列で絞る。split 列のないケースはどれにも含める
    #[arg(long, value_enum, default_value = "all")]
    pub(crate) split: SplitArg,
    /// `unreachable` の印があるケースも評価する
    #[arg(long)]
    pub(crate) include_unreachable: bool,
    /// ケースごとの warm 計測の繰り返し回数
    #[arg(long, default_value_t = EvalConfig::default().repeat)]
    pub(crate) repeat: usize,
    /// この順位まで正解を探す
    #[arg(long, default_value_t = EvalConfig::default().max_rank)]
    pub(crate) max_rank: usize,
    /// ケース別の結果を TSV で書き出すパス
    #[arg(long)]
    pub(crate) out_cases: Option<PathBuf>,
    /// 全体の1位正解率（0..1）がこれを下回れば終了コード 1
    #[arg(long)]
    pub(crate) min_top1: Option<String>,
    /// 基準の結果（--out-cases の TSV）。基準で1位だったケースが1位でなくなれば、その一覧を出す
    #[arg(long)]
    pub(crate) baseline: Option<PathBuf>,
    /// --baseline との比較で回帰があれば終了コード 1
    #[arg(long, requires = "baseline")]
    pub(crate) fail_on_regression: bool,
    /// profile 列のあるケースをプロファイルの有無の両方で実行して差分を出す。否定・文法ケースが下がれば終了コード 1
    #[arg(long)]
    pub(crate) profile_ab: bool,
    /// 学習のシナリオ（TSV: step, context, reading, surface）。一時的なユーザーディレクトリで実行する
    #[arg(long)]
    pub(crate) scenario: Option<PathBuf>,
    /// 取り込んだ文章の評価用 2 割から作ったケースを評価する（ユーザーディレクトリ）。--set の代わりになる
    #[arg(long)]
    pub(crate) ingest_holdout: Option<PathBuf>,

    // --- 構成を変えるフラグ（ikari と同じ意味）
    /// cost = 単語と連接のコストだけ（フェーズ0の構成 R0）、analyzer = R1（解析器と Rewriter）、doc = R2、topic = R3（vectors が要る）、
    /// full = 学習とプロファイルを含むすべて。
    /// 比較実験が主なので、既定は cost（構成はレポートに書く）
    #[arg(long, value_parser = ranker_value_parser(), default_value = RankerSet::Cost.as_str())]
    pub(crate) ranker: RankerSet,
    /// 文脈の解析をしない（BOS を固定する）。full のとき
    #[arg(long)]
    pub(crate) no_analyzer: bool,
    /// vectors.bin（省略時は <dict>/vectors.bin があれば使う）
    #[arg(long)]
    pub(crate) vectors: Option<PathBuf>,
    /// プロファイル: none = 使わない、名前 = 全ケースにそのプリセットを使う。省略時はケースの profile 列（full のとき）
    #[arg(long)]
    pub(crate) profile: Option<String>,
    /// Ranker の重みの上書き（key=value の行。zenz.alpha など）
    #[arg(long)]
    pub(crate) ranker_config: Option<PathBuf>,
    /// zenz の GGUF（feature lm が要る）
    #[arg(long)]
    pub(crate) lm: Option<PathBuf>,
    /// LM の待ち時間の上限（ms）。省略時は ZenzConfig の既定（300ms）。実際の待ちは変換1回の締め切りで決まる
    #[arg(long)]
    pub(crate) lm_budget_ms: Option<u64>,
    /// 変換1回の締め切り（ms）。先行計算が済んでいる状況（エンジンの Prefetch は最大 300ms）の再現用。
    /// 省略時は `ConvertOptions.lm_budget`（max(30ms, 6ms×読みの文字数)、上限 300ms）
    #[arg(long)]
    pub(crate) lm_deadline_ms: Option<u64>,
    /// 左文脈の目標の長さ（文字数。ADR 0046 の `carry_fill_chars`）。0 なら履歴を足さない。
    /// 既定は `CarryParams::DEFAULT`（engine と同じ。ADR 0049）。
    /// `field` 列のあるセットとシナリオにだけ効く
    #[arg(long, default_value_t = ikari_core::CarryParams::DEFAULT.fill_chars)]
    pub(crate) carry_fill: usize,
    /// 足した履歴と入力欄の左のあいだの区切り（ADR 0046 の `carry_joint`）
    /// 既定は `CarryParams::DEFAULT`
    #[arg(long, value_enum, default_value_t = CarryJointArg::from(ikari_core::CarryParams::DEFAULT.joint))]
    pub(crate) carry_joint: CarryJointArg,
    /// 学習ストアの置き場所。省略時は一時ディレクトリ（利用者のデータを評価に混ぜない）。
    /// 指定したディレクトリに user_dict.tsv があれば、ユーザー辞書として重ねる
    #[arg(long)]
    pub(crate) user_dir: Option<PathBuf>,
    /// 同梱の語彙集（IT）を重ねない（比較用。既定は重ねる。エンジンと同じ構成）
    #[arg(long)]
    pub(crate) no_packs: bool,
    /// 住所 ↔ 郵便番号の表（postal.tsv）。既定は `<dict>/postal.tsv`（なければ郵便番号の候補は出ない。ADR 0032）
    #[arg(long)]
    pub(crate) postal: Option<PathBuf>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn carry_params_default_follows_core_default() {
        let cli = Cli::try_parse_from(["ikari-eval"]).expect("no flags parse");
        assert_eq!(cli.run.carry_params(), ikari_core::CarryParams::DEFAULT);
    }

    #[test]
    fn carry_fill_zero_means_no_carry() {
        let cli = Cli::try_parse_from(["ikari-eval", "--carry-fill", "0"]).expect("parse");
        assert_eq!(cli.run.carry_params().fill_chars, 0);
    }
}
