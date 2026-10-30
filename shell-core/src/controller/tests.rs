//! 殻の判断のテスト（シナリオで書けないもの: 設定のファイル、資源がないとき、主スレッドの外の問い合わせ、英語のモードのキー）。
//! エンジンの返事に頼らない（`EngineLink` が stub でも、L-B の実装でも同じ結果になるもの）。

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use super::*;
use crate::host::{SystemClock, TransportError, TransportHandle};
use crate::types::{KeymapPreset, Modifiers, SelectedText, SurroundingText};

struct NoTransport;

impl EngineTransport for NoTransport {
    fn connect(&mut self) -> Result<TransportHandle, TransportError> {
        Err(TransportError::Unavailable)
    }
    fn request(
        &mut self,
        _: TransportHandle,
        _: &[u8],
        _: Duration,
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

fn resources() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../macos/Resources")
}

fn shell(settings: SettingsSource, resources_dir: PathBuf) -> Shell {
    let mut shell = Shell::new(
        ShellConfig {
            shell_version: "test".into(),
            settings,
            resources_dir,
        },
        Box::new(NoTransport),
        Arc::new(SystemClock::new()),
    )
    .unwrap();
    shell.set_background_mode(BackgroundMode::Inline);
    shell
}

/// テストごとの作業場所（`std::env::temp_dir` の下。終わったら消す）。
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "ikari-shell-core-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn key(kind: KeyKind) -> ShellEvent {
    ShellEvent::Key(KeyInput {
        key: kind,
        modifiers: Modifiers::NONE,
        is_repeat: false,
        secure: false,
    })
}

fn activate(shell: &mut Shell) -> Vec<ShellAction> {
    shell.dispatch(
        ShellEvent::Activate {
            bundle_id: "com.apple.TextEdit".into(),
        },
        &mut NoText,
    )
}

#[test]
fn the_first_dispatch_publishes_the_menu_and_only_changes_publish_it_again() {
    let mut shell = shell(SettingsSource::Inline(String::new()), resources());
    let actions = activate(&mut shell);
    assert!(
        actions
            .iter()
            .any(|action| matches!(action, ShellAction::ShowMenu(_)))
    );
    let actions = shell.dispatch(key(KeyKind::Space), &mut NoText);
    assert_eq!(actions, vec![ShellAction::PassThrough]);
}

#[test]
fn menu_commands_write_the_settings_file_and_the_machine_follows_when_idle() {
    let dir = TempDir::new();
    let path = dir.0.join("settings.tsv");
    std::fs::write(&path, "# 手で書いた注釈\npaging.size\t5\n").unwrap();
    let mut shell = shell(SettingsSource::File(path.clone()), resources());
    activate(&mut shell);
    let actions = shell.dispatch(
        ShellEvent::MenuCommand {
            command: MenuCommand::SelectKeymapPreset {
                preset: KeymapPreset::MsIme,
            },
        },
        &mut NoText,
    );
    assert!(
        actions
            .iter()
            .any(|action| matches!(action, ShellAction::ShowMenu(_)))
    );
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.starts_with("# 手で書いた注釈\npaging.size\t5\n"));
    assert!(text.contains("keymap.preset\tms-ime"));
    assert_eq!(shell.settings().keymap_preset, KeymapPreset::MsIme);
    // 次のキーで状態機械に反映する
    shell.dispatch(key(KeyKind::Space), &mut NoText);
    assert_eq!(
        shell.machine().unwrap().settings().keymap_preset,
        KeymapPreset::MsIme
    );
}

#[test]
fn a_hand_edit_is_picked_up_on_settings_file_changed_and_on_activate() {
    let dir = TempDir::new();
    let path = dir.0.join("settings.tsv");
    let mut shell = shell(SettingsSource::File(path.clone()), resources());
    activate(&mut shell);
    assert_eq!(shell.settings().page_size, 3);
    std::fs::write(&path, "paging.size\t7\n").unwrap();
    shell.dispatch(ShellEvent::SettingsFileChanged, &mut NoText);
    assert_eq!(shell.settings().page_size, 7);
    std::fs::write(&path, "paging.size\t2\nkeymap.preset\tatok\n").unwrap();
    activate(&mut shell);
    assert_eq!(shell.settings().page_size, 2);
}

#[test]
fn a_custom_romaji_table_next_to_the_settings_is_used() {
    let dir = TempDir::new();
    std::fs::write(dir.0.join(ROMAJI_FILE_NAME), "q\tく\n").unwrap();
    let mut shell = shell(
        SettingsSource::File(dir.0.join("settings.tsv")),
        resources(),
    );
    activate(&mut shell);
    let actions = shell.dispatch(key(KeyKind::Character { text: "q".into() }), &mut NoText);
    assert!(matches!(
        actions.first(),
        Some(ShellAction::SetMarkedText(marked)) if marked.text == "く"
    ));
}

#[test]
fn missing_resources_fall_back_to_the_function_keys_only() {
    let dir = TempDir::new();
    let mut shell = shell(SettingsSource::Inline(String::new()), dir.0.clone());
    activate(&mut shell);
    for c in ['h', 'i'] {
        shell.dispatch(
            key(KeyKind::Character {
                text: c.to_string(),
            }),
            &mut NoText,
        );
    }
    let actions = shell.dispatch(key(KeyKind::Function { number: 7 }), &mut NoText);
    assert!(actions.iter().any(|action| matches!(
        action,
        ShellAction::SetMarkedText(marked) if marked.text == "ヒ"
    )));
}

