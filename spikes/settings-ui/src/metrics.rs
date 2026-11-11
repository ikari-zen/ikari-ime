//! 起動時間とメモリの計測（Windows のみ。候補窓のスパイク #319 の `common.rs` から抜いたもの）。
//! 出力は `RESULT` で始まる行。

use windows::Win32::Foundation::{CloseHandle, FILETIME};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::ProcessStatus::{
    GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
};
use windows::Win32::System::SystemInformation::GetSystemTimePreciseAsFileTime;
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, GetProcessTimes, OpenProcess,
    PROCESS_QUERY_LIMITED_INFORMATION,
};

pub fn qpc() -> i64 {
    let mut v = 0i64;
    // SAFETY: 出力先は有効な i64
    unsafe {
        let _ = QueryPerformanceCounter(&mut v);
    }
    v
}

pub fn qpc_ms(from: i64, to: i64) -> f64 {
    let mut f = 0i64;
    // SAFETY: 出力先は有効な i64
    unsafe {
        let _ = QueryPerformanceFrequency(&mut f);
    }
    (to - from) as f64 * 1000.0 / f.max(1) as f64
}

fn filetime_u64(ft: FILETIME) -> u64 {
    ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64
}

/// プロセスが作られてからの経過（ms）。コールドスタートの「起動から」の基準
pub fn ms_since_process_creation() -> Option<f64> {
    let (mut c, mut e, mut k, mut u) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    // SAFETY: 出力先はすべて有効
    unsafe {
        GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u).ok()?;
        let now = filetime_u64(GetSystemTimePreciseAsFileTime());
        Some((now.saturating_sub(filetime_u64(c))) as f64 / 10_000.0) // 100ns 単位
    }
}

/// (WorkingSet, PrivateUsage) のバイト数
fn memory_of(pid: u32) -> Option<(usize, usize)> {
    // SAFETY: ハンドルは使ったあとに閉じる。構造体の cb は大きさに合わせる
    unsafe {
        let own = pid == GetCurrentProcessId();
        let h = if own {
            GetCurrentProcess()
        } else {
            OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?
        };
        let mut c = PROCESS_MEMORY_COUNTERS_EX {
            cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
            ..Default::default()
        };
        let r = GetProcessMemoryInfo(
            h,
            &mut c as *mut PROCESS_MEMORY_COUNTERS_EX as *mut PROCESS_MEMORY_COUNTERS,
            c.cb,
        );
        if !own {
            let _ = CloseHandle(h);
        }
        r.ok()?;
        Some((c.WorkingSetSize, c.PrivateUsage))
    }
}

/// 自分と、その子孫のプロセス（WebView2 の msedgewebview2.exe 群）を (pid, 名前) で返す
fn self_and_descendants() -> Vec<(u32, String)> {
    // SAFETY: 引数なし
    let me = unsafe { GetCurrentProcessId() };
    let mut all: Vec<(u32, u32, String)> = Vec::new();
    // SAFETY: スナップショットは閉じる。構造体の dwSize を設定する
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return vec![(me, "self".into())];
        };
        let mut e = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut ok = Process32FirstW(snap, &mut e).is_ok();
        while ok {
            let len = e
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(e.szExeFile.len());
            all.push((
                e.th32ProcessID,
                e.th32ParentProcessID,
                String::from_utf16_lossy(&e.szExeFile[..len]),
            ));
            ok = Process32NextW(snap, &mut e).is_ok();
        }
        let _ = CloseHandle(snap);
    }
    let mut out: Vec<(u32, String)> = vec![(me, "self".into())];
    let mut i = 0;
    while i < out.len() {
        let parent = out[i].0;
        for (pid, ppid, name) in &all {
            if *ppid == parent && !out.iter().any(|(p, _)| p == pid) {
                out.push((*pid, name.clone()));
            }
        }
        i += 1;
    }
    out
}

/// 自分と子孫のメモリを出す。合計の PrivateUsage が「常駐のメモリ」の目安
pub fn print_memory(tag: &str) {
    let (mut ws_sum, mut priv_sum) = (0usize, 0usize);
    for (pid, name) in self_and_descendants() {
        if let Some((ws, pr)) = memory_of(pid) {
            println!(
                "MEM {tag} pid={pid} {name} working_set={}MB private={}MB",
                ws >> 20,
                pr >> 20
            );
            ws_sum += ws;
            priv_sum += pr;
        }
    }
    println!(
        "RESULT MEM {tag} total working_set={}MB private={}MB",
        ws_sum >> 20,
        priv_sum >> 20
    );
}

/// 最初の描画が終わったときの記録（両方式で同じ書式）。
/// `t_main` は main の先頭での `qpc()`。
pub fn report_first_paint(tag: &str, t_main: i64) {
    println!(
        "RESULT {tag} first_paint_from_main={:.1}ms",
        qpc_ms(t_main, qpc())
    );
    if let Some(ms) = ms_since_process_creation() {
        println!("RESULT {tag} cold_start_total={ms:.1}ms (process creation -> first paint)");
    }
    print_memory(&format!("{tag} first-paint"));
}
