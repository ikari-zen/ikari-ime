//! 子プロセスの起動と回収の OS ごとの部分。`unsafe` はこのモジュール（`os/` の下を含む）に閉じる。
//!
//! - 回収: kill のあと必ず wait する（ゾンビを残さない）。`Drop` でも同じ。
//! - 子に渡すのは、標準入力・標準出力（パイプ）と標準エラー（捨て先）の 3 つだけ（ADR 0068 §6.1、0067 §4.3）。
//!   実行ファイルは、渡された絶対パスをそのまま使い、PATH から探さない。
//! - 親が死んだとき: 子は stdin が閉じたことで自分で終わる（`serve`）。Windows は、それに加えて Job Object に入れ、
//!   親のハンドルが閉じたら OS が子を終わらせる。
//! - 檻の段 1（ADR 0072 §3）: Windows は `windows` の説明のとおり（Job の制限、低い整合性）。macOS は、エンジンの
//!   App Sandbox を引き継ぐ（今のまま）。
//! - 檻の段 2（ADR 0072 §3、§12 の実測）: Windows だけ。[`Cage::AppContainer`] で、能力なしの AppContainer に入れる
//!   （`appcontainer`）。読めるのは、渡したファイル 1 つ（起動の前に読み取りを許す）と、すべてのアプリのパッケージに
//!   開かれた場所（インストール先の Program Files、システムのフォルダ）だけ。ほかの OS では起動しない（失敗を返す）。
//!   段 2 が張れないときに段 1 へ落とすかは、呼び出し側が決める（ここでは落とさない）。
//! - 絞りがかけられないときは、子を止めて失敗を返す。呼び出し側（`scorer`）は失敗に数え、LM なしで動く。
//! - macOS: 子の stdin のパイプに `F_SETNOSIGPIPE` を付ける。エンジンは Swift の XPC サービスの中の静的ライブラリで、
//!   Rust の `main` を通らないので SIGPIPE が無視されていない。付けないと、死んだ子への書き込みでエンジンが落ちる。
//!   ほかの unix は、Rust の実行時（`main`）が SIGPIPE を無視している前提（製品の対象ではない。テストと CLI だけ）。

use std::ffi::OsString;
use std::io;
use std::path::Path;
#[cfg(not(windows))]
use std::process::{ChildStdin, ChildStdout};

/// LM の子の AppContainer（段 2）。部品は `ikari_sandbox::appcontainer`（ADR 0078 §3.2 の 3）。ここは LM の
/// プロファイルの名前を決めて呼ぶだけ。能力は 1 つも付けない
#[cfg(windows)]
mod appcontainer {
    use std::io;
    use std::path::Path;

    use ikari_sandbox::appcontainer::{self as sandbox, ContainerProfile};

    pub(super) use ikari_sandbox::appcontainer::Container as AppContainerSid;
    pub(super) use ikari_sandbox::appcontainer::grant_read;

    /// プロファイル（利用者ごとに 1 つ）
    #[cfg(not(test))]
    pub(super) const PROFILE: ContainerProfile = ContainerProfile {
        name: crate::host::CAGE_PROFILE_NAME,
        display_name: "ikari LM host",
    };
    /// 単体テストのプロファイル（製品のプロファイルに触れない）
    #[cfg(test)]
    pub(super) const PROFILE: ContainerProfile = ContainerProfile {
        name: "io.github.ikari-zen.lm-host-test",
        display_name: "ikari LM host",
    };

    /// プロファイルを用意して、その SID を返す（無ければ作る）
    pub(super) fn ensure_profile() -> io::Result<AppContainerSid> {
        AppContainerSid::ensure(&PROFILE, &[])
    }

    /// プロファイルを消す（アンインストールのとき。無くても成功）
    pub(super) fn delete_profile() -> io::Result<()> {
        sandbox::delete_profile(&PROFILE)
    }

    /// `path` から、LM のプロファイルの SID の ACE を外す（無くても成功）
    pub(super) fn revoke_read(path: &Path) -> io::Result<()> {
        sandbox::revoke_read(path, &PROFILE)
    }
}
#[cfg(target_vendor = "apple")]
mod apple;
#[cfg(any(windows, test))]
mod cmdline;
#[cfg(unix)]
mod fd;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
pub(crate) use fd::fd_facts;

