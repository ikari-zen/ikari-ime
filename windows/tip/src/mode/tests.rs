use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use ikari_shell_core::{
    EngineTransport, KeyInput, Modifiers, SelectedText, SettingsSource, ShellConfig,
    SurroundingText, SystemClock, TextAccess, TransportError, TransportHandle,
};

use super::*;

struct NoTransport;

impl EngineTransport for NoTransport {
    fn connect(&self) -> Result<TransportHandle, TransportError> {
        Err(TransportError::Unavailable)
    }
    fn request(
        &self,
        _: TransportHandle,
        _: &[u8],
        _: Duration,
    ) -> Result<Vec<u8>, TransportError> {
        Err(TransportError::Unavailable)
    }
    fn post(&self, _: TransportHandle, _: &[u8]) {}
    fn close(&self, _: TransportHandle) {}
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

fn shell(settings: &str) -> Shell {
    let resources = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../shell-core/resources");
    let mut shell = Shell::new(
        ShellConfig {
            shell_version: "test".into(),
            settings: SettingsSource::Inline(settings.into()),
            resources_dir: resources,
            engine_owner: false,
        },
        Box::new(NoTransport),
        Arc::new(SystemClock::new()),
    )
    .unwrap();
    shell.dispatch(
        ShellEvent::Activate {
            bundle_id: "notepad.exe".into(),
            secure: false,
        },
        &mut NoText,
    );
    shell
}

fn key(kind: KeyKind, control: bool) -> ShellEvent {
    ShellEvent::Key(KeyInput {
        key: kind,
        modifiers: Modifiers {
            control,
            ..Modifiers::NONE
        },
        is_repeat: false,
        secure: false,
        layout_text: None,
    })
}

fn character(text: &str, control: bool) -> ShellEvent {
    key(KeyKind::Character { text: text.into() }, control)
}

/// 1 打鍵ぶんの `dispatch` と後始末（`com/service.rs` と同じ順）
fn press(shell: &mut Shell, event: ShellEvent) -> Vec<ShellAction> {
    let fallback = requested_by_event(&event);
    let mut actions = shell.dispatch(event, &mut NoText);
    actions.extend(settle(shell, fallback));
    actions
}

#[test]
fn values_round_trip_through_both_compartments() {
    for mode in [InputMode::Japanese, InputMode::English] {
        let values = values_of(mode);
        assert_eq!(from_open(values.open), mode);
        assert_eq!(from_conversion(values.conversion), mode);
    }
}

#[test]
fn values_match_the_documented_modes() {
    let japanese = values_of(InputMode::Japanese);
    assert!(japanese.open);
    assert_eq!(japanese.conversion, 0x19);
    let english = values_of(InputMode::English);
    assert!(!english.open);
    assert_eq!(english.conversion, 0);
}

#[test]
fn conversion_reads_only_the_native_bit() {
    // 日本語でもカタカナ・半角の組み合わせは日本語のまま
    assert_eq!(
        from_conversion(conversion::NATIVE | 0x2),
        InputMode::Japanese
    );
    assert_eq!(from_conversion(conversion::NATIVE), InputMode::Japanese);
    // 全角・ローマ字だけで NATIVE がなければ英数
    assert_eq!(
        from_conversion(conversion::FULLSHAPE | conversion::ROMAN),
        InputMode::English
    );
}

#[test]
fn labels_differ_per_mode() {
    assert_eq!(label(InputMode::Japanese), "あ");
    assert_eq!(label(InputMode::English), "A");
    assert_ne!(tooltip(InputMode::Japanese), tooltip(InputMode::English));
}

#[test]
fn eisu_and_kana_keys_pick_the_mode_like_the_os_does() {
    assert_eq!(
        requested_by_event(&key(KeyKind::Eisu, false)),
        Some(InputMode::English)
    );
    assert_eq!(
        requested_by_event(&key(KeyKind::Kana, false)),
        Some(InputMode::Japanese)
    );
    assert_eq!(requested_by_event(&character("a", false)), None);
    assert_eq!(requested_by_event(&ShellEvent::Deactivate), None);
}

#[test]
fn tracker_emits_only_on_change_and_after_reset() {
    let mut tracker = ModeTracker::new();
    let show = |mode| Some(TsfOp::ShowInputMode { mode });
    assert_eq!(tracker.sync(InputMode::Japanese), show(InputMode::Japanese));
    assert_eq!(tracker.sync(InputMode::Japanese), None);
    assert_eq!(tracker.sync(InputMode::English), show(InputMode::English));
    assert_eq!(tracker.sync(InputMode::English), None);
    tracker.reset();
    assert_eq!(tracker.sync(InputMode::English), show(InputMode::English));
}

#[test]
fn a_mode_key_switches_to_english_and_keys_then_pass_through() {
    let mut shell = shell("keymap.mode.toggle\tctrl+e\n");
    assert_eq!(shell.input_mode(), InputMode::Japanese);
    press(&mut shell, character("e", true));
    assert_eq!(shell.input_mode(), InputMode::English);
    // 英語のモード: 打鍵はアプリにそのまま渡る（キーを食べない）
    let actions = press(&mut shell, character("a", false));
    let ops = crate::ops::OpsTranslator::new().translate_all(&actions);
    assert!(!crate::ops::key_consumed(&ops), "{actions:?}");
    // もう一度で日本語に戻る
    press(&mut shell, character("e", true));
    assert_eq!(shell.input_mode(), InputMode::Japanese);
}

#[test]
fn eisu_and_kana_keys_switch_the_mode_and_are_consumed() {
    let mut shell = shell("");
    let actions = press(&mut shell, key(KeyKind::Eisu, false));
    assert_eq!(shell.input_mode(), InputMode::English);
    let ops = crate::ops::OpsTranslator::new().translate_all(&actions);
    assert!(crate::ops::key_consumed(&ops), "{actions:?}");
    press(&mut shell, key(KeyKind::Kana, false));
    assert_eq!(shell.input_mode(), InputMode::Japanese);
    // すでに日本語のときのかなキーは何も変えない
    press(&mut shell, key(KeyKind::Kana, false));
    assert_eq!(shell.input_mode(), InputMode::Japanese);
}

#[test]
fn an_os_side_change_reaches_the_shell() {
    let mut shell = shell("");
    shell.dispatch(
        ShellEvent::InputModeChanged {
            mode_id: InputMode::English.mode_id().into(),
            secure: false,
        },
        &mut NoText,
    );
    assert_eq!(shell.input_mode(), InputMode::English);
}
