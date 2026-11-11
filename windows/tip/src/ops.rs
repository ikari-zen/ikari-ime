//! `ShellAction` → `TsfOp` の変換（純粋）。

use ikari_shell_core::ShellAction;

use crate::tsf_op::TsfOp;

/// 動作を `TsfOp` に写す。合成の開始と更新を分けるため、合成中かどうかを覚える（動作の列を順に渡す）。
#[derive(Debug, Clone, Default)]
pub struct OpsTranslator {
    composing: bool,
}

impl OpsTranslator {
    pub fn new() -> Self {
        Self::default()
    }

    /// いま合成中か（`translate` の結果から追う）
    pub fn is_composing(&self) -> bool {
        self.composing
    }

    /// 動作 1 つを操作の列にする。何も要らない動作は空の列
    pub fn translate(&mut self, _action: &ShellAction) -> Vec<TsfOp> {
        todo!("W2-a")
    }

    /// 動作の列を順に写す
    pub fn translate_all(&mut self, _actions: &[ShellAction]) -> Vec<TsfOp> {
        todo!("W2-a")
    }

    /// 合成の記憶を捨てる（Deactivate、アプリの切り替え、OS が合成を終わらせたとき）
    pub fn reset(&mut self) {
        self.composing = false;
    }
}

/// 列がキーを食べたか（`PassThrough` を含まなければ食べた。`ResolveStrayComposition` は COM の層が結果で決める）
pub fn key_consumed(_ops: &[TsfOp]) -> bool {
    todo!("W2-a")
}
