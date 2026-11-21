//! 取得の核は `ikari-shell-core` の `settings::fetch` に移した（ADR 0079 §3.6）。macOS と共有する。
//! この crate の `fetch_os`・`fetch_abi` は、ここから同じ名前で使う。

pub use ikari_shell_core::settings::fetch::*;