#[cfg(target_vendor = "apple")]
use apple as imp;
#[cfg(not(any(target_vendor = "apple", windows)))]
use other_unix as imp;
#[cfg(windows)]
use windows as imp;

/// 子の標準入力に書く口
#[cfg(not(windows))]
pub(super) type ToChild = ChildStdin;
/// 子の標準出力を読む口
#[cfg(not(windows))]
pub(super) type FromChild = ChildStdout;
/// 子の標準入力に書く口。Windows は `CreatePipe` の同期のハンドルのまま読み書きする型を使う
/// （`ChildStdin` / `ChildStdout` は、重なった I/O で開いたハンドルを前提に読み書きするので、同期のハンドルでは、
/// 子が終わっても読み取りが終わらない。実機で確かめた）
#[cfg(windows)]
pub(super) type ToChild = io::PipeWriter;
/// 子の標準出力を読む口（Windows。上の `ToChild` の説明のとおり）
#[cfg(windows)]
pub(super) type FromChild = io::PipeReader;

/// 子にかける檻（ADR 0072 §3）
#[derive(Clone, Copy, Debug)]
pub(super) enum Cage<'a> {
    /// 段 1（Windows: Job の制限と、低い整合性。macOS: エンジンの Sandbox を引き継ぐ）
    Standard,
    /// 段 2（Windows だけ）: 能力なしの AppContainer。`readable` があれば、読み取りを許してから起動する
    /// （None は許可を足さない。継承の読み取りで読める同梱の場所のモデル。ADR 0080 §3.1）
    #[cfg_attr(not(windows), allow(dead_code))]
    AppContainer { readable: Option<&'a Path> },
}

/// 実際にかけた檻（`lm_load` のログの `box=` の語）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CageState {
    /// まだ起動していない
    Unknown,
    /// AppContainer（段 2。Windows）
    AppContainer,
    /// 低い整合性と Job（段 1。Windows）
    LowIntegrity,
    /// Job だけ（段 1 から低い整合性を外した開発用の設定。Windows）、または絞りなし（製品の対象でない unix）
    None,
    /// エンジンの App Sandbox を引き継ぐ（段 1。macOS）
    Inherited,
    /// 檻役が起動し、檻役の App Sandbox を引き継ぐ（段 2。macOS。ADR 0077）
    Cage,
}

impl CageState {
    pub(crate) fn word(self) -> &'static str {
        match self {
            CageState::Unknown => "unknown",
            CageState::AppContainer => "appcontainer",
            CageState::LowIntegrity => "low",
            CageState::None => "none",
            CageState::Inherited => "inherited",
            CageState::Cage => "cage",
        }
    }

    pub(crate) const ALL: [CageState; 6] = [
        CageState::Unknown,
        CageState::AppContainer,
        CageState::LowIntegrity,
        CageState::None,
        CageState::Inherited,
        CageState::Cage,
    ];
}

/// 子の持ち手
enum ProcHandle {
    /// 自分で起動した子
    Own(imp::Proc),
    /// 檻役が起動した子（止めるのは檻役に頼む。macOS）
    #[cfg(target_vendor = "apple")]
    Caged(Box<dyn crate::host::cage::CageChild>),
}

#[cfg(all(test, windows))]
impl ProcHandle {
    /// 子のプロセスのハンドル（Windows のテストが、かけた檻を確かめる）
    fn process_handle(&self) -> windows_sys::Win32::Foundation::HANDLE {
        let ProcHandle::Own(p) = self;
        p.process_handle()
    }
}

/// 起動した子。捨てると kill して wait する（檻役の子は、檻役に止めて回収してもらう）
pub(super) struct ChildProc {
    inner: ProcHandle,
    cage: CageState,
}

impl ChildProc {
    pub(super) fn id(&self) -> u32 {
        match &self.inner {
            ProcHandle::Own(p) => p.id(),
            #[cfg(target_vendor = "apple")]
            ProcHandle::Caged(c) => c.pid(),
        }
    }

    /// この子にかけた檻
    pub(super) fn cage(&self) -> CageState {
        self.cage
    }
}

