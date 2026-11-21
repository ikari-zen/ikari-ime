//! `ikari-data.exe` を起こす OS の部分（ADR 0076 §3.1、§3.2。D-2c）。
//!
//! - 実行ファイルは、自分（設定画面）の場所から決めたインストール先の絶対パス（PATH から探さない。シェルを通さない）。
//!   `<インストール先>\settings\ikari-settings.exe` の隣の `<インストール先>\ikari-data.exe`。
//! - 起動の作りは `ikari-sandbox`: 止めた状態で作り、Job Object に入れてから動かす（入る前に動く隙を作らない）。
//!   Job: 閉じたら終わる。プロセスの数の上限は `ikari-data` と `transfer` の 2。
//!   継承するハンドルは標準出力のパイプだけ（標準入力と標準エラーは渡さない）。窓を出さない。
//!   整合性は設定画面と同じ（中。`inbox` に書くため。ADR 0076 §3.2）。
//! - 設定画面のプロセス自身は通信しない（通信するのは `ikari-data` だけ。ADR 0066 §6.2 の L2）。
//!
//! 判断の置き場所: Rust。Windows 以外では起こせない（`Unsupported`。画面は取得の入口を出さない）。

use std::io;
use std::path::{Path, PathBuf};

use crate::fetch::{Launcher, Running};

const DATA_EXE: &str = "ikari-data.exe";
const SETTINGS_EXE: &str = "ikari-settings.exe";
const SETTINGS_DIR: &str = "settings";

/// 自分の実行ファイルの場所から、`ikari-data.exe` の場所を決める。自分がインストール先の形（`…\settings\ikari-settings.exe`）
/// でなければ `None`（開発用の置き場所からは起こさない）
pub fn data_exe_path(own: &Path) -> Option<PathBuf> {
    let name = own.file_name()?.to_str()?;
    let dir = own.parent()?;
    let dir_name = dir.file_name()?.to_str()?;
    if !name.eq_ignore_ascii_case(SETTINGS_EXE) || !dir_name.eq_ignore_ascii_case(SETTINGS_DIR) {
        return None;
    }
    Some(dir.parent()?.join(DATA_EXE))
}

/// 製品の起こし方
pub struct ProductLauncher;

#[cfg(not(windows))]
impl Launcher for ProductLauncher {
    fn launch(&self, _args: &[String]) -> io::Result<Running> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "ikari-data は Windows の設定画面からだけ起こす",
        ))
    }
}

/// `ikari-data` の Job の制限（`ikari-data` と、その子の `transfer`）。UI の制限は `ikari-sandbox` が必ずかける
/// （`ikari-data` に画面は無い。ADR 0076 §3.2）
#[cfg(windows)]
const DATA_JOB: ikari_sandbox::JobLimits = ikari_sandbox::JobLimits { max_processes: 2 };

#[cfg(windows)]
impl Launcher for ProductLauncher {
    fn launch(&self, args: &[String]) -> io::Result<Running> {
        use std::ffi::OsString;
        use std::os::windows::io::{AsHandle, OwnedHandle};
        use std::sync::Arc;

        use ikari_sandbox::{Integrity, Launch, Stdio, spawn};

        let own = std::env::current_exe()?;
        let exe = data_exe_path(&own)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "not installed"))?;
        let args: Vec<OsString> = args.iter().map(OsString::from).collect();
        // 子が書く側（標準出力）。親の端は継承できない形のまま
        let (stdout, child_out) = io::pipe()?;
        let child_out = OwnedHandle::from(child_out);
        let child = spawn(&Launch {
            program: &exe,
            args: &args,
            environment: None,
            integrity: Integrity::Inherit,
            job: DATA_JOB,
            stdio: Stdio {
                output: Some(child_out.as_handle()),
                ..Stdio::default()
            },
            inherit: &[],
            container: None,
        })?;
        // 子の側の端は、親では閉じる（閉じないと、子が終わっても読み取りが終わらない）
        drop(child_out);
        // `kill` は Job の中をすべて止める（`transfer` も）。`wait` は、`ikari-data` が終わったあとに Job の中の
        // 残りを止める（Job を閉じたのと同じ）
        let child = Arc::new(child);
        let kill_child = Arc::clone(&child);
        Ok(Running {
            stdout: Box::new(stdout),
            kill: Box::new(move || kill_child.terminate()),
            wait: Box::new(move || {
                let status = child.wait().ok();
                child.terminate();
                status.map(|code| i32::from_ne_bytes(code.to_ne_bytes()))
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_installed_layout_is_accepted() {
        let ok = Path::new("/p/ikari-ime/settings/ikari-settings.exe");
        assert_eq!(
            data_exe_path(ok),
            Some(PathBuf::from("/p/ikari-ime/ikari-data.exe"))
        );
        let upper = Path::new("/p/ikari-ime/Settings/IKARI-SETTINGS.EXE");
        assert!(data_exe_path(upper).is_some());
        for bad in [
            "/p/ikari-ime/ikari-settings.exe",
            "/p/ikari-ime/settings/other.exe",
            "/p/ikari-ime/dev/ikari-settings.exe",
            "ikari-settings.exe",
            "/",
        ] {
            assert_eq!(data_exe_path(Path::new(bad)), None, "{bad}");
        }
    }
}
