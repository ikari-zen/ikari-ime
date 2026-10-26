//! プロセスの最大 RSS（`getrusage`。ADR 0002 §13.3）。
//!
//! eval は依存を増やさない方針なので、libc の `getrusage` を直接宣言する。unix 以外では None。

/// 最大常駐メモリ（バイト）。測れなければ None。
pub fn max_rss_bytes() -> Option<u64> {
    imp::max_rss_bytes()
}

#[cfg(unix)]
mod imp {
    /// `RUSAGE_SELF`（macOS と Linux で 0）
    const RUSAGE_SELF: i32 = 0;
    /// `ru_maxrss` の単位。macOS はバイト、Linux は KiB
    #[cfg(target_os = "macos")]
    const MAXRSS_UNIT: u64 = 1;
    #[cfg(not(target_os = "macos"))]
    const MAXRSS_UNIT: u64 = 1024;

    /// `struct rusage`（64 ビットの macOS と Linux で、同じ大きさの long の並び）。
    /// 先頭の 2 つの `timeval` は 16 バイトずつ、その後に `ru_maxrss` と残り 13 個の long が続く。
    #[repr(C)]
    struct RUsage {
        ru_utime: [i64; 2],
        ru_stime: [i64; 2],
        ru_maxrss: i64,
        rest: [i64; 13],
    }

    unsafe extern "C" {
        fn getrusage(who: i32, usage: *mut RUsage) -> i32;
    }

    pub fn max_rss_bytes() -> Option<u64> {
        if cfg!(not(target_pointer_width = "64")) {
            return None;
        }
        let mut usage = RUsage {
            ru_utime: [0; 2],
            ru_stime: [0; 2],
            ru_maxrss: 0,
            rest: [0; 13],
        };
        // SAFETY: `usage` は `struct rusage` と同じ大きさの有効な書き込み先で、getrusage は
        // その範囲の外に書かない。
        let rc = unsafe { getrusage(RUSAGE_SELF, &mut usage) };
        if rc != 0 {
            return None;
        }
        u64::try_from(usage.ru_maxrss)
            .ok()
            .map(|v| v.saturating_mul(MAXRSS_UNIT))
    }
}

#[cfg(not(unix))]
mod imp {
    pub fn max_rss_bytes() -> Option<u64> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn reports_a_plausible_value() {
        let rss = max_rss_bytes().expect("getrusage works on unix");
        // テストのプロセスは 1MiB より大きく、1TiB より小さい
        assert!(rss > 1024 * 1024, "{rss}");
        assert!(rss < (1u64 << 40), "{rss}");
    }

    #[cfg(unix)]
    #[test]
    fn grows_when_memory_is_touched() {
        // ru_maxrss は今の RSS ではなく最大値。同じプロセスの別のテストが先に使って解放していると、
        // 64MiB を触っても最大値は伸びきらない。最大値そのものより大きく触れば、今の RSS に関係なく伸びる。
        let before = max_rss_bytes().unwrap();
        let len = usize::try_from(before).unwrap() + 64 * 1024 * 1024;
        let mut big = vec![0u8; len];
        for i in (0..big.len()).step_by(4096) {
            big[i] = 1;
        }
        std::hint::black_box(&big);
        let after = max_rss_bytes().unwrap();
        assert!(after >= before + 32 * 1024 * 1024, "{before} -> {after}");
    }
}
