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
        layout_text: None,
    })
}

fn activate(shell: &mut Shell) -> Vec<ShellAction> {
    shell.dispatch(
        ShellEvent::Activate {
            bundle_id: "com.apple.TextEdit".into(),
            secure: false,
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
            secure: false,
        },
        &mut NoText,
    );
    let second = shell.machine().unwrap().session();
    assert_eq!(second.session_id, first + 1);
    assert_eq!(second.client_bundle_id, "com.apple.Safari");
}

fn activate_app(shell: &mut Shell, bundle_id: &str) -> u64 {
    shell.dispatch(
        ShellEvent::Activate {
            bundle_id: bundle_id.into(),
            secure: false,
        },
        &mut NoText,
    );
    shell.machine().unwrap().session().session_id
}

#[test]
fn going_back_to_an_app_reuses_its_session_id() {
    // ADR 0046 A1: A → B → A で A の ID が戻る
    let mut shell = shell(SettingsSource::Inline(String::new()), resources());
    let a = activate_app(&mut shell, "com.example.A");
    let b = activate_app(&mut shell, "com.example.B");
    assert_ne!(a, b);
    assert_eq!(activate_app(&mut shell, "com.example.A"), a);
    assert_eq!(activate_app(&mut shell, "com.example.B"), b);
    // 覚えていない新しいアプリは、これまでと重ならない ID
    let c = activate_app(&mut shell, "com.example.C");
    assert!(c != a && c != b);
}

#[test]
fn an_unknown_bundle_id_is_never_reused() {
    let mut shell = shell(SettingsSource::Inline(String::new()), resources());
    let first = activate_app(&mut shell, "");
    activate_app(&mut shell, "com.example.A");
    assert_ne!(activate_app(&mut shell, ""), first);
}

