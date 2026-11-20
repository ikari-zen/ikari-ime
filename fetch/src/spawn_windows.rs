//! Windows: 自分（`ikari-data`）を、絞った子（`ikari-data transfer …`、`version-probe`）として起こす
//! （ADR 0076 §3.2、ADR 0078 §3.1・§3.2）。`transfer` は取得の子だけの AppContainer（能力は `internetClient` だけ）、
//! `version-probe` は低い整合性。
//!
//! 起動の作りは `ikari-sandbox`（止めた状態で作り、Job Object に入れてから動かす。継承するハンドルを限る。整合性を
//! 低くする。窓を出さない。実行ファイルは絶対パスで明示し、PATH から探さない。シェルを通さない）。
//! ここで決めるのは、この子への求めだけ:
//! - 整合性は低。Job の中のプロセスは子の 1 つだけ。UI の制限は LM の子と同じ。
//! - `Confine::AppContainer` は、プロファイル（無ければ作る）と `internetClient` の能力だけを付けて起こす。読み取りの
//!   許可は 1 つも足さない（書く先は継承したハンドルだけ）。どの檻で起こしたかは `--box` で子に知らせる（子の自己確認）。
//!   起こせないときに低い整合性へ落とすかは、呼ぶ側（`run.rs`）が決める。
//! - 標準入力と標準エラーは渡さない。
//! - 環境は、プロキシの変数と OS が動くのに要るものだけ（`child::filter_env`。ADR 0076 §3.4）。
//! - `transfer` には、親が作った `download.bin` のハンドルから作った、**追記だけ**の写し（`FILE_APPEND_DATA |
//!   SYNCHRONIZE`）を 1 つだけ継承させ、その値を `--out-handle` で知らせる。子は読めず、切り詰められない。
//!   子は場所を一つも持たない。
//! - 親は、子の行を読みながら、自分のハンドルで大きさを見張る（上限を超えたら Job を閉じて `too-large`）。
//!   子が終わったら、自分のハンドルでディスクに出す（`FlushFileBuffers`）。

use std::ffi::OsString;
use std::fs::File;
use std::io::{self, BufReader, PipeReader};
use std::os::windows::io::{AsHandle, AsRawHandle, BorrowedHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};

use ikari_resource::fetch_report::{
    ChildBox, FetchErrorCode, Line, Report, ReportReader, read_line_capped,
};
use ikari_sandbox::appcontainer::{Capability, Container, ContainerProfile};
use ikari_sandbox::{Confined, Integrity, JobLimits, Launch, Stdio, environment_block, spawn};
use windows_sys::Win32::Foundation::{DuplicateHandle, HANDLE};
use windows_sys::Win32::Security::{GetTokenInformation, TOKEN_QUERY, TokenIsAppContainer};
use windows_sys::Win32::Storage::FileSystem::{FILE_APPEND_DATA, SYNCHRONIZE};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use crate::child::{SizeWatch, filter_env, watch_size, with_box, with_out_handle};
use crate::error::FetchError;
use crate::limits::SIZE_WATCH_INTERVAL;

/// `transfer` の子の Job の制限（子そのもの 1 つ。窓を持たない）
const TRANSFER_JOB: JobLimits = JobLimits { max_processes: 1 };
/// 子に渡す書く先の写しの権利。追記と待つことだけ（読む、切り詰める、位置を選んで書くことはできない）
const CHILD_OUT_ACCESS: u32 = FILE_APPEND_DATA | SYNCHRONIZE;

/// 取得の子の能力（ADR 0078 §3.2 の 2。`privateNetworkClientServer` は付けない）
const TRANSFER_CAPABILITIES: &[Capability] = &[Capability::InternetClient];

/// 取得の子だけの AppContainer のプロファイル（LM の子と分ける。ADR 0078 §3.2 の 1）
#[cfg(not(test))]
pub const TRANSFER_PROFILE: ContainerProfile = ContainerProfile {
    name: crate::child::TRANSFER_PROFILE_NAME,
    display_name: "ikari data transfer",
};
/// 単体テストのプロファイル（製品のプロファイルに触れない）
#[cfg(test)]
pub const TRANSFER_PROFILE: ContainerProfile = ContainerProfile {
    name: "io.github.ikari-zen.data-transfer-test",
    display_name: "ikari data transfer",
};

/// 子をどの檻で起こすか（ADR 0078 §4.2）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confine {
    /// 低い整合性
    Low,
    /// AppContainer（プロファイルは無ければ作る。能力は `internetClient` だけ。整合性は低）
    AppContainer(&'static ContainerProfile),
}

impl Confine {
    /// 子に知らせる檻の語
    fn child_box(self) -> ChildBox {
        match self {
            Self::Low => ChildBox::Low,
            Self::AppContainer(_) => ChildBox::AppContainer,
        }
    }
}

/// `program args…` を低い整合性で起こす（`version-probe`。書く先を渡さない）。子の標準出力の読み取り側を返す。
/// 子は、捨てると（Job を閉じると）終わる
pub fn spawn_low(program: &Path, args: &[String]) -> io::Result<(Confined, PipeReader)> {
    spawn_confined(program, args, &[], Confine::Low)
}

/// `transfer` を起こす。`out`（親が作った `download.bin`）の追記だけの写しを作って継承させ、`--box <檻>` と
/// `--out-handle <値>` を引数の末尾に足す。親の写しは、起動のあとに閉じる（子が持つ写しは、子が終わると OS が閉じる）
pub fn spawn_transfer(
    program: &Path,
    args: &[String],
    out: &File,
    confine: Confine,
) -> io::Result<(Confined, PipeReader)> {
    let child_out = append_only(out)?;
    let value = child_out.as_raw_handle() as usize as u64;
    let args = with_out_handle(with_box(args.to_vec(), confine.child_box()), value);
    spawn_confined(program, &args, &[child_out.as_handle()], confine)
}