impl Drop for ChildProc {
    fn drop(&mut self) {
        match &mut self.inner {
            ProcHandle::Own(p) => p.reap(),
            // 返事が来なくても、持ち手を捨てると接続が切れ、檻役が止める
            #[cfg(target_vendor = "apple")]
            ProcHandle::Caged(c) => {
                let _ = c.stop();
            }
        }
    }
}

/// `program`（絶対パス）を、標準入出力をパイプにして、`cage` の檻に入れて起動する。標準エラーは捨てる
pub(super) fn spawn(
    program: &Path,
    args: &[OsString],
    cage: Cage<'_>,
) -> io::Result<(ChildProc, ToChild, FromChild)> {
    #[cfg(windows)]
    let (inner, stdin, stdout, cage) = imp::spawn(program, args, cage)?;
    #[cfg(not(windows))]
    let (inner, stdin, stdout, cage) = match cage {
        Cage::Standard => {
            let (inner, stdin, stdout) = imp::spawn(program, args)?;
            (inner, stdin, stdout, NOT_WINDOWS_CAGE)
        }
        Cage::AppContainer { .. } => {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "appcontainer is windows only",
            ));
        }
    };
    // ここから先は、失敗しても `proc` の Drop が子を回収する
    let proc = ChildProc {
        inner: ProcHandle::Own(inner),
        cage,
    };
    #[cfg(target_vendor = "apple")]
    imp::no_sigpipe(&stdin)?;
    Ok((proc, stdin, stdout))
}

/// 檻役（`launcher` の口）に子の起動を頼む（macOS の段 2。ADR 0077）。
///
/// 契約のスタブ（第 4 弾）: 中身はレーン D-3-mac。`args` は今の `--model <path> --gpu <語>` のまま渡す
/// （檻役には `--gpu` の語だけを [`cage::CageGpu`] で送り、モデルは照合したハンドルの写し（`model.for_child()`。
/// 起動のたびに位置を先頭へ戻したもの）を fd で渡す。パスで開き直さない。FC-3）。
/// 口が無い（`slot` が空）ときは「檻なし」で起動しない。檻役の子は段 2 なので、`cage` の求め（段 1 か段 2）は
/// どれも満たす。macOS 以外では起動しない（`Unsupported`）。
///
/// [`cage::CageGpu`]: crate::host::cage::CageGpu
pub(super) fn spawn_via_cage(
    slot: &crate::host::cage::CageSlot,
    model: &crate::host::ModelHandle,
    args: &[OsString],
    cage: crate::host::scorer::HostCage,
) -> io::Result<(ChildProc, ToChild, FromChild)> {
    let _ = cage;
    #[cfg(target_vendor = "apple")]
    return apple_cage::spawn(slot, model, args);
    #[cfg(not(target_vendor = "apple"))]
    {
        let _ = (slot, model, args);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "lm cage launcher is macos only",
        ))
    }
}

/// 檻役に頼む起動（macOS）
#[cfg(target_vendor = "apple")]
mod apple_cage {
    use std::ffi::OsString;
    use std::io;
    use std::os::fd::OwnedFd;
    use std::process::{ChildStdin, ChildStdout};

    use super::{CageState, ChildProc, ProcHandle, imp};
    use crate::host::ARG_GPU;
    use crate::host::ModelHandle;
    use crate::host::cage::{CageGpu, CageLaunch, CageSlot};

    /// `--gpu <語>` の語（無い・知らない語は誤り）
    fn gpu_of(args: &[OsString]) -> io::Result<CageGpu> {
        args.iter()
            .position(|a| a == ARG_GPU)
            .and_then(|i| args.get(i + 1))
            .and_then(|v| v.to_str())
            .and_then(CageGpu::from_word)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "lm host gpu argument"))
    }

    pub(super) fn spawn(
        slot: &CageSlot,
        model: &ModelHandle,
        args: &[OsString],
    ) -> io::Result<(ChildProc, ChildStdin, ChildStdout)> {
        let Some(cage) = slot.current() else {
            return Err(io::Error::new(io::ErrorKind::NotConnected, "no lm cage"));
        };
        let gpu = gpu_of(args)?;
        // 子が読む側（標準入力）と、子が書く側（標準出力）
        let (child_in, parent_in) = io::pipe()?;
        let (parent_out, child_out) = io::pipe()?;
        let launch = CageLaunch {
            stdin: child_in,
            stdout: child_out,
            model: model.for_child()?,
            gpu,
        };
        // 子の側の端は `launch` の中で閉じる
        let child = cage.launch(launch)?;
        let proc = ChildProc {
            inner: ProcHandle::Caged(child),
            cage: CageState::Cage,
        };
        let stdin = ChildStdin::from(OwnedFd::from(parent_in));
        let stdout = ChildStdout::from(OwnedFd::from(parent_out));
        imp::no_sigpipe(&stdin)?;
        Ok((proc, stdin, stdout))
    }
}