#[test]
fn the_oldest_app_is_forgotten_after_sixty_four_apps() {
    let mut shell = shell(SettingsSource::Inline(String::new()), resources());
    let first = activate_app(&mut shell, "com.example.app0");
    for n in 1..SESSION_ID_MEMORY_MAX {
        activate_app(&mut shell, &format!("com.example.app{n}"));
    }
    // 64 件までは覚えている
    assert_eq!(activate_app(&mut shell, "com.example.app0"), first);
    // 65 個目で、最も長く使っていないもの（app1）を忘れる。app0 は今使ったので残る
    activate_app(&mut shell, "com.example.extra");
    assert_eq!(activate_app(&mut shell, "com.example.app0"), first);
    let forgotten = shell
        .session_ids
        .iter()
        .all(|(b, _)| b != "com.example.app1");
    assert!(forgotten);
    assert_eq!(shell.session_ids.len(), SESSION_ID_MEMORY_MAX);
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
            layout_text: None,
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
        layout_text: None,
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

// ---------------------------------------------------------------- L-E で足した口

mod lane_e {
    use prost::Message;

    use super::*;
    use crate::bridge::proto::{
        BridgeErrorCode, BridgeHello, Frame, GetSettings, SettingEntry, UpdateSettings, event,
        frame::Body,
    };
    use crate::panel::{AnchorSource, PanelSide};

    fn frame(request_id: u64, body: Body) -> Vec<u8> {
        Frame {
            request_id,
            body: Some(body),
        }
        .encode_to_vec()
    }

    fn hello(version: u32) -> Vec<u8> {
        frame(
            1,
            Body::Hello(BridgeHello {
                bridge_version: version,
                app_version: "t".into(),
            }),
        )
    }

    fn send(shell: &mut Shell, conn: u64, bytes: Vec<u8>) -> Vec<ShellAction> {
        shell.dispatch(
            ShellEvent::BridgeFrame {
                conn: ConnId(conn),
                frame: bytes,
            },
            &mut NoText,
        )
    }

    /// 動作の列から、設定アプリへの返事の枠だけを取り出す
    fn replies(actions: &[ShellAction]) -> Vec<(u64, Frame)> {
        actions
            .iter()
            .filter_map(|action| match action {
                ShellAction::BridgeReply { conn, frame } => {
                    Some((conn.0, Frame::decode(frame.as_slice()).unwrap()))
                }
                _ => None,
            })
            .collect()
    }

    fn error_code(frame: &Frame) -> Option<(i32, String)> {
        match &frame.body {
            Some(Body::Error(error)) => Some((error.code, error.key.clone())),
            _ => None,
        }
    }

    #[test]
    fn bridge_handshake_then_settings_get_and_update_with_notification() {
        let dir = TempDir::new();
        let path = dir.0.join("settings.tsv");
        std::fs::write(&path, "# note\nengine.lm\ton\n").unwrap();
        let mut shell = shell(SettingsSource::File(path.clone()), resources());

        let actions = send(&mut shell, 5, hello(1));
        let got = replies(&actions);
        assert_eq!(got.len(), 1);
        assert!(matches!(got[0].1.body, Some(Body::HelloReply(_))));

        let actions = send(&mut shell, 5, frame(7, Body::GetSettings(GetSettings {})));
        let got = replies(&actions);
        let Some(Body::Settings(snapshot)) = &got[0].1.body else {
            panic!("no snapshot: {got:?}");
        };
        assert_eq!(got[0].1.request_id, 7);
        assert!(
            snapshot
                .effective
                .iter()
                .any(|e| e.key == "paging.size" && e.value == "3")
        );

        let update = UpdateSettings {
            set: vec![SettingEntry {
                key: "paging.size".into(),
                value: "4".into(),
            }],
            reset: Vec::new(),
        };
        let actions = send(&mut shell, 5, frame(8, Body::UpdateSettings(update)));
        let got = replies(&actions);
        assert_eq!(got.len(), 2, "{got:?}");
        assert!(matches!(&got[0].1.body, Some(Body::Settings(s))
            if s.effective.iter().any(|e| e.key == "paging.size" && e.value == "4")));
        assert!(matches!(&got[1].1.body, Some(Body::Event(e))
            if matches!(e.kind, Some(event::Kind::SettingsChanged(_)))));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "# note\nengine.lm\ton\npaging.size\t4\n"
        );
    }

    #[test]
    fn bridge_rejects_invalid_values_with_the_key_only_and_writes_nothing() {
        let dir = TempDir::new();
        let path = dir.0.join("settings.tsv");
        std::fs::write(&path, "# note\n").unwrap();
        let mut shell = shell(SettingsSource::File(path.clone()), resources());
        send(&mut shell, 1, hello(1));
        let update = |key: &str, value: &str| UpdateSettings {
            set: vec![SettingEntry {
                key: key.into(),
                value: value.into(),
            }],
            reset: Vec::new(),
        };
        let got = replies(&send(
            &mut shell,
            1,
            frame(9, Body::UpdateSettings(update("paging.size", "99"))),
        ));
        assert_eq!(
            error_code(&got[0].1),
            Some((BridgeErrorCode::InvalidSetting as i32, "paging.size".into()))
        );
        // 知らないキーは名前を返さない
        let got = replies(&send(
            &mut shell,
            1,
            frame(10, Body::UpdateSettings(update("no.such", "1"))),
        ));
        assert_eq!(
            error_code(&got[0].1),
            Some((BridgeErrorCode::InvalidSetting as i32, String::new()))
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# note\n");
    }

    #[test]
    fn bridge_refuses_a_version_mismatch_and_closes_broken_or_unexpected_first_frames() {
        let mut shell = shell(SettingsSource::Inline(String::new()), resources());
        let actions = send(&mut shell, 2, hello(99));
        let got = replies(&actions);
        assert_eq!(
            error_code(&got[0].1).map(|(code, _)| code),
            Some(BridgeErrorCode::VersionMismatch as i32)
        );
        assert!(actions.contains(&ShellAction::BridgeClose { conn: ConnId(2) }));

        let actions = send(&mut shell, 3, frame(1, Body::GetSettings(GetSettings {})));
        assert!(replies(&actions).is_empty());
        assert!(actions.contains(&ShellAction::BridgeClose { conn: ConnId(3) }));

        let actions = send(&mut shell, 4, vec![0xff, 0xff]);
        assert!(actions.contains(&ShellAction::BridgeClose { conn: ConnId(4) }));
    }

    #[test]
    fn notifications_go_only_to_a_ready_connection_and_stop_after_it_closes() {
        let mut shell = shell(SettingsSource::Inline(String::new()), resources());
        let open = |shell: &mut Shell| {
            shell.dispatch(
                ShellEvent::MenuCommand {
                    command: MenuCommand::OpenSettings {
                        tab: Some(crate::types::SettingsTab::UserDictionary),
                    },
                },
                &mut NoText,
            )
        };
        assert!(replies(&open(&mut shell)).is_empty());
        send(&mut shell, 6, hello(1));
        let actions = open(&mut shell);
        let got = replies(&actions);
        assert!(matches!(&got[0].1.body, Some(Body::Event(e))
            if matches!(&e.kind, Some(event::Kind::OpenTab(t)) if t.tab == "dictionary")));
        assert_eq!(
            actions.last(),
            Some(&ShellAction::OpenSettings {
                tab: Some(crate::types::SettingsTab::UserDictionary)
            })
        );
        shell.dispatch(ShellEvent::BridgeClosed { conn: ConnId(6) }, &mut NoText);
        assert!(replies(&open(&mut shell)).is_empty());
    }

    #[test]
    fn eisu_and_kana_keys_are_absorbed_in_any_mode() {
        let mut shell = shell(SettingsSource::Inline(String::new()), resources());
        activate(&mut shell);
        for kind in [KeyKind::Eisu, KeyKind::Kana] {
            let actions = shell.dispatch(key(kind), &mut NoText);
            assert!(actions.contains(&ShellAction::Consume));
            assert!(!actions.contains(&ShellAction::PassThrough));
        }
    }

    #[test]
    fn jis_keys_with_layout_text_are_read_as_characters_unless_kana_input() {
        let jis = |text: &str, command: bool| KeyInput {
            modifiers: Modifiers {
                command,
                ..Modifiers::NONE
            },
            layout_text: Some(text.into()),
            ..KeyInput::new(KeyKind::Jis { code: 22 })
        };
        let character = |text: &str| KeyKind::Character { text: text.into() };
        assert_eq!(
            japanese_key(&jis("a", false), InputMethodKind::Romaji).key,
            character("a")
        );
        assert_eq!(
            japanese_key(&jis("a", false), InputMethodKind::Kana).key,
            KeyKind::Jis { code: 22 }
        );
        assert_eq!(
            japanese_key(&jis("a", true), InputMethodKind::Kana).key,
            character("a")
        );
        assert_eq!(
            japanese_key(&jis("", false), InputMethodKind::Romaji).key,
            KeyKind::Other
        );
        // 文字がないとき（シナリオ）は、届いたまま
        let bare = KeyInput::new(KeyKind::Jis { code: 22 });
        assert_eq!(japanese_key(&bare, InputMethodKind::Romaji), bare);
        // 英語のモードは配列の文字を優先する（刻印は `a` でも、配列が出す文字）
        assert_eq!(english_key(&jis("q", false)).key, character("q"));
    }

    #[test]
    fn panel_side_follows_panel_events_and_resets_when_hidden() {
        let mut shell = shell(SettingsSource::Inline(String::new()), resources());
        activate(&mut shell);
        shell.dispatch(
            ShellEvent::PanelShown {
                side: PanelSide::Above,
                source: Some(AnchorSource::Client),
            },
            &mut NoText,
        );
        assert_eq!(shell.machine().unwrap().panel_side(), PanelSide::Above);
        shell.dispatch(ShellEvent::PanelHidden, &mut NoText);
        assert_eq!(shell.machine().unwrap().panel_side(), PanelSide::Below);
        shell.dispatch(
            ShellEvent::PanelShown {
                side: PanelSide::Above,
                source: None,
            },
            &mut NoText,
        );
        // `HideCandidates` を返した回（ここではモードの切り替え）は、OS の知らせを待たずに戻す
        let actions = shell.dispatch(
            ShellEvent::InputModeChanged {
                mode_id: crate::english::input_mode_id::ENGLISH.into(),
                secure: false,
            },
            &mut NoText,
        );
        assert!(actions.contains(&ShellAction::HideCandidates));
        assert_eq!(shell.machine().unwrap().panel_side(), PanelSide::Below);
        assert_eq!(shell.input_mode(), InputMode::English);
    }
}

// ---------------------------------------------------------------- 学習ファイルの退避の通知（#271）

mod learning_quarantine {
    use super::*;

    fn engine_info(shell: &Shell, learning_quarantined: bool) -> JobResult {
        JobResult::EngineInfo {
            attempt: 0,
            generation: shell.poll_generation,
            lm: Some(EngineLmState::Ready),
            profile: None,
            learning_quarantined,
        }
    }

    fn count(actions: &[ShellAction]) -> usize {
        actions
            .iter()
            .filter(|a| **a == ShellAction::ShowLearningQuarantineNotice)
            .count()
    }

    fn new_shell() -> Shell {
        shell(SettingsSource::Inline(String::new()), resources())
    }

    #[test]
    fn quarantined_hello_shows_the_notice_once() {
        let mut shell = new_shell();
        let first = shell.accept(engine_info(&shell, true));
        assert_eq!(count(&first), 1);
        // エンジンは 1 回しか true を返さない。続く Hello（false）では出ない
        let second = shell.accept(engine_info(&shell, false));
        assert_eq!(count(&second), 0);
    }

    #[test]
    fn normal_hello_shows_nothing() {
        let mut shell = new_shell();
        assert_eq!(count(&shell.accept(engine_info(&shell, false))), 0);
    }

    #[test]
    fn a_stale_generation_still_delivers_the_notice() {
        let mut shell = new_shell();
        let stale = JobResult::EngineInfo {
            attempt: 0,
            generation: shell.poll_generation.wrapping_add(1),
            lm: Some(EngineLmState::Ready),
            profile: None,
            learning_quarantined: true,
        };
        assert_eq!(count(&shell.accept(stale)), 1);
    }
}
