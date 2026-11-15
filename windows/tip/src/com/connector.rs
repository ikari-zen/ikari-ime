//! 名前付きパイプへの接続（`transport::Connector` の Windows の実装）。
//!
//! パイプの名前は `pipe::pipe_name(ユーザーの SID, セッション番号)`。SID とセッション番号の取得、重ね合わせの I/O の
//! `Duplex` は名前付きパイプの OS 層（W1-b。`pipe/src/os/windows.rs`。#315）が持つ。
//!
//! つながらなければ `None`（`TransportError::Unavailable`。キーは失わず、漢字に変換できないだけ）。
//!
//! # 相手の確認（#409）
//! つないだら、何かを送る前に、相手が本物のエンジンかを確かめる。相手の PID（pipe 層が返す）から実行ファイルの
//! パスを取り、TIP の DLL 自身のパスとあわせて `peer::verify_engine_path`（判断。OS に依存しない）に渡す。
//! ここは OS からパスを取ることと、8.3 の短い名前を長い名前に直すことだけを行う。PID やパスが取れないときも拒む
//! （フェイルクローズ）。拒んだら接続を閉じて `None`（何も送らない）。理由の種類だけを診断のログに 1 行出す。
//!
//! この確認は、インストール先に書き込めるのが管理者だけであることを前提にする（本番は Program Files。
//! ADR 0063 Q2）。開発用の導入のように利用者が書ける場所では、同じ利用者が書ける場所を信頼するだけになる。
//!
//! エンジンの起動（常駐の `engine-host` がないとき）は W5（S3）。

use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::io::{FromRawHandle, OwnedHandle};
use std::path::Path;

use ikari_pipe::os::windows::{NamedPipeDuplex, connect_duplex, current_pipe_name};
use windows::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, HANDLE};
use windows::Win32::Storage::FileSystem::GetLongPathNameW;
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows_core::{PCWSTR, PWSTR};

use super::host::{INITIAL_PATH_LEN, MAX_LONG_PATH, module_file};
use super::{dll, log};
use crate::peer::{PeerRejection, verify_engine_path};
use crate::transport::Connector;

pub struct PipeConnector;

impl Connector for PipeConnector {
    type Conn = NamedPipeDuplex;

    fn open(&self) -> Option<(NamedPipeDuplex, Option<u32>)> {
        let name = current_pipe_name().ok()?;
        let tip_dll = dll::module().and_then(|module| module_file(Some(module)));
        open_verified(&name, tip_dll.as_deref())
    }
}

/// `name` につなぎ、相手が `tip_dll` から決まるエンジンなら流れを返す。違えば閉じて `None`。
fn open_verified(name: &str, tip_dll: Option<&Path>) -> Option<(NamedPipeDuplex, Option<u32>)> {
    let (duplex, pid) = connect_duplex(name).ok()?;
    match verify_peer(tip_dll, pid) {
        Ok(()) => Some((duplex, pid)),
        Err(reason) => {
            // 何も送らずに閉じる（ハンドルを閉じれば、相手には切断に見える）
            drop(duplex);
            log::peer_rejected(reason.as_str());
            None
        }
    }
}

fn verify_peer(tip_dll: Option<&Path>, pid: Option<u32>) -> Result<(), PeerRejection> {
    let tip_dll = tip_dll.ok_or(PeerRejection::NoTipPath)?;
    let pid = pid.ok_or(PeerRejection::NoPeerPid)?;
    let peer_exe = process_image_path(pid).ok_or(PeerRejection::NoPeerPath)?;
    let tip_dll = long_path(tip_dll.as_os_str());
    let peer_exe = long_path(&peer_exe);
    let tip_dll = tip_dll.to_str().ok_or(PeerRejection::TipPathInvalid)?;
    let peer_exe = peer_exe.to_str().ok_or(PeerRejection::PeerPathInvalid)?;
    verify_engine_path(tip_dll, peer_exe)
}