/// Windows でない OS の段 1 の檻
#[cfg(target_vendor = "apple")]
const NOT_WINDOWS_CAGE: CageState = CageState::Inherited;
#[cfg(not(any(target_vendor = "apple", windows)))]
const NOT_WINDOWS_CAGE: CageState = CageState::None;

/// `path` から、段 2 の子への読み取りの許可を外す（Windows。無くても成功）。ほかの OS では何もしない
pub(super) fn revoke_cage_read(path: &Path) -> io::Result<()> {
    #[cfg(windows)]
    return appcontainer::revoke_read(path);
    #[cfg(not(windows))]
    {
        let _ = path;
        Ok(())
    }
}

/// 段 2 の AppContainer のプロファイルを消す（Windows。アンインストールのとき。無くても成功）。
/// 消えるのは、呼んだ利用者のものだけ（プロファイルは利用者ごと）
#[cfg(windows)]
pub(super) fn delete_cage_profile() -> io::Result<()> {
    appcontainer::delete_profile()
}

/// `SetDefaultDllDirectories` に渡す DLL 探索の範囲（ADR 0067 §4.4: system32 と自分の exe のフォルダだけ）。
/// `LOAD_LIBRARY_SEARCH_APPLICATION_DIR` (0x200) | `LOAD_LIBRARY_SEARCH_SYSTEM32` (0x800)。
pub(super) const DLL_SEARCH_FLAGS: u32 = 0x0000_0200 | 0x0000_0800;

/// 以後の DLL の読み込みを system32 と自分のフォルダに限る（ADR 0067 §4.4）。Windows 以外では何もしない
pub(super) fn restrict_dll_search() {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::LibraryLoader::{
            LOAD_LIBRARY_SEARCH_APPLICATION_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32,
            SetDefaultDllDirectories,
        };
        const _: () = assert!(
            DLL_SEARCH_FLAGS
                == (LOAD_LIBRARY_SEARCH_APPLICATION_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32)
        );
        // SAFETY: 引数は定数のフラグの組み合わせ。失敗しても既定の探索のまま続ける
        unsafe { SetDefaultDllDirectories(DLL_SEARCH_FLAGS) };
    }
}

/// 製品の対象でない unix（テストと CLI だけ）。`std::process` で起動する。Rust の標準ライブラリが開く fd は
/// close-on-exec なので、子に渡るのは標準入出力だけ
#[cfg(not(any(target_vendor = "apple", windows)))]
mod other_unix {
    use std::ffi::OsString;
    use std::io;
    use std::path::Path;
    use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

    pub(super) struct Proc(Child);

    impl Proc {
        pub(super) fn id(&self) -> u32 {
            self.0.id()
        }

        pub(super) fn reap(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    pub(super) fn spawn(
        program: &Path,
        args: &[OsString],
    ) -> io::Result<(Proc, ChildStdin, ChildStdout)> {
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let pipes = (child.stdin.take(), child.stdout.take());
        let mut proc = Proc(child);
        match pipes {
            (Some(stdin), Some(stdout)) => Ok((proc, stdin, stdout)),
            _ => {
                proc.reap();
                Err(io::Error::other("child pipes unavailable"))
            }
        }
    }
}

#[cfg(all(test, target_vendor = "apple"))]
mod tests {
    use std::io::{Read, Write};
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

    use super::*;

    /// 親で、close-on-exec の印なしに開いておく fd の番号（ほかのテストの fd と重ならない大きさ）
    const PROBE_FD: i32 = 200;

    fn probe_script() -> OsString {
        format!("if [ -e /dev/fd/{PROBE_FD} ]; then echo open; else echo closed; fi").into()
    }