/// 子の行を読み、大きさを見張り、終わるのを待ち、書いた分をディスクに出す（ADR 0078 §3.1 の 6・7）。
/// 子の行は信じない。進み具合は表示にだけ使い、受けた大きさは写すときに数え直す。上限を超えたら、子の行より先に
/// `too-large`
pub fn supervise_transfer(
    child: &Confined,
    stdout: PipeReader,
    out: &File,
    limit: u64,
    progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<(), FetchError> {
    let tool = || FetchError(FetchErrorCode::Tool);
    let done = AtomicBool::new(false);
    let (watched, failure, exit) = std::thread::scope(|scope| {
        let watcher = scope.spawn(|| {
            watch_size(
                &|| out.metadata().map(|m| m.len()),
                limit,
                SIZE_WATCH_INTERVAL,
                &|| child.terminate(),
                &done,
            )
        });
        let failure = read_reports(stdout, progress);
        let exit = child.wait();
        done.store(true, Ordering::Release);
        watcher.thread().unpark();
        // 見張りが落ちたら、上限を超えたかは分からない。止めて失敗にする
        let watched = watcher.join().unwrap_or_else(|_| {
            child.terminate();
            SizeWatch::TooLarge
        });
        (watched, failure, exit)
    });
    if watched == SizeWatch::TooLarge {
        return Err(FetchErrorCode::TooLarge.into());
    }
    let exit = exit.map_err(|_| tool())?;
    // 子は `sync_all` を呼ばない。親が自分のハンドルでディスクに出す
    out.sync_all()
        .map_err(|_| FetchError(FetchErrorCode::Internal))?;
    match (failure, exit) {
        (Some(code), _) => Err(FetchError(code)),
        (None, 0) => Ok(()),
        (None, _) => Err(FetchError(FetchErrorCode::Internal)),
    }
}

/// 子の行を終わりまで読む。進み具合を `progress` に渡し、`error` の行があればその語を返す
fn read_reports(
    stdout: PipeReader,
    progress: &mut dyn FnMut(u64, Option<u64>),
) -> Option<FetchErrorCode> {
    let mut reader = BufReader::new(stdout);
    let mut reports = ReportReader::new();
    let mut failure = None;
    loop {
        match read_line_capped(&mut reader) {
            Ok(Line::Text(line)) => match reports.feed(&line) {
                Some(Report::Progress { received, total }) => progress(received, total),
                Some(Report::Error(code)) => failure = Some(code),
                Some(_) | None => {}
            },
            Ok(Line::Dropped) => {}
            Ok(Line::Eof) | Err(_) => break,
        }
    }
    failure
}

/// `file` の、追記だけの写し（継承の印なし。印は `ikari-sandbox` が起動の間だけ付ける）
fn append_only(file: &File) -> io::Result<OwnedHandle> {
    let mut copy: HANDLE = ptr::null_mut();
    // SAFETY: 自分のプロセスの擬似ハンドルと、持っている開いたファイルのハンドル。書き込み先は `copy`。
    // 求める権利は元の権利（読み書き）の一部
    let ok = unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            file.as_raw_handle() as HANDLE,
            GetCurrentProcess(),
            &mut copy,
            CHILD_OUT_ACCESS,
            0,
            0,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: 成功した `DuplicateHandle` が返した、まだ誰も持っていないハンドル
    Ok(unsafe { OwnedHandle::from_raw_handle(copy) })
}

fn spawn_confined(
    program: &Path,
    args: &[String],
    inherit: &[BorrowedHandle<'_>],
    confine: Confine,
) -> io::Result<(Confined, PipeReader)> {
    let env = environment_block(filter_env(std::env::vars_os()))?;
    let args: Vec<OsString> = args.iter().map(OsString::from).collect();
    // AppContainer の SID と能力の SID は、起動が終わるまで持つ
    let container = match confine {
        Confine::Low => None,
        Confine::AppContainer(profile) => Some(Container::ensure(profile, TRANSFER_CAPABILITIES)?),
    };
    // 子が書く側（標準出力）。親の端は継承できない形のまま
    let (parent_out, child_out) = io::pipe()?;
    let child_out = OwnedHandle::from(child_out);
    let child = spawn(&Launch {
        program,
        args: &args,
        environment: Some(&env),
        integrity: Integrity::Low,
        job: TRANSFER_JOB,
        stdio: Stdio {
            output: Some(child_out.as_handle()),
            ..Stdio::default()
        },
        inherit,
        container: container.as_ref(),
    })?;
    // 子の側の端は、親では閉じる（閉じないと、子が終わっても読み取りが終わらない）
    drop(child_out);
    Ok((child, parent_out))
}

/// 自分のトークンが AppContainer か（`transfer` の自己確認。ADR 0078 §3.2 の 4）。取れなければ `None`
pub fn current_is_appcontainer() -> Option<bool> {
    let mut token: HANDLE = ptr::null_mut();
    // SAFETY: 自分のプロセスの擬似ハンドルと、書き込み先を渡すだけ
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return None;
    }
    // SAFETY: 成功した `OpenProcessToken` が返した、まだ誰も持っていないハンドル
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    let mut value = 0u32;
    let mut len = 0u32;
    // SAFETY: `value` は渡す大きさの、書き込める領域
    let ok = unsafe {
        GetTokenInformation(
            token.as_raw_handle() as HANDLE,
            TokenIsAppContainer,
            ptr::from_mut(&mut value).cast(),
            size_of::<u32>() as u32,
            &mut len,
        )
    };
    (ok != 0).then_some(value != 0)
}

#[cfg(test)]
mod tests;
