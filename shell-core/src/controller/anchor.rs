//! 同じアプリで最後に取れた正常なキャレットの矩形（候補ウィンドウの位置のフォールバック②。今の `AnchorMemory`）。
//!
//! 矩形は OS の座標なので、OS の入口が `panel::resolve_anchor` の前後で使う（`dispatch` は通らない）。

use std::collections::HashMap;

use crate::panel::Rect;

/// アプリ（bundle ID）ごとの、最後に取れた正常なキャレットの矩形。
#[derive(Debug, Clone, Default)]
pub struct AnchorMemory {
    rects: HashMap<String, Rect>,
}

impl AnchorMemory {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn last_known(&self, bundle_id: &str) -> Option<Rect> {
        self.rects.get(bundle_id).copied()
    }

    /// キャレットから取れたとき（`AnchorSource::Client`）だけ覚える。
    pub fn remember(&mut self, rect: Rect, bundle_id: &str) {
        self.rects.insert(bundle_id.to_string(), rect);
    }
}
