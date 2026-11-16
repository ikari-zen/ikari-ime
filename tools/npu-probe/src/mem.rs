//! プロセスの作業メモリの最大（ADR 0075 §4.4）。Windows は Private Bytes の最大（`PeakPagefileUsage`。
//! コミットした私的なメモリの最大）。macOS は最大の常駐（`ru_maxrss`。比べるための参考）。

#[cfg(windows)]
pub fn peak_bytes() -> Option<u64> {
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    // SAFETY: すべて 0 の値は、この構造体として正しい（OS が埋める）
    let mut counters: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
    let size = size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    counters.cb = size;
    // SAFETY: 自分のプロセスの擬似ハンドルと、大きさを合わせた書き込み先を渡すだけ
    let ok = unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, size) };
    (ok != 0).then_some(counters.PeakPagefileUsage as u64)
}

#[cfg(unix)]
pub fn peak_bytes() -> Option<u64> {
    // SAFETY: すべて 0 の値は、この構造体として正しい（OS が埋める）
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    // SAFETY: 自分のプロセスの使用量を、この関数の中の変数に書かせるだけ
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) } != 0 {
        return None;
    }
    let max = u64::try_from(usage.ru_maxrss).ok()?;
    // macOS はバイト、Linux は KiB
    Some(if cfg!(target_os = "macos") {
        max
    } else {
        max * 1024
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn peak_is_positive() {
        assert!(super::peak_bytes().is_some_and(|b| b > 0));
    }
}
