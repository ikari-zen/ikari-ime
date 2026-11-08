//! 常駐エンジンの RSS（ADR 0034 §3.4）。
//!
//! 以前は外部コマンド `ps` を起動していたが、App Sandbox のエンジンでは `ps` を起動できず、実機の
//! `stat` 行に `rss_kb` が一度も出なかった（#238）。Mach の `task_info` は自プロセスの情報なので
//! Sandbox のなかでも取れる。この crate の unsafe はここだけ（FFI 呼び出し 1 つ）。

/// 自プロセスの RSS（KB）。取れなければ None。見回りのスレッドからだけ呼ぶ
#[cfg(target_os = "macos")]
pub(crate) fn rss_kb() -> Option<u64> {
    use std::mem::MaybeUninit;

    let mut info = MaybeUninit::<libc::mach_task_basic_info>::zeroed();
    let mut count = libc::MACH_TASK_BASIC_INFO_COUNT;
    // SAFETY: `info` は `mach_task_basic_info` 1 つ分の書き込み先で、`count` はその語数。
    // `task_info` は自タスクの情報を `count` 語まで書き、成功（KERN_SUCCESS）のときだけ `info` を読む
    // `mach_task_self` は libc で deprecated（mach2 を勧める）だが、依存を足さずここで足りる
    #[allow(deprecated)]
    let kr = unsafe {
        libc::task_info(
            libc::mach_task_self(),
            libc::MACH_TASK_BASIC_INFO,
            info.as_mut_ptr().cast(),
            &mut count,
        )
    };
    if kr != libc::KERN_SUCCESS {
        return None;
    }
    // SAFETY: 上で成功を確かめた
    let resident = unsafe { info.assume_init() }.resident_size;
    Some(resident / 1024)
}

/// macOS 以外では取らない（`rss_kb` を省く）
#[cfg(not(target_os = "macos"))]
pub(crate) fn rss_kb() -> Option<u64> {
    None
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::rss_kb;

    #[test]
    fn rss_is_positive_on_macos() {
        let kb = rss_kb().expect("task_info は自プロセスで成功する");
        assert!(kb > 0);
    }
}
