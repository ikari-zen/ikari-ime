//! 純粋な部分と COM の層の間の契約（ADR 0040 §5。W2-0）。
//!
//! COM の層は、`ops::OpsTranslator` が返した列を**順に**実行する。`ShellAction` の各形が、ここのどれか（または何も要らない）に対応する。

use ikari_shell_core::{CandidatePanelModel, ConnId, MarkedText, SettingsTab, TimerId, Utf16Range};

/// COM の層が実行する 1 つの操作。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TsfOp {
    /// このキーを IME が処理しない（アプリにそのまま渡す）。列に含まれていれば、キーは食べない
    PassThrough,
    /// 合成（下線つきの未確定文字列）を始める。すでに合成中なら出ない
    StartComposition,
    /// 合成中の文字列を置き換える（強調、文節、置き換える範囲は `MarkedText` のとおり）
    UpdateComposition(MarkedText),
    /// 合成中の文字列を `text` に置き換えて確定し、合成を終える
    CommitComposition { text: String },
    /// 合成を（文字列を残さず）終える
    EndComposition,
    /// 取り残された合成があれば確定する。なければキーはアプリに渡す（キーを食べたかは、この結果で決まる）
    ResolveStrayComposition,
    /// 候補窓を出す
    ShowCandidates(CandidatePanelModel),
    /// 候補窓の選択を動かす
    MoveCandidates(CandidatePanelModel),
    /// 候補窓を閉じる
    HideCandidates,
    /// 確定済みの文字列の範囲を消す（合成がないときだけ）
    DeleteText { range: Utf16Range },
    /// 元の文字列を、`replacing` の範囲を置き換える 1 回の挿入で書き戻す
    RestoreText { text: String, replacing: Utf16Range },
    /// タイマーを予約する（時間が来たら `ShellEvent::Timer` で返す）
    ScheduleTimer { id: TimerId, after_ms: u32 },
    /// タイマーを取り消す
    CancelTimer { id: TimerId },
    /// 入力モードを選んでもらう（値はモードの ID）
    SelectMode { mode_id: String },
    /// 設定画面を開く
    OpenSettings { tab: Option<SettingsTab> },
    /// 設定画面へ枠を返す
    BridgeReply { conn: ConnId, frame: Vec<u8> },
    /// 設定画面との接続を閉じる
    BridgeClose { conn: ConnId },
    /// 不正な学習ファイルを退避したことを知らせる（いまの入力処理が終わってから）
    ShowLearningQuarantineNotice,
}
