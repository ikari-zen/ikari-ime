//! 自分のプロセスのトークンから、種類の判定に使う事実を取る（ADR 0067 §5。判定は `process_kind::classify`）。
//!
//! **契約のスタブ**。中身（`OpenProcessToken`、`GetTokenInformation` の `TokenIntegrityLevel` と
//! `TokenIsAppContainer`）は、プロセスの種類のレーン（A-1）が実装し、`ActivateEx` で `Shell` を作る前に呼ぶ。
//! 取れなかった項目は `None` にする（判定は素通しに倒れる）。

use crate::process_kind::ProcessFacts;

/// 自分のプロセスの事実。今はスタブで、何も取れなかったものとして返す。
// TODO(A-1): トークンを読む。呼ぶ側（`service`）ができるまで使われない
#[allow(dead_code)]
pub(super) fn current_process_facts() -> ProcessFacts {
    ProcessFacts::UNKNOWN
}
