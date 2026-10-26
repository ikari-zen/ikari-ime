//! Ranker の名前。`Ranker::name()`、設定キーの接頭辞（`<name>.<key>`）、`CompositeRanker` の分類で共有する。

pub const COST: &str = "cost";
pub const LEFT_CONTEXT: &str = "left_context";
pub const KANA_STYLE: &str = "kana_style";
pub const DOC_CACHE: &str = "doc_cache";
pub const LEARNING: &str = "learning";
pub const PROFILE: &str = "profile";
pub const TOPIC: &str = "topic";
pub const COMPOSITE: &str = "composite";
