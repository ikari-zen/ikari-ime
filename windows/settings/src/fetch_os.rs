//! `ikari-data.exe` を起こす OS の部分（ADR 0076 §3.1、§3.2。D-2c）。
//!
//! - 実行ファイルは、自分（設定画面）の場所から決めたインストール先の絶対パス（PATH から探さない。シェルを通さない）。
//!   `<インストール先>\settings\ikari-settings.exe` の隣の `<インストール先>\ikari-data.exe`。
//! - 子は Job Object に入れる（閉じたら終わる。プロセスの数の上限は `ikari-data` と `transfer` の 2）。
//!   標準入力と標準エラーは渡さず、標準出力のパイプだけを読む。窓を出さない。
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

#[cfg(windows)]
impl Launcher for ProductLauncher {
    fn launch(&self, args: &[String]) -> io::Result<Running> {
        use std::os::windows::process::CommandExt;
        use std::process::{Command, Stdio};
        use std::sync::{Arc, Mutex};

        use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

        let own = std::env::current_exe()?;
        let exe = data_exe_path(&own)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "not installed"))?;
        let job = job::Job::confined()?;
        let mut child = Command::new(&exe)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()?;
        if let Err(e) = job.assign(&child) {
            let _ = child.kill();
            return Err(e);
        }
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("no stdout"))?;
        // Job を閉じると子の木が終わる。`kill` と `wait` が同じ Job を持つ
        let job = Arc::new(Mutex::new(Some(job)));
        let kill_job = Arc::clone(&job);
        Ok(Running {
            stdout: Box::new(stdout),
            kill: Box::new(move || {
                kill_job.lock().unwrap_or_else(|e| e.into_inner()).take();
            }),
            wait: Box::new(move || {
                let status = child.wait().ok().and_then(|s| s.code());
                drop(job);
                status
            }),
        })
    }
}

#[cfg(windows)]
mod job {
    use std::io;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::process::Child;
    use std::ptr;

    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_ACTIVE_PROCESS,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JobObjectExtendedLimitInformation, SetInformationJobObject,
    };

    /// Job の中のプロセスの上限（`ikari-data` と、その子の `transfer`）
    const MAX_ACTIVE_PROCESSES: u32 = 2;

    /// 閉じる（捨てる）と中のプロセスが終わる Job Object
    pub struct Job(OwnedHandle);

    impl Job {
        pub fn confined() -> io::Result<Job> {
            // SAFETY: 名前なし・既定のセキュリティで作る。戻り値は検査する
            let handle = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
            if handle.is_null() {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: 成功した `CreateJobObjectW` が返した、まだ誰も持っていないハンドル
            let job = Job(unsafe { OwnedHandle::from_raw_handle(handle) });
            // SAFETY: すべて 0 の値は、この構造体として正しい（制限なし）
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
            limits.BasicLimitInformation.LimitFlags =
                JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
            limits.BasicLimitInformation.ActiveProcessLimit = MAX_ACTIVE_PROCESSES;
            // SAFETY: `limits` は呼び出しの間生きていて、クラスに合う型で、大きさを正しく渡す
            let ok = unsafe {
                SetInformationJobObject(
                    job.0.as_raw_handle() as HANDLE,
                    JobObjectExtendedLimitInformation,
                    ptr::from_ref(&limits).cast(),
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
            };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(job)
        }

        pub fn assign(&self, child: &Child) -> io::Result<()> {
            // SAFETY: どちらも、まだ閉じていないハンドル
            let ok = unsafe {
                AssignProcessToJobObject(
                    self.0.as_raw_handle() as HANDLE,
                    child.as_raw_handle() as HANDLE,
                )
            };
            if ok == 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        }
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