/// プロセス `pid` の実行ファイルのフルパス（Win32 の形。OS が起動のときに記録したもの）。取れなければ `None`。
fn process_image_path(pid: u32) -> Option<OsString> {
    // SAFETY: 引数は値だけ。成功したら返るハンドルを下で所有する
    let raw: HANDLE = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    // SAFETY: raw は OpenProcess が返した有効なハンドルで、ほかに所有者はいない（落ちるときに閉じる）
    let _process = unsafe { OwnedHandle::from_raw_handle(raw.0) };
    let mut buffer = vec![0u16; INITIAL_PATH_LEN];
    loop {
        let mut len = u32::try_from(buffer.len()).ok()?;
        // SAFETY: raw は上で開いたプロセスのハンドル（`_process` が所有し、まだ閉じていない）。
        // buffer は len 個の u16 を書ける。成功すると len に NUL を除いた長さが入る
        let result = unsafe {
            QueryFullProcessImageNameW(
                raw,
                PROCESS_NAME_WIN32,
                PWSTR(buffer.as_mut_ptr()),
                &mut len,
            )
        };
        match result {
            Ok(()) => {
                let len = usize::try_from(len).ok()?.min(buffer.len());
                return Some(OsString::from_wide(&buffer[..len]));
            }
            Err(e) if e.code() == ERROR_INSUFFICIENT_BUFFER.to_hresult() => {
                if buffer.len() >= MAX_LONG_PATH {
                    return None;
                }
                buffer.resize((buffer.len() * 2).min(MAX_LONG_PATH), 0);
            }
            Err(_) => return None,
        }
    }
}

/// 8.3 の短い名前を長い名前に直す（`GetLongPathNameW`）。直せなければそのまま返す（そのままでは期待のパスと
/// 一致しないので、拒む側に倒れる）。
///
/// 各段の名前を、同じフォルダの中の長い名前に置き換えるだけで、親のフォルダの並びは変わらない（リンクの先に
/// 置き換えるのではない）。利用者の書ける場所から起動したプロセスのパスを、インストール先に見せることはできない。
fn long_path(path: &OsStr) -> OsString {
    let wide: Vec<u16> = path.encode_wide().chain(Some(0)).collect();
    let mut buffer = vec![0u16; INITIAL_PATH_LEN];
    loop {
        // SAFETY: wide は NUL で終わる UTF-16 で、この呼び出しの間生きている。buffer はその長さまで書ける
        let written =
            unsafe { GetLongPathNameW(PCWSTR(wide.as_ptr()), Some(&mut buffer)) } as usize;
        if written == 0 {
            return path.to_owned();
        }
        if written < buffer.len() {
            return OsString::from_wide(&buffer[..written]);
        }
        // 足りないときは、NUL を含めた要る長さが返る
        if written > MAX_LONG_PATH {
            return path.to_owned();
        }
        buffer.resize(written, 0);
    }
}

/// TIP の DLL のパスを、試験で決めた場所にして `open_verified` を確かめる（Windows の CI でだけ流れる）。
#[cfg(test)]
mod tests {
    use super::*;
    use ikari_pipe::os::windows::NamedPipeListener;
    use ikari_pipe::{Duplex, Listener, Shutdown};
    use std::io::{BufRead, BufReader, Read};
    use std::path::PathBuf;
    use std::process::{Child, Command, Stdio};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::mpsc;
    use std::time::Duration;

    /// 子のプロセスに、偽のエンジン（パイプの受け口）として動かせるための環境変数（値はパイプの名前）
    const FAKE_ENGINE_ENV: &str = "IKARI_TIP_TEST_FAKE_ENGINE_PIPE";
    /// 子が受け口を作ったと知らせる行
    const READY_LINE: &str = "fake-engine-ready";
    const FAKE_ENGINE_TEST: &str = "com::connector::tests::fake_engine";
    /// 子が受け口を作るまで待つ上限（試験が固まらないための締め切り。exe の写しの初回の検査を見込んだ目安）
    const READY_TIMEOUT: Duration = Duration::from_secs(30);
    /// 子が自分で終わるまでの上限（親が止め損ねても、子が残り続けないため）
    const FAKE_ENGINE_LIFETIME: Duration = Duration::from_secs(60);

    static NEXT_NAME: AtomicU32 = AtomicU32::new(0);

    fn unique_name() -> String {
        format!(
            r"\\.\pipe\ikari-tip-peer-test-{}-{}",
            std::process::id(),
            NEXT_NAME.fetch_add(1, Ordering::Relaxed)
        )
    }

    /// 子のプロセスの中身: 受け口を作り、1 本受けて、相手が閉じるまで読んで終わる。ふだんは何もしない。
    #[test]
    #[ignore = "helper process for the peer tests (runs only with IKARI_TIP_TEST_FAKE_ENGINE_PIPE)"]
    fn fake_engine() {
        let Ok(name) = std::env::var(FAKE_ENGINE_ENV) else {
            return;
        };
        let lifetime = ikari_testtime::scaled(FAKE_ENGINE_LIFETIME);
        std::thread::spawn(move || {
            std::thread::sleep(lifetime);
            std::process::exit(1);
        });
        let mut listener = NamedPipeListener::bind(&name).expect("bind the fake engine pipe");
        // `--test-threads=1` の libtest は「test 名前 ... 」を改行なしで先に出すので、行の頭から書く
        println!("\n{READY_LINE}");
        if let Ok(Some(duplex)) = listener.accept() {
            let (mut reader, _writer, _closer) = duplex.split();
            let mut sink = Vec::new();
            let _ = reader.read_to_end(&mut sink);
        }
    }

