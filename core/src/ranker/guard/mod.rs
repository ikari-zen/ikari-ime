//! Guard: 各 Ranker の差分の合計を見て score を直す部品。Ranker ではない（phase2-q §2.1）。
//! 差分を数えて呼ぶのは [`CompositeRanker`](super::CompositeRanker) だけなので、crate の外には出さない。

pub(super) mod grammar;
pub(super) mod learning;
