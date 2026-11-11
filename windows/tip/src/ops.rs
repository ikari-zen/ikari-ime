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

    /// 動作 1 つを操作の列にする。何も要らない動作は空の列。
    ///
    /// `match` は `_` を持たない。`ShellAction` に形が足されたら、ここでコンパイルが落ちる
    pub fn translate(&mut self, action: &ShellAction) -> Vec<TsfOp> {
        match action {
            ShellAction::PassThrough => vec![TsfOp::PassThrough],
            // 食べるだけ（表示は変えない）
            ShellAction::Consume => Vec::new(),
            // 空の文字列は合成を消す指示（OS の合成を終える）
            ShellAction::SetMarkedText(marked) if marked.text.is_empty() => {
                if self.composing {
                    self.composing = false;
                    vec![TsfOp::EndComposition]
                } else {
                    Vec::new()
                }
            }
            ShellAction::SetMarkedText(marked) => {
                let mut ops = Vec::with_capacity(2);
                if !self.composing {
                    self.composing = true;
                    ops.push(TsfOp::StartComposition);
                }
                ops.push(TsfOp::UpdateComposition(marked.clone()));
                ops
            }
            ShellAction::Commit { text } => {
                // 合成がなくても、確定は 1 つの操作で行える（COM の層が合成を作ってすぐ確定する）
                self.composing = false;
                vec![TsfOp::CommitComposition { text: text.clone() }]
            }
            ShellAction::ShowCandidates(model) => vec![TsfOp::ShowCandidates(model.clone())],
            ShellAction::MoveSelection(model) => vec![TsfOp::MoveCandidates(model.clone())],
            ShellAction::HideCandidates => vec![TsfOp::HideCandidates],
            ShellAction::DeleteText { range } => vec![TsfOp::DeleteText { range: *range }],
            ShellAction::Restore { text, replacing } => vec![TsfOp::RestoreText {
                text: text.clone(),
                replacing: *replacing,
            }],
            ShellAction::ResolveStrayMarkedText => {
                // 取り残された合成があれば確定される。結果は COM の層が見るので、こちらの記憶は捨てる
                self.composing = false;
                vec![TsfOp::ResolveStrayComposition]
            }
            // エンジンへの Prefetch は shell-core が送る。OS 側で要る操作はない
            ShellAction::Prefetch { .. } => Vec::new(),
            // メニューの中身は OS 側が持っておくもので、TSF の側で写す操作はない（言語バーのメニューは W4 で扱う）
            ShellAction::ShowMenu(_) => Vec::new(),
            ShellAction::ScheduleTimer { id, after_ms } => vec![TsfOp::ScheduleTimer {
                id: *id,
                after_ms: *after_ms,
            }],
            ShellAction::CancelTimer { id } => vec![TsfOp::CancelTimer { id: *id }],
            ShellAction::BridgeReply { conn, frame } => vec![TsfOp::BridgeReply {
                conn: *conn,
                frame: frame.clone(),
            }],
            ShellAction::OpenSettings { tab } => vec![TsfOp::OpenSettings { tab: *tab }],
            ShellAction::SelectMode { mode_id } => vec![TsfOp::SelectMode {
                mode_id: mode_id.clone(),
            }],
            ShellAction::BridgeClose { conn } => vec![TsfOp::BridgeClose { conn: *conn }],
            ShellAction::ShowLearningQuarantineNotice => vec![TsfOp::ShowLearningQuarantineNotice],
        }
    }

    /// 動作の列を順に写す
    pub fn translate_all(&mut self, actions: &[ShellAction]) -> Vec<TsfOp> {
        actions
            .iter()
            .flat_map(|action| self.translate(action))
            .collect()
    }

    /// 合成の記憶を捨てる（Deactivate、アプリの切り替え、OS が合成を終わらせたとき）
    pub fn reset(&mut self) {
        self.composing = false;
    }
}

/// 列がキーを食べたか（`PassThrough` を含まなければ食べた。`ResolveStrayComposition` は COM の層が結果で決める）
pub fn key_consumed(ops: &[TsfOp]) -> bool {
    !ops.contains(&TsfOp::PassThrough)
}

#[cfg(test)]
mod tests;