    /// このテストの exe を `dir\ikari-engine-host.exe` に写して、偽のエンジンとして起動する。受け口ができるまで待つ。
    fn spawn_fake_engine(dir: &Path, name: &str) -> Child {
        std::fs::create_dir_all(dir).expect("create the folder");
        let exe = dir.join(crate::peer::ENGINE_EXE_NAME);
        std::fs::copy(std::env::current_exe().expect("current exe"), &exe).expect("copy the exe");
        let mut child = Command::new(&exe)
            .args([
                FAKE_ENGINE_TEST,
                "--exact",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(FAKE_ENGINE_ENV, name)
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn the fake engine");
        let stdout = child.stdout.take().expect("stdout");
        let (ready_tx, ready_rx) = mpsc::channel();
        // 準備の行を見たら知らせ、そのあとも子の出力を終わりまで読み捨てる（子の書きが詰まらないように）
        std::thread::spawn(move || {
            let mut ready_tx = Some(ready_tx);
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if line.trim() == READY_LINE
                    && let Some(tx) = ready_tx.take()
                {
                    let _ = tx.send(());
                }
            }
        });
        if ready_rx
            .recv_timeout(ikari_testtime::scaled(READY_TIMEOUT))
            .is_err()
        {
            finish(child);
            panic!("the fake engine did not start");
        }
        child
    }

    /// 期待のフォルダ（8.3 を展開した長い名前。CI の一時フォルダは短い名前のことがある）
    fn long_dir(dir: &Path) -> PathBuf {
        PathBuf::from(long_path(dir.as_os_str()))
    }

    fn finish(mut child: Child) {
        // 接続を閉じたら子は終わる。終わらなければ止める（自分が起こしたテストのプロセスだけ）
        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn rejects_a_listener_that_is_not_the_engine() {
        // この試験の exe 自身が受け口を作る（`ikari-engine-host.exe` ではない）
        let name = unique_name();
        let mut listener = NamedPipeListener::bind(&name).expect("bind");
        let closer = listener.closer();
        let accept = std::thread::spawn(move || {
            let _ = listener.accept();
        });
        let dir = long_dir(&std::env::temp_dir());
        let tip_dll = dir.join("ikari_tip.dll");
        let rejected = open_verified(&name, Some(&tip_dll)).is_none();
        // つながらなかったときも受け付けの待ちを起こす（試験を固めない）
        closer.shutdown();
        accept.join().expect("accept thread");
        assert!(rejected);
    }

    #[test]
    fn rejects_an_engine_named_exe_in_another_folder() {
        let temp = tempfile::tempdir().expect("temp dir");
        let root = long_dir(temp.path());
        let name = unique_name();
        let child = spawn_fake_engine(&root.join("elsewhere"), &name);
        let tip_dll = root
            .join("install")
            .join("tip")
            .join("1")
            .join("ikari_tip.dll");
        assert!(open_verified(&name, Some(&tip_dll)).is_none());
        finish(child);
    }

    #[test]
    fn rejects_without_the_tip_path() {
        let temp = tempfile::tempdir().expect("temp dir");
        let root = long_dir(temp.path());
        let name = unique_name();
        let child = spawn_fake_engine(&root, &name);
        assert!(open_verified(&name, None).is_none());
        finish(child);
    }

    #[test]
    fn accepts_the_engine_at_the_install_folder() {
        let temp = tempfile::tempdir().expect("temp dir");
        let root = long_dir(temp.path());
        let name = unique_name();
        let child = spawn_fake_engine(&root, &name);
        let tip_dll = root.join("tip").join("1").join("ikari_tip.dll");
        let (duplex, pid) = open_verified(&name, Some(&tip_dll)).expect("the engine is accepted");
        assert_eq!(pid, Some(child.id()));
        drop(duplex);
        finish(child);
    }

    #[test]
    fn accepts_the_engine_next_to_the_dll() {
        let temp = tempfile::tempdir().expect("temp dir");
        let root = long_dir(temp.path());
        let name = unique_name();
        let child = spawn_fake_engine(&root, &name);
        let tip_dll = root.join("ikari_tip.dll");
        assert!(open_verified(&name, Some(&tip_dll)).is_some());
        finish(child);
    }
}
