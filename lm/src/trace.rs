//! スレッドごとの LM 計測の記録（ADR 0061 §4.1）。
//!
//! 契約（レーン 0）。型と関数の宣言だけで、中身はスタブ（`begin` は何もせず、`take` は既定値を返す）。
//! 実装はレーン 1a。

use crate::scorer::LmDevice;

/// 呼び出したスレッドの、`rank` の記録（ADR 0061 §4.1）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LmTrace {
    /// 採点の対象があり締め切りのある `rank` の回数（`LmStats.calls` と同じ数え方）
    pub calls: u32,
    /// そのうち間に合わなかった回数
    pub timeouts: u32,
    /// 遅れて届いた結果を使った回数
    pub late_hits: u32,
    /// 返事を待った時間の合計（us）
    pub wait_micros: u64,
    /// 受け取った返事の、ワーカーでの推論の時間の合計（us）
    pub infer_micros: u64,
    /// 取り出したときの装置。モデルが読まれていなければ `Unknown`
    pub device: LmDevice,
}

/// このスレッドの記録を 0 にする（スタブ: 何もしない）。
pub fn begin() {}

/// このスレッドの記録を取り出して 0 にする（スタブ: 既定値を返す）。
pub fn take() -> LmTrace {
    LmTrace::default()
}
