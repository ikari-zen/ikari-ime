//! 殻の 1 本の口（`ikari_shell_*`、`ikari_settings_*`。ADR 0041 §4.4）を、C から呼ぶのと同じ形で Rust から呼ぶ。
//!
//! bytes の往復、不正な bytes、誤りの回の pass_through、解放（二重の free、free 後の使用、release の回数）、
//! どのスレッドからでも呼べる engine_down を確かめる。panic の止め方は `src/shell.rs` の単体テスト
//! （dispatch の中で panic させる仕掛けはテストのビルドにだけある）。

use std::ffi::c_void;
use std::ptr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use ikari_proto::shell::v1 as pb;
use ikari_shell_core::{KeyInput, KeyKind, Modifiers, ShellAction, ShellEvent};
use ikari_shell_ffi::convert;
use ikari_shell_ffi::*;
use prost::Message;

// ---------------------------------------------------------------- 替え玉のホスト

/// ホストの ctx。release の回数を数える
#[derive(Default)]
struct Fake {
    released: AtomicUsize,
}

/// `release` の回数が `want` になるのを待って確かめる。`ikari_shell_free` は表から外すだけで、
/// 中身は最後の Arc（問い合わせのスレッドが持つ分を含む）が落ちたときに片付くので、free の直後は数が足りないことがある。
/// 多すぎる（二重の release）ときは、待たずに失敗する。
fn assert_released(fake: &Fake, want: usize) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let got = fake.released.load(Ordering::SeqCst);
        if got >= want || std::time::Instant::now() >= deadline {
            assert_eq!(got, want);
            // 二重の release が遅れて来ないことも、少し待って確かめる
            std::thread::sleep(std::time::Duration::from_millis(20));
            assert_eq!(fake.released.load(Ordering::SeqCst), want);
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

unsafe extern "C" fn surrounding(_: *mut c_void, _: *mut IkariShellBuf) -> i32 {
    1
}
unsafe extern "C" fn selection(_: *mut c_void, _: *mut IkariShellBuf) -> i32 {
    1
}
unsafe extern "C" fn connect(_: *mut c_void) -> u64 {
    0
}
unsafe extern "C" fn request(
    _: *mut c_void,
    _: u64,
    _: IkariStr,
    _: u32,
    _: *mut IkariShellBuf,
) -> i32 {
    IKARI_SHELL_TRANSPORT_UNAVAILABLE
}
unsafe extern "C" fn post(_: *mut c_void, _: u64, _: IkariStr) {}
unsafe extern "C" fn close(_: *mut c_void, _: u64) {}
unsafe extern "C" fn pid(_: *mut c_void, _: u64) -> i32 {
    -1
}
unsafe extern "C" fn release(ctx: *mut c_void) {
    // ctx は Arc::into_raw で渡した。release で 1 つ分を返す
    let fake = unsafe { Arc::from_raw(ctx as *const Fake) };
    fake.released.fetch_add(1, Ordering::SeqCst);
}

fn host(fake: &Arc<Fake>) -> IkariShellHost {
    IkariShellHost {
        ctx: Arc::into_raw(Arc::clone(fake)) as *mut c_void,
        surrounding: Some(surrounding),
        selection: Some(selection),
        engine_connect: Some(connect),
        engine_request: Some(request),
        engine_post: Some(post),
        engine_close: Some(close),
        engine_pid: Some(pid),
        release: Some(release),
    }
}

fn config() -> IkariShellConfig {
    IkariShellConfig {
        shell_version: IkariStr::of("0.1.0-test"),
        settings_path: IkariStr::EMPTY,
        resources_dir: IkariStr::EMPTY,
        diag_dir: IkariStr::EMPTY,
    }
}

fn new_shell(fake: &Arc<Fake>) -> *mut IkariShell {
    let mut err = IkariShellBuf::EMPTY;
    let s = unsafe { ikari_shell_new(&config(), host(fake), &mut err) };
    assert!(!s.is_null(), "{}", String::from_utf8_lossy(&take(err)));
    s
}

fn take(buf: IkariShellBuf) -> Vec<u8> {
    unsafe { buf.into_vec() }
}

fn dispatch_bytes(s: *mut IkariShell, event: &[u8]) -> (i32, Vec<ShellAction>) {
    let mut out = IkariShellBuf::EMPTY;
    let status = unsafe { ikari_shell_dispatch(s, IkariStr::of_bytes(event), &mut out) };
    assert!(!out.ptr.is_null() || out.len == 0);
    let actions = convert::decode_actions(&take(out)).expect("actions are always valid");
    (status, actions)
}

fn space() -> Vec<u8> {
    convert::encode_event(&ShellEvent::Key(KeyInput {
        key: KeyKind::Space,
        modifiers: Modifiers::NONE,
        is_repeat: false,
        secure: false,
        layout_text: None,
    }))
}

// ---------------------------------------------------------------- 殻

#[test]
fn dispatch_round_trips_protobuf_bytes() {
    let fake = Arc::new(Fake::default());
    let s = new_shell(&fake);
    for event in [
        space(),
        convert::encode_event(&ShellEvent::Activate {
            bundle_id: "com.apple.TextEdit".into(),
            secure: false,
        }),
        convert::encode_event(&ShellEvent::Deactivate),
        // C 側が proto を直接組み立てた場合も同じ
        pb::ShellEvent {
            event: Some(pb::shell_event::Event::Timer(pb::TimerFired { id: 1 })),
        }
        .encode_to_vec(),
    ] {
        let (status, actions) = dispatch_bytes(s, &event);
        assert_eq!(status, IKARI_SHELL_OK);
        // 返った列は proto から読めて、Rust の `Shell::dispatch` の結果と同じ形（L-A2 で stub でなくなった。
        // 中身はシナリオが縛る。ここは FFI の往復だけを見る）
        assert!(
            actions
                .iter()
                .all(|action| !matches!(action, ShellAction::BridgeReply { .. }))
        );
    }
    // 最初のキー（Space、idle）はアプリへ渡す
    let (_, first) = dispatch_bytes(s, &space());
    assert!(first.contains(&ShellAction::PassThrough));
    unsafe { ikari_shell_free(s) };
    assert_released(&fake, 1);
}

#[test]
fn invalid_event_bytes_return_decode_and_pass_through() {
    let fake = Arc::new(Fake::default());
    let s = new_shell(&fake);
    let unspecified_key = pb::ShellEvent {
        event: Some(pb::shell_event::Event::Key(pb::KeyInput::default())),
    }
    .encode_to_vec();
    let empty_oneof = pb::ShellEvent { event: None }.encode_to_vec();
    for event in [
        vec![0xff, 0xff, 0xff, 0xff],
        vec![0x0a, 0x7f], // 長さが足りない
        unspecified_key,
        empty_oneof,
        Vec::new(),
    ] {
        let (status, actions) = dispatch_bytes(s, &event);
        assert_eq!(status, IKARI_SHELL_ERR_DECODE, "{event:?}");
        assert_eq!(actions, vec![ShellAction::PassThrough]);
    }
    // NULL と巨大な長さ（NULL なので読まない）
    let mut out = IkariShellBuf::EMPTY;
    let status = unsafe {
        ikari_shell_dispatch(
            s,
            IkariStr {
                ptr: ptr::null(),
                len: usize::MAX,
            },
            &mut out,
        )
    };
    assert_eq!(status, IKARI_SHELL_ERR_DECODE);
    take(out);
    // 出力先が NULL
    let event = space();
    assert_eq!(
        unsafe { ikari_shell_dispatch(s, IkariStr::of_bytes(&event), ptr::null_mut()) },
        IKARI_SHELL_ERR_NULL
    );
    unsafe { ikari_shell_free(s) };
}

#[test]
fn free_is_idempotent_and_freed_handles_are_rejected() {
    let fake = Arc::new(Fake::default());
    let s = new_shell(&fake);
    unsafe { ikari_shell_free(s) };
    unsafe { ikari_shell_free(s) }; // 二重の free は何もしない
    assert_released(&fake, 1);

    // free 後の使用
    let (status, actions) = dispatch_bytes(s, &space());
    assert_eq!(status, IKARI_SHELL_ERR_INVALID_HANDLE);
    assert_eq!(actions, vec![ShellAction::PassThrough]);
    unsafe { ikari_shell_engine_down(s, 1, IKARI_SHELL_DOWN_INVALIDATED) };

    // 番号は使い回さない（古いハンドルの二重の free が新しい shell を解放しない）
    let fresh = new_shell(&fake);
    assert_ne!(fresh, s);
    unsafe { ikari_shell_free(s) };
    assert_eq!(dispatch_bytes(fresh, &space()).0, IKARI_SHELL_OK);
    unsafe { ikari_shell_free(fresh) };
    assert_released(&fake, 2);

    // NULL と知らない番号
    unsafe { ikari_shell_free(ptr::null_mut()) };
    assert_eq!(
        dispatch_bytes(ptr::null_mut(), &space()).0,
        IKARI_SHELL_ERR_INVALID_HANDLE
    );
    let bogus = usize::MAX as *mut IkariShell;
    assert_eq!(
        dispatch_bytes(bogus, &space()).0,
        IKARI_SHELL_ERR_INVALID_HANDLE
    );
    unsafe { ikari_shell_free(bogus) };
    // ctx は全部返った（Arc の数が元に戻る）
    assert_eq!(Arc::strong_count(&fake), 1);
}

#[test]
fn new_takes_ownership_of_the_host_even_when_it_fails() {
    let fake = Arc::new(Fake::default());

    let mut missing = host(&fake);
    missing.engine_request = None;
    let mut err = IkariShellBuf::EMPTY;
    assert!(unsafe { ikari_shell_new(&config(), missing, &mut err) }.is_null());
    assert!(String::from_utf8(take(err)).unwrap().contains("NULL"));
    assert_released(&fake, 1);

    assert!(unsafe { ikari_shell_new(ptr::null(), host(&fake), ptr::null_mut()) }.is_null());
    assert_released(&fake, 2);

    let invalid = [0xffu8, 0xfe];
    let bad = IkariShellConfig {
        shell_version: IkariStr::of_bytes(&invalid),
        ..config()
    };
    let mut err = IkariShellBuf::EMPTY;
    assert!(unsafe { ikari_shell_new(&bad, host(&fake), &mut err) }.is_null());
    assert!(String::from_utf8(take(err)).unwrap().contains("UTF-8"));
    assert_released(&fake, 3);
    assert_eq!(Arc::strong_count(&fake), 1);
}

#[test]
fn engine_down_is_callable_from_any_thread_while_dispatching() {
    let fake = Arc::new(Fake::default());
    let s = new_shell(&fake);
    let handle = s as usize;
    let downs: Vec<_> = (0..4)
        .map(|i| {
            std::thread::spawn(move || {
                for generation in 0..2_000u64 {
                    let kind = ((generation + i) % 3) as u8; // 2 は範囲外（無視）
                    unsafe { ikari_shell_engine_down(handle as *mut IkariShell, generation, kind) };
                }
            })
        })
        .collect();
    let event = space();
    for _ in 0..2_000 {
        assert_eq!(dispatch_bytes(s, &event).0, IKARI_SHELL_OK);
    }
    for down in downs {
        down.join().unwrap();
    }
    unsafe { ikari_shell_free(s) };
    assert_released(&fake, 1);
}

#[test]
fn free_while_another_thread_sends_engine_down_releases_once() {
    for _ in 0..200 {
        let fake = Arc::new(Fake::default());
        let s = new_shell(&fake);
        let handle = s as usize;
        let down = std::thread::spawn(move || {
            for generation in 0..50 {
                unsafe {
                    ikari_shell_engine_down(
                        handle as *mut IkariShell,
                        generation,
                        IKARI_SHELL_DOWN_INTERRUPTED,
                    )
                };
            }
        });
        unsafe { ikari_shell_free(s) };
        down.join().unwrap();
        // release は最後の Arc（engine_down が持っていたものを含む）が落ちたときに 1 回だけ
        assert_released(&fake, 1);
    }
}

#[test]
fn buf_copy_makes_a_rust_owned_copy() {
    let data = "あいう".as_bytes();
    let copy = unsafe { ikari_shell_buf_copy(IkariStr::of_bytes(data)) };
    assert_ne!(copy.ptr as *const u8, data.as_ptr());
    assert_eq!(take(copy), data);
    let empty = unsafe {
        ikari_shell_buf_copy(IkariStr {
            ptr: ptr::null(),
            len: usize::MAX,
        })
    };
    assert_eq!(empty.len, 0);
    unsafe { ikari_shell_buf_free(empty) };
}

// ---------------------------------------------------------------- 設定

fn settings_call(f: impl FnOnce(*mut IkariShellBuf) -> i32) -> (i32, Vec<u8>) {
    let mut out = IkariShellBuf::EMPTY;
    let status = f(&mut out);
    (status, take(out))
}

#[test]
fn settings_validate_returns_a_snapshot_and_issues() {
    let tsv = "keymap.preset\tatok\nno.such.key\t1\n";
    let (status, out) =
        settings_call(|out| unsafe { ikari_settings_validate(IkariStr::of(tsv), out) });
    assert_eq!(status, IKARI_SHELL_OK);
    let validation = pb::SettingsValidation::decode(out.as_slice()).unwrap();
    assert_eq!(
        validation.snapshot.unwrap().keymap_preset,
        i32::from(pb::KeymapPreset::Atok)
    );
    assert_eq!(validation.issues.len(), 1);
    assert_eq!(validation.issues[0].line, 2);

    let invalid = [0xffu8];
    let (status, out) =
        settings_call(|out| unsafe { ikari_settings_validate(IkariStr::of_bytes(&invalid), out) });
    assert_eq!(status, IKARI_SHELL_ERR_DECODE);
    assert!(out.is_empty());
    assert_eq!(
        unsafe { ikari_settings_validate(IkariStr::EMPTY, ptr::null_mut()) },
        IKARI_SHELL_ERR_NULL
    );
}

#[test]
fn settings_rewrite_returns_text_or_added_issues() {
    let changes = |key: &str, value: &str| {
        pb::SettingsChanges {
            set: vec![pb::SettingsEntry {
                key: key.into(),
                value: value.into(),
            }],
            reset: Vec::new(),
        }
        .encode_to_vec()
    };
    let file = "# 注釈\nkeymap.preset\tmacos\n";
    let ok = changes("keymap.preset", "atok");
    let (status, out) = settings_call(|out| unsafe {
        ikari_settings_rewrite(IkariStr::of(file), IkariStr::of_bytes(&ok), out)
    });
    assert_eq!(status, IKARI_SHELL_OK);
    match pb::SettingsRewrite::decode(out.as_slice()).unwrap().result {
        Some(pb::settings_rewrite::Result::Text(text)) => {
            assert_eq!(text, "# 注釈\nkeymap.preset\tatok\n")
        }
        other => panic!("{other:?}"),
    }

    let bad_value = changes("keymap.preset", "no-such-preset");
    let (status, out) = settings_call(|out| unsafe {
        ikari_settings_rewrite(IkariStr::of(file), IkariStr::of_bytes(&bad_value), out)
    });
    assert_eq!(status, IKARI_SHELL_OK);
    assert!(matches!(
        pb::SettingsRewrite::decode(out.as_slice()).unwrap().result,
        Some(pb::settings_rewrite::Result::IssuesAdded(_))
    ));

    let garbage = [0xffu8, 0xff];
    let (status, out) = settings_call(|out| unsafe {
        ikari_settings_rewrite(IkariStr::of(file), IkariStr::of_bytes(&garbage), out)
    });
    assert_eq!(status, IKARI_SHELL_ERR_DECODE);
    assert!(out.is_empty());
    assert_eq!(
        unsafe { ikari_settings_rewrite(IkariStr::EMPTY, IkariStr::EMPTY, ptr::null_mut()) },
        IKARI_SHELL_ERR_NULL
    );
}
