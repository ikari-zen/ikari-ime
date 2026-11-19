//! Windows の子の、絞った起動（ADR 0072 §3 の段 1 の作り。ADR 0076 §3.2）。
//!
//! - 子は止めた状態（`CREATE_SUSPENDED`）で作り、Job Object に入れてから動かす（入る前に動く隙を作らない）。
//!   Job: 持ち主のハンドルが閉じたら中のプロセスが終わる。プロセスの数の上限。UI の制限（どの子にも必ずかける）。
//! - 継承するハンドルは、渡す標準入出力と、呼ぶ側が明示したもの（`Launch::inherit`。取得の子の書く先。ADR 0078）だけ
//!   （`PROC_THREAD_ATTRIBUTE_HANDLE_LIST`）。渡さない標準入出力は null。渡すハンドルの継承の印は、起動の直前に付け、
//!   起動のあとに外す。
//! - 整合性を低くできる（トークンを複製して `WinLowLabelSid`。`CreateProcessAsUserW`）。
//! - 窓を出さない（`CREATE_NO_WINDOW`）。実行ファイルは絶対パスで明示し、コマンドラインはすべて引用符で囲む
//!   （`cmdline`）。カレントディレクトリは実行ファイルのフォルダ。シェルを通さない。
//! - どれか 1 つでもかけられなければ、子を止めて失敗を返す。
//!
//! 使う側: `ikari-data` の `transfer` の起動（`fetch/src/spawn_windows.rs`）、設定画面の核の `ikari-data` の起動
//! （`windows/settings/src/fetch_os.rs`）。LM の子（`lm/src/host/os/windows.rs`）は、段 2（AppContainer）の
//! 統合のあとで移す（ADR 0076 §17）。

pub mod cmdline;

#[cfg(windows)]
mod os;
#[cfg(windows)]
pub use os::{Confined, Launch, Stdio, environment_block, spawn};

/// 子の整合性
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Integrity {
    /// 親と同じ
    Inherit,
    /// 低（`WinLowLabelSid`。利用者のファイルに書けない）
    Low,
}

/// 子を入れる Job Object の制限。UI の制限（クリップボード、ほかのプロセスの窓、デスクトップ、画面とシステムの
/// 設定、グローバルなアトム、サインアウト）は、どの子にも必ずかける（ここで絞る子は、どれも画面を持たない）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JobLimits {
    /// Job の中のプロセスの数の上限（子そのものを含む）。1 なら、子はさらにプロセスを作れない
    pub max_processes: u32,
}
