//! 3 方式で共通の部分: 時計、メモリ、キャレットの位置、フォーカスの記録。

use windows::Win32::Foundation::{CloseHandle, FILETIME, HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::ClientToScreen;
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
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetCursorPos, GetForegroundWindow, GetGUIThreadInfo, GetWindowLongPtrW,
    GetWindowRect, GetWindowTextW, GetWindowThreadProcessId, SetWindowLongPtrW, GUITHREADINFO,
    GWL_EXSTYLE, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
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
        let h = if pid == GetCurrentProcessId() {
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
        if pid != GetCurrentProcessId() {
            let _ = CloseHandle(h);
        }
        r.ok()?;
        Some((c.WorkingSetSize, c.PrivateUsage))
    }
}

/// 自分と、その子孫のプロセス（WebView2 の msedgewebview2.exe 群）を (pid, 名前) で返す
fn self_and_descendants() -> Vec<(u32, String)> {
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

pub struct Anchor {
    pub x: i32,
    pub y: i32,
    pub source: &'static str,
}

fn window_text(h: HWND) -> String {
    let mut buf = [0u16; 128];
    // SAFETY: バッファは有効
    let n = unsafe { GetWindowTextW(h, &mut buf) } as usize;
    let mut cls = [0u16; 64];
    let m = unsafe { GetClassNameW(h, &mut cls) } as usize;
    format!(
        "{:?} class={:?}",
        String::from_utf16_lossy(&buf[..n]),
        String::from_utf16_lossy(&cls[..m])
    )
}

/// 前面のウィンドウの記録（窓を出す前後で比べる）
pub fn foreground_snapshot() -> (isize, String) {
    // SAFETY: 引数なし
    let h = unsafe { GetForegroundWindow() };
    (h.0 as isize, window_text(h))
}

/// 前面のアプリのキャレットの位置（画面の座標。キャレットの下端の左）。
/// `GetGUIThreadInfo` が空を返すアプリ（自前描画のエディタなど）は、前面のウィンドウの左上から (40, 80) にする。
/// それも取れなければマウスの位置
pub fn caret_anchor() -> Anchor {
    // SAFETY: 構造体の cbSize を設定する。出力先は有効
    unsafe {
        let fg = GetForegroundWindow();
        if !fg.0.is_null() {
            let tid = GetWindowThreadProcessId(fg, None);
            let mut gti = GUITHREADINFO {
                cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
                ..Default::default()
            };
            if GetGUIThreadInfo(tid, &mut gti).is_ok()
                && !gti.hwndCaret.0.is_null()
                && gti.rcCaret != RECT::default()
            {
                let mut p = POINT {
                    x: gti.rcCaret.left,
                    y: gti.rcCaret.bottom,
                };
                if ClientToScreen(gti.hwndCaret, &mut p).as_bool() {
                    return Anchor {
                        x: p.x,
                        y: p.y,
                        source: "GetGUIThreadInfo.rcCaret",
                    };
                }
            }
            let mut r = RECT::default();
            if GetWindowRect(fg, &mut r).is_ok() {
                return Anchor {
                    x: r.left + 40,
                    y: r.top + 80,
                    source: "foreground window rect (caret unavailable)",
                };
            }
        }
        let mut p = POINT::default();
        let _ = GetCursorPos(&mut p);
        Anchor {
            x: p.x,
            y: p.y,
            source: "cursor",
        }
    }
}

/// フォーカスを奪わない窓にする拡張スタイル
pub fn apply_noactivate_styles(hwnd: HWND) {
    // SAFETY: 自分の窓のスタイルを替えるだけ
    unsafe {
        let cur = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let add = (WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0 | WS_EX_TOPMOST.0) as isize;
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, cur | add);
    }
}

/// 窓を出す前後の前面の窓を比べて、奪われたか記録する
pub fn report_focus(before: &(isize, String), after: &(isize, String)) {
    if before.0 == after.0 {
        println!("RESULT FOCUS unchanged: {}", after.1);
    } else {
        println!(
            "RESULT FOCUS CHANGED: before={} after={}",
            before.1, after.1
        );
    }
}
