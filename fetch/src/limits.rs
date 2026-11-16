//! 通信と取得の値（ADR 0076 §4.4、§8.3、§15 Q3）。
//!
//! **どれも仮置き。計測していない**（ユーザー決定 2026-11-17: 仮置きのまま実装し、VM の確認で見直す）。
//! 値を変えるときは、計測してから（`docs/user-feedback.md` の「値は計測で決める」）。

use std::time::Duration;

/// 追ってよい転送の回数（計測なし）
pub const MAX_REDIRECTS: u32 = 5;
/// 接続と名前の解決の締め切り（計測なし）
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// 無通信の締め切り。読むたびに数え直す（計測なし）。全体の締め切りは置かない（大きいモデルが遅い回線で落ちないため）
pub const READ_TIMEOUT: Duration = Duration::from_secs(60);
/// 版の確認で読む本文の上限（バイト。計測なし。本文は読んで捨てる。実装は D-2d）
pub const VERSION_CHECK_BODY_LIMIT: u64 = 64 * 1024;
/// ディスクの空きの余裕（バイト。計測なし）
pub const DISK_MARGIN: u64 = 256 * 1024 * 1024;
/// 自由指定のモデルの大きさの上限（バイト）。ADR 0060 §4 の値は未定なので、仮に 4GiB（計測なし。未定の値に合わせて直す）
pub const MODEL_MAX_SIZE: u64 = 4 * 1024 * 1024 * 1024;
/// 進み具合の行を出す最小の間隔（行が多すぎて核が詰まらないように。計測なし）
pub const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);
/// 要求の User-Agent（版を付けない。ADR 0076 §4.4）
pub const USER_AGENT: &str = "ikari-data";
