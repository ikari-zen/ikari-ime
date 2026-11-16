//! TIP が有効になるときの動き方を、入った先のプロセスの種類から決める（ADR 0067 §5）。**OS に依存しない**。
//!
//! COM の層（`com::service`、`com::host`）は、この表のとおりに登録と `Shell` の作成を行うだけで、判断を持たない。

use ikari_shell_core::english::InputMode;

use crate::process_kind::ProcessKind;

/// 有効になるときの動き方
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Activation {
    /// `Shell` を作る（設定を読む、エンジンにつなぐ、計測の印を読む）
    pub shell: bool,
    /// OS の通知（キー、文書のフォーカス、入力モードの変更）を受けて `Shell` に渡す。受けなければキーはすべてアプリに渡る
    pub events: bool,
    /// 有効になったときに表示する入力モード
    pub initial_mode: InputMode,
}

impl Activation {
    /// 通常どおり
    const NORMAL: Self = Self {
        shell: true,
        events: true,
        initial_mode: InputMode::Japanese,
    };

    /// 素通し: `Shell` を作らず、通知を受けず、表示は英語のモードと同じにする（変換できないことが見える）
    const PASSTHROUGH: Self = Self {
        shell: false,
        events: false,
        initial_mode: InputMode::English,
    };

    /// 種類から動き方を決める（`Normal` だけが通常どおり。ほかはすべて素通し）
    pub fn for_kind(kind: ProcessKind) -> Self {
        if kind.is_passthrough() {
            Self::PASSTHROUGH
        } else {
            Self::NORMAL
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_runs_the_shell() {
        let a = Activation::for_kind(ProcessKind::Normal);
        assert!(a.shell);
        assert!(a.events);
        assert_eq!(a.initial_mode, InputMode::Japanese);
    }

    #[test]
    fn other_kinds_pass_through() {
        for kind in [
            ProcessKind::Elevated,
            ProcessKind::Sandboxed,
            ProcessKind::Unknown,
        ] {
            let a = Activation::for_kind(kind);
            assert!(!a.shell, "{kind:?}");
            assert!(!a.events, "{kind:?}");
            assert_eq!(a.initial_mode, InputMode::English, "{kind:?}");
        }
    }
}
