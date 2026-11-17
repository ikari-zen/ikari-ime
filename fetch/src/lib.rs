//! 取得する部品 `ikari-data`（ADR 0076。モデルの取得。ADR 0071 §3）。
//!
//! **通信してよいのはこの crate だけ**（依存、ソース、OS の規則、実機の記録の 4 つで言える。ADR 0076 §14）。
//! 別の実行ファイル（`ikari-data`）で、1 回の起動で 1 つの仕事をして終わる。常駐しない。
//!
//! - `args`: コマンドの引数。`url`: 取得先の URL を厳しく読む。`limits`: 仮置きの値
//! - `transfer`: 通信（`ureq`。設定は 1 か所）。`child`、`spawn_windows`: Windows の低い整合性の子
//! - `workspace`: 作業の場所、錠、片づけ、ディスクの見積もり
//! - `job`: 流れ（落とす → 信じずに数え直して写す → rename）。`run`: 入口
//!
//! 判断の置き場所: すべて Rust（UI の C#・Swift には判断を置かない）。

pub mod args;
pub mod child;
pub mod error;
pub mod job;
pub mod limits;
mod nofollow;
pub mod run;
#[cfg(windows)]
pub mod spawn_windows;
pub mod transfer;
pub mod url;
pub mod workspace;

pub use error::FetchError;
pub use run::run;
