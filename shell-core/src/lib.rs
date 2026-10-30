//! ikari-shell-core: 殻の OS に依存しない処理（ADR 0041）。macOS（Swift の薄い入口から `shell-ffi` 経由）と
//! Windows（TIP から crate として直接）で共有する。
//!
//! 境界は 1 本の口: OS のイベント（`ShellEvent`）→ `Shell::dispatch` → 動作の列（`ShellAction`）。
//! 周辺テキストと選択（`TextAccess`）、エンジンへのバイト列の運び（`EngineTransport`）、時計（`Clock`）はホストの口。
//!
//! # 持ち主（ADR 0041 §7.2）
//! - M1-a で固めた（レーンは変えない）: `lib.rs`、`types.rs`、`host.rs`、`scenario.rs`、各モジュールの公開のシグネチャ
//!   （`controller::Shell`、`engine::EngineLink`、`settings::{validate, rewrite}`）
//! - L-A1: `keymap`、`small_letters`、`menu`、`panel`、`timeouts`、`english`、`shortcode`、`postal`、`diag`
//! - L-A2: `machine`、`controller`
//! - L-B: `engine`、`bridge`
//! - L-C: `settings`
//!
//! # 約束（ADR 0041 §4.2）
//! - 位置と長さは UTF-16 の単位（`Utf16Len`、`Utf16Range`）。
//! - panic しない。`unwrap`・`expect` は clippy で禁じる（テストは除く）。

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

pub mod bridge;
pub mod controller;
pub mod diag;
pub mod engine;
pub mod english;
pub mod host;
pub mod keymap;
pub mod machine;
pub mod menu;
pub mod panel;
pub mod postal;
pub mod scenario;
pub mod settings;
pub mod shortcode;
pub mod small_letters;
pub mod timeouts;
pub mod types;

pub use controller::{SettingsSource, Shell, ShellConfig, ShellError};
pub use engine::EngineLink;
pub use host::{
    Clock, DownKind, EngineTransport, SystemClock, TextAccess, TransportError, TransportHandle,
};
pub use settings::{SettingsChanges, SettingsEntry, SettingsError, SettingsValidation};
pub use types::*;

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send<T: Send>() {}
    fn assert_send_sync<T: Send + Sync>() {}

    /// ADR 0041 §4.2: `Shell` は `Send`。`EngineLink` は落ちた通知のために `Send + Sync`
    #[test]
    fn threading_contract() {
        assert_send::<Shell>();
        assert_send_sync::<EngineLink>();
    }

    #[test]
    fn utf16_len_counts_surrogate_pairs_as_two() {
        assert_eq!(Utf16Len::of("あ"), Utf16Len(1));
        assert_eq!(Utf16Len::of("😀"), Utf16Len(2));
        assert_eq!(Utf16Len::of("ab"), Utf16Len(2));
    }

    #[test]
    fn actions_round_trip_through_the_scenario_json_shape() {
        let actions = vec![
            ShellAction::PassThrough,
            ShellAction::SetMarkedText(MarkedText {
                highlighted_length: Utf16Len(3),
                ..MarkedText::plain("ひんし")
            }),
            ShellAction::Commit {
                text: "品詞".into(),
            },
            ShellAction::OpenSettings { tab: None },
        ];
        let json = serde_json::to_string(&actions).unwrap();
        assert_eq!(
            json,
            r#"[{"type":"pass_through"},{"type":"set_marked_text","text":"ひんし","highlighted_length":3},{"type":"commit","text":"品詞"},{"type":"open_settings"}]"#
        );
        let back: Vec<ShellAction> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, actions);
    }

    #[test]
    fn unknown_fields_are_rejected_so_typos_in_scenarios_fail() {
        let typo = r#"{"type":"set_marked_text","text":"a","highlighted_lenght":1}"#;
        assert!(serde_json::from_str::<ShellAction>(typo).is_err());
    }

    #[test]
    fn stubs_do_not_panic() {
        struct NoTransport;
        impl EngineTransport for NoTransport {
            fn connect(&mut self) -> Result<TransportHandle, TransportError> {
                Err(TransportError::Unavailable)
            }
            fn request(
                &mut self,
                _: TransportHandle,
                _: &[u8],
                _: std::time::Duration,
            ) -> Result<Vec<u8>, TransportError> {
                Err(TransportError::Unavailable)
            }
            fn post(&mut self, _: TransportHandle, _: &[u8]) {}
            fn close(&mut self, _: TransportHandle) {}
            fn pid(&self, _: TransportHandle) -> Option<i32> {
                None
            }
        }
        struct NoText;
        impl TextAccess for NoText {
            fn surrounding(&mut self) -> Option<SurroundingText> {
                None
            }
            fn selection(&mut self) -> Option<SelectedText> {
                None
            }
        }
        let config = ShellConfig {
            shell_version: "test".into(),
            settings: SettingsSource::Inline(String::new()),
            resources_dir: std::path::PathBuf::new(),
        };
        let mut shell = Shell::new(
            config,
            Box::new(NoTransport),
            std::sync::Arc::new(SystemClock::new()),
        )
        .unwrap();
        let event = ShellEvent::Key(KeyInput {
            key: KeyKind::Space,
            modifiers: Modifiers::NONE,
            is_repeat: false,
            secure: false,
        });
        let _ = shell.dispatch(event, &mut NoText);
        shell
            .engine()
            .notify_down(TransportHandle(1), DownKind::Interrupted);
        // L-C が実装したので、settings は Ok を返す（stub の間は Err だった）
        assert!(settings::validate("").is_ok());
        assert!(settings::rewrite("", &SettingsChanges::default()).is_ok());
    }
}