    fn run(stdin: ChildStdin, mut stdout: ChildStdout) -> String {
        drop(stdin);
        let mut out = String::new();
        stdout.read_to_string(&mut out).unwrap();
        out
    }

    #[test]
    fn child_stdin_does_not_raise_sigpipe() {
        let (_proc, stdin, _stdout) = spawn(Path::new("/bin/cat"), &[], Cage::Standard).unwrap();
        assert!(imp::has_no_sigpipe(&stdin));
    }

    #[test]
    fn the_child_gets_only_the_three_standard_fds() {
        let file = tempfile::tempfile().unwrap();
        // SAFETY: 開いた fd を、使われていない番号に複製するだけ（`dup2` の結果には close-on-exec が付かない）
        let fd = unsafe { libc::dup2(file.as_raw_fd(), PROBE_FD) };
        assert_eq!(fd, PROBE_FD);
        // SAFETY: いま開いた fd で、ほかに持ち主はいない
        let _probe = unsafe { OwnedFd::from_raw_fd(fd) };

        // 対照: `std::process` の子には渡る（このテストが意味を持つことの確かめ）
        let leaked = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(probe_script())
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&leaked.stdout), "open\n");

        let (_proc, stdin, stdout) = spawn(
            Path::new("/bin/sh"),
            &["-c".into(), probe_script()],
            Cage::Standard,
        )
        .unwrap();
        assert_eq!(run(stdin, stdout), "closed\n");
    }

    #[test]
    fn stdin_and_stdout_reach_the_child() {
        let (_proc, mut stdin, stdout) = spawn(Path::new("/bin/cat"), &[], Cage::Standard).unwrap();
        stdin.write_all(b"ping\n").unwrap();
        assert_eq!(run(stdin, stdout), "ping\n");
    }

    #[test]
    fn a_missing_program_is_an_error() {
        assert!(spawn(Path::new("/nonexistent/ikari-lm-host"), &[], Cage::Standard).is_err());
    }

    #[test]
    fn the_child_inherits_the_engine_sandbox() {
        let (proc, _stdin, _stdout) = spawn(Path::new("/bin/cat"), &[], Cage::Standard).unwrap();
        assert_eq!(proc.cage(), CageState::Inherited);
    }

    #[test]
    fn appcontainer_is_refused_outside_windows() {
        let model = Path::new("/nonexistent/model.gguf");
        let err = spawn(
            Path::new("/bin/cat"),
            &[],
            Cage::AppContainer {
                readable: Some(model),
            },
        )
        .err()
        .unwrap();
        assert_eq!(err.kind(), io::ErrorKind::Unsupported);
    }

    #[test]
    fn dropping_the_child_kills_and_reaps_it() {
        let (proc, _stdin, _stdout) = spawn(Path::new("/bin/cat"), &[], Cage::Standard).unwrap();
        let pid = proc.id() as libc::pid_t;
        drop(proc);
        // 回収済みなら、もう自分の子ではない
        // SAFETY: 状態を聞くだけ（WNOHANG）
        let rc = unsafe { libc::waitpid(pid, std::ptr::null_mut(), libc::WNOHANG) };
        assert_eq!(rc, -1);
    }
}

#[cfg(test)]
mod cage_tests {
    use super::{CageState, DLL_SEARCH_FLAGS, restrict_dll_search};

    #[test]
    fn cage_words_are_fixed_and_distinct() {
        let mut words: Vec<&str> = CageState::ALL.iter().map(|s| s.word()).collect();
        words.sort_unstable();
        words.dedup();
        assert_eq!(words.len(), CageState::ALL.len());
    }

    #[test]
    fn dll_search_flags_allow_only_application_dir_and_system32() {
        // LOAD_LIBRARY_SEARCH_APPLICATION_DIR (0x200) | LOAD_LIBRARY_SEARCH_SYSTEM32 (0x800)
        assert_eq!(DLL_SEARCH_FLAGS, 0x0000_0200 | 0x0000_0800);
        assert_eq!(DLL_SEARCH_FLAGS & !0x0000_0a00, 0);
        restrict_dll_search();
    }
}

#[cfg(all(test, windows))]
mod windows_tests;
