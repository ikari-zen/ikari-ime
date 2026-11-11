//! `Shell` を作る（設定と資源の場所、トランスポート、時計）。
//!
//! - 設定: `%LOCALAPPDATA%\ikari-ime\settings.tsv`（`IKARI_USER_DIR` があればその下）。`core::user::UserDir` と同じ決め方を、
//!   DLL に `ikari-core`（辞書など）を入れないために、ここで繰り返す。場所が決められなければ空の設定（既定値）で動く。
//! - 資源（キー割り当ての表など）: DLL と同じ場所の `resources`。TODO(W5): インストーラの配置に合わせる。

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::sync::Arc;

use ikari_shell_core::{SettingsSource, Shell, ShellConfig, SystemClock};
use windows::Win32::System::LibraryLoader::GetModuleFileNameW;

use super::connector::PipeConnector;
use super::{dll, log};
use crate::latency::{self, Meter};
use crate::transport::{DownSink, PipeTransport};

/// `core::user::dir` の `USER_DIR_ENV`・`LOCAL_APPDATA_ENV`・`APP_DIR_NAME` と同じ値
const USER_DIR_ENV: &str = "IKARI_USER_DIR";
const LOCAL_APPDATA_ENV: &str = "LOCALAPPDATA";
const APP_DIR_NAME: &str = "ikari-ime";
/// `engine::SETTINGS_FILE_NAME` と同じ値
const SETTINGS_FILE_NAME: &str = "settings.tsv";
/// DLL の隣の資源のディレクトリ
const RESOURCES_DIR_NAME: &str = "resources";
/// パスの上限（UTF-16。長いパスを含む）
const MAX_LONG_PATH: usize = 32_768;
/// 最初に読むパスのバッファ
const INITIAL_PATH_LEN: usize = 260;

fn env_dir(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

pub fn user_dir() -> Option<PathBuf> {
    env_dir(USER_DIR_ENV)
        .or_else(|| env_dir(LOCAL_APPDATA_ENV).map(|local| local.join(APP_DIR_NAME)))
}

/// `module` の実行ファイルのパス（`None` は今のプロセスの exe）。
pub fn module_file(module: Option<windows::Win32::Foundation::HMODULE>) -> Option<PathBuf> {
    let mut buffer = vec![0u16; INITIAL_PATH_LEN];
    loop {
        // SAFETY: `module` は自分の DLL のハンドルか `None`（今のプロセスの exe）。`buffer` は書ける長さを渡す
        let written = unsafe { GetModuleFileNameW(module, &mut buffer) } as usize;
        if written == 0 {
            return None;
        }
        if written < buffer.len() {
            return Some(PathBuf::from(OsString::from_wide(&buffer[..written])));
        }
        if buffer.len() >= MAX_LONG_PATH {
            return None;
        }
        buffer.resize((buffer.len() * 2).min(MAX_LONG_PATH), 0);
    }
}

/// 入力先のアプリの名前（`ShellEvent::Activate` の `bundle_id`。実行ファイルの名前。取れなければ空）。
pub fn client_name() -> String {
    module_file(None)
        .and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_default()
}

fn resources_dir() -> PathBuf {
    dll::module()
        .and_then(|module| module_file(Some(module)))
        .and_then(|path| path.parent().map(|dir| dir.join(RESOURCES_DIR_NAME)))
        .unwrap_or_default()
}

/// このプロセスの往復時間の測定（S1。オンかは最初に呼んだときに決める。既定はオフ。`latency`）
pub fn meter() -> &'static Arc<Meter> {
    latency::global(user_dir)
}

/// `Shell` を作る。作れなければ `None`（キーはすべてアプリに渡す）。
pub fn create_shell() -> Option<Shell> {
    let settings = match user_dir() {
        Some(dir) => SettingsSource::File(dir.join(SETTINGS_FILE_NAME)),
        None => SettingsSource::Inline(String::new()),
    };
    let config = ShellConfig {
        shell_version: env!("CARGO_PKG_VERSION").to_owned(),
        settings,
        resources_dir: resources_dir(),
    };
    let sink = DownSink::new();
    let transport =
        PipeTransport::new(PipeConnector, Arc::clone(&sink)).with_meter(Arc::clone(meter()));
    match Shell::new(config, Box::new(transport), Arc::new(SystemClock::new())) {
        Ok(shell) => {
            sink.bind(shell.engine());
            Some(shell)
        }
        Err(_) => {
            log::event("cannot create the shell");
            None
        }
    }
}