#[test]
fn a_new_bundle_gets_a_new_session_and_the_same_bundle_keeps_it() {
    let mut shell = shell(SettingsSource::Inline(String::new()), resources());
    activate(&mut shell);
    let first = shell.machine().unwrap().session().session_id;
    activate(&mut shell);
    assert_eq!(shell.machine().unwrap().session().session_id, first);
    shell.dispatch(
        ShellEvent::Activate {
            bundle_id: "com.apple.Safari".into(),
        },
        &mut NoText,
    );
    let second = shell.machine().unwrap().session();
    assert_eq!(second.session_id, first + 1);
    assert_eq!(second.client_bundle_id, "com.apple.Safari");
}

#[test]
fn excluded_apps_do_not_share_context() {
    let mut shell = shell(SettingsSource::Inline(String::new()), resources());
    shell.dispatch(
        ShellEvent::Activate {
            bundle_id: "com.apple.Terminal".into(),
        },
        &mut NoText,
    );
    assert!(!shell.machine().unwrap().session().share_context);
    activate(&mut shell);
    assert!(shell.machine().unwrap().session().share_context);
}

#[test]
fn a_key_without_activate_still_creates_a_client() {
    let mut shell = shell(SettingsSource::Inline(String::new()), resources());
    let actions = shell.dispatch(key(KeyKind::Character { text: "a".into() }), &mut NoText);
    assert!(matches!(
        actions.first(),
        Some(ShellAction::SetMarkedText(marked)) if marked.text == "あ"
    ));
}

#[test]
fn mode_requests_are_taken_once_and_a_switch_back_by_the_os_is_recorded() {
    let mut shell = shell(
        SettingsSource::Inline("keymap.mode.english\tctrl+e\n".into()),
        resources(),
    );
    activate(&mut shell);
    let actions = shell.dispatch(
        ShellEvent::Key(KeyInput {
            key: KeyKind::Character { text: "e".into() },
            modifiers: Modifiers {
                control: true,
                ..Modifiers::NONE
            },
            is_repeat: false,
            secure: false,
        }),
        &mut NoText,
    );
    assert_eq!(actions, vec![ShellAction::HideCandidates]);
    assert_eq!(shell.take_mode_request(), Some(InputMode::English));
    assert_eq!(shell.take_mode_request(), None);
    assert!(shell.input_mode_changed("unknown.mode").is_empty());
    shell.input_mode_changed(crate::english::input_mode_id::ENGLISH);
    assert_eq!(shell.input_mode(), InputMode::English);
    // 同じモードの通知は何もしない
    assert!(
        shell
            .input_mode_changed(crate::english::input_mode_id::ENGLISH)
            .is_empty()
    );
}

#[test]
fn english_mode_reads_jis_keys_by_their_ascii_legend() {
    let jis = |code, shift| KeyInput {
        key: KeyKind::Jis { code },
        modifiers: Modifiers {
            shift,
            ..Modifiers::NONE
        },
        is_repeat: false,
        secure: false,
    };
    let text = |input: KeyInput| match english_key(&input).key {
        KeyKind::Character { text } => text,
        other => format!("{other:?}"),
    };
    assert_eq!(text(jis(22, false)), "a");
    assert_eq!(text(jis(22, true)), "A");
    assert_eq!(text(jis(0, false)), "1");
    assert_eq!(text(jis(21, true)), "_");
    assert_eq!(text(jis(0, true)), "Other");
}

#[test]
fn the_panel_side_reaches_the_machine_and_hiding_resets_it() {
    let mut shell = shell(SettingsSource::Inline(String::new()), resources());
    activate(&mut shell);
    shell.panel_shown(PanelSide::Above, Some(AnchorSource::Client));
    assert_eq!(shell.machine().unwrap().panel_side(), PanelSide::Above);
    shell.panel_hidden();
    assert_eq!(shell.machine().unwrap().panel_side(), PanelSide::Below);
}

#[test]
fn background_jobs_on_threads_are_collected_by_the_poll_timer() {
    let mut shell = Shell::new(
        ShellConfig {
            shell_version: "test".into(),
            settings: SettingsSource::Inline(String::new()),
            resources_dir: resources(),
        },
        Box::new(NoTransport),
        Arc::new(SystemClock::new()),
    )
    .unwrap();
    let mut actions = activate(&mut shell);
    // 結果が来るまで、見回りのタイマーを頼み続ける（来たら頼まない）
    for _ in 0..200 {
        let scheduled = actions.iter().any(|action| {
            matches!(action, ShellAction::ScheduleTimer { id, .. } if *id == BACKGROUND_POLL_TIMER)
        });
        if !scheduled && !shell.background.has_outstanding() {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
        actions = shell.dispatch(
            ShellEvent::Timer {
                id: BACKGROUND_POLL_TIMER,
            },
            &mut NoText,
        );
    }
    panic!("background jobs never finished");
}

#[test]
fn anchor_memory_remembers_per_app() {
    let mut memory = AnchorMemory::new();
    let rect = crate::panel::Rect {
        origin: crate::panel::Point { x: 10.0, y: 20.0 },
        size: crate::panel::Size {
            width: 1.0,
            height: 17.0,
        },
    };
    assert_eq!(memory.last_known("a"), None);
    memory.remember(rect, "a");
    assert_eq!(memory.last_known("a"), Some(rect));
    assert_eq!(memory.last_known("b"), None);
}
