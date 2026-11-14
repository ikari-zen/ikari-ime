//! ikari-engine.exe: Windows の常駐のエンジン（ADR 0040 §2.2、§7 の W5-a）。
//!
//! 起動と終了の手順は `ikari_engine_host::run`（OS に依存しない。Mac でテストする）。ここは、`cfg(windows)` の
//! 部分（パイプの受け口、Ctrl+C の受け取り）をつなぐだけ。判断の置き場所: Rust。判断は engine。
//!
//! 終了コード: 0 は正常、またはすでに別のエンジンが動いている。1 は非 Windows。2 は起動できない。
//! 64 は引数の誤り。`hide-console` を付けたビルドは、コンソールの窓を出さない。

#![cfg_attr(all(windows, feature = "hide-console"), windows_subsystem = "windows")]

use std::process::ExitCode;

#[cfg(not(windows))]
fn main() -> ExitCode {
    eprintln!("{}", ikari_engine_host::run::UNSUPPORTED_MESSAGE);
    ExitCode::from(ikari_engine_host::run::EXIT_UNSUPPORTED)
}

#[cfg(windows)]
fn main() -> ExitCode {
    ExitCode::from(windows_main())
}

#[cfg(windows)]
fn windows_main() -> u8 {
    use std::sync::Arc;

    use ikari_engine_host::run::{
        EXIT_OK, EXIT_START_FAILED, EXIT_USAGE, StartError, StopWait, parse_args, record_failure,
        resolve_data_dir, run_host, start,
    };
    use ikari_engine_host::settings_pipe::bind_settings_listener;
    use ikari_engine_host::{ctrl, endsession};
    use ikari_pipe::Shutdown;
    use ikari_pipe::os::windows::NamedPipeListener;

    let options = match parse_args(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(e) => {
            eprintln!("ikari-engine: {e}");
            return EXIT_USAGE;
        }
    };
    let dir = match resolve_data_dir(options.data_dir.as_deref()) {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("ikari-engine: {e}");
            return e.exit_code();
        }
    };
    let started = match start(&dir) {
        Ok(started) => started,
        Err(e) => {
            // すでに動いているときは静かに終わる（何も出さない）
            if !matches!(e, StartError::AlreadyRunning) {
                eprintln!("ikari-engine: {e}");
                record_failure(&dir, &e);
            }
            return e.exit_code();
        }
    };

    // エンジンが開いてから受け口を作る（つながってきた相手を、開く前に待たせない）
    let bound = NamedPipeListener::bind_current_user()
        .and_then(|engine| bind_settings_listener().map(|settings| (engine, settings)));
    let (engine_listener, settings_listener) = match bound {
        Ok(pair) => pair,
        Err(e) => {
            let e = StartError::Failed(format!("cannot create the pipes: {e}"));
            eprintln!("ikari-engine: {e}");
            record_failure(&dir, &e);
            return EXIT_START_FAILED;
        }
    };
    let stoppers: Vec<Arc<dyn Shutdown>> = vec![
        Arc::new(engine_listener.closer()),
        Arc::new(settings_listener.closer()),
    ];

    let stop = StopWait::new();
    if let Err(e) = ctrl::install(stop.handle()) {
        // 受け取れなくても動く（プロセスの終了で止まる）
        eprintln!("ikari-engine: cannot install the Ctrl+C handler: {e}");
    }
    if cfg!(feature = "hide-console") {
        // コンソールがなく、制御イベントが届かない。ログオフ・シャットダウンは非表示の最上位ウィンドウで受ける（#372）
        if let Err(e) = endsession::spawn(Arc::downgrade(&started.engine)) {
            eprintln!("ikari-engine: cannot watch the session end: {e}");
        }
    }
    let result = run_host(engine_listener, settings_listener, started, &stoppers, stop);
    ctrl::mark_finished();
    match result {
        Ok(()) => EXIT_OK,
        Err(e) => {
            eprintln!("ikari-engine: the pipe server stopped: {e}");
            EXIT_START_FAILED
        }
    }
}
