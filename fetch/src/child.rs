//! 通信する子（Windows の `ikari-data transfer`）の起動の、OS に依存しない部分（ADR 0076 §3.2、§3.4、ADR 0078 §3.1）。
//! Mac でもテストする。OS を呼ぶ部分は `spawn_windows.rs`（起動の作りとコマンドラインは `ikari-sandbox`）。

#![cfg_attr(not(windows), allow(dead_code))]

use std::ffi::OsString;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::args::TransferArgs;

/// 書く先のハンドルの値を子に知らせる引数（ADR 0078 §3.1 の 4。値は十進）。`spawn_windows` が末尾に足す
pub const OUT_HANDLE_FLAG: &str = "--out-handle";

/// 子に渡す環境変数（ADR 0076 §3.4）。プロキシの変数と、OS が動くのに要るものだけ。ほかは消す。
/// Windows の環境変数の名前は大文字と小文字を区別しないので、比べるときは大文字にそろえる。
const PASS_ENV: &[&str] = &[
    "ALL_PROXY",
    "HTTPS_PROXY",
    "HTTP_PROXY",
    "NO_PROXY",
    "SYSTEMROOT",
    "TEMP",
    "TMP",
];

/// 整合性のレベル（`SECURITY_MANDATORY_*_RID`）。低は 0x1000、中は 0x2000
const MEDIUM_INTEGRITY_RID: u32 = 0x2000;

/// 子が低い整合性か。取れなかった（`None`）ときは、低いとは言わない（安全な側）
pub fn is_low_integrity(rid: Option<u32>) -> bool {
    rid.is_some_and(|rid| rid < MEDIUM_INTEGRITY_RID)
}

/// 親の環境から、子に渡すものだけを選ぶ。名前は元のまま
pub fn filter_env(
    vars: impl IntoIterator<Item = (OsString, OsString)>,
) -> Vec<(OsString, OsString)> {
    vars.into_iter()
        .filter(|(name, _)| {
            name.to_str()
                .is_some_and(|n| PASS_ENV.contains(&n.to_ascii_uppercase().as_str()))
        })
        .collect()
}

/// `transfer` の引数（子の `parse` が読む形）。全部こちらで作る。書く先（`OUT_HANDLE_FLAG`）は、ハンドルの写しを
/// 作る `spawn_windows` が末尾に足す（場所は渡さない。ADR 0078 §3.1）
pub fn transfer_args(args: &TransferArgs) -> Vec<String> {
    let mut v = vec![
        "transfer".to_owned(),
        "--url".to_owned(),
        args.url.clone(),
        "--max-size".to_owned(),
        args.max_size.to_string(),
    ];
    if let Some(expect) = args.expect_size {
        v.push("--expect-size".to_owned());
        v.push(expect.to_string());
    }
    v
}

/// `transfer_args` に、書く先のハンドルの値を足す
pub fn with_out_handle(mut args: Vec<String>, handle: u64) -> Vec<String> {
    args.push(OUT_HANDLE_FLAG.to_owned());
    args.push(handle.to_string());
    args
}

/// 親の、書く先の大きさの見張りの結果（ADR 0078 §3.1 の 6）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeWatch {
    /// 上限を超えないうちに、見張りを終えた（`done`）
    Within,
    /// 上限を超えたので、子を止めた
    TooLarge,
}

/// 書く先の大きさ（`len`。親のハンドルで読む）を `interval` ごとに見て、`limit` を超えたら `stop_child` を呼んで
/// `TooLarge`。`done` が立てば `Within`（立てた側は、見張りのスレッドを `unpark` すると早く終わる）。
/// 大きさを読めなかった回は、超えていないとみなして続ける（落としたあとの数え直しで受ける）
pub fn watch_size(
    len: &dyn Fn() -> io::Result<u64>,
    limit: u64,
    interval: Duration,
    stop_child: &dyn Fn(),
    done: &AtomicBool,
) -> SizeWatch {
    loop {
        if len().is_ok_and(|n| n > limit) {
            stop_child();
            return SizeWatch::TooLarge;
        }
        if done.load(Ordering::Acquire) {
            return SizeWatch::Within;
        }
        std::thread::park_timeout(interval);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::sync::atomic::AtomicU64;

    use super::*;

    fn os(s: &str) -> OsString {
        OsString::from(s)
    }

    #[test]
    fn only_proxy_and_system_variables_are_passed() {
        let env = vec![
            (os("HTTPS_PROXY"), os("http://p:3128")),
            (os("https_proxy"), os("http://p:3128")),
            (os("No_Proxy"), os("localhost")),
            (os("SystemRoot"), os(r"C:\Windows")),
            (os("TEMP"), os(r"C:\t")),
            (os("PATH"), os("x")),
            (os("CURL_CA_BUNDLE"), os("x")),
            (os("SSLKEYLOGFILE"), os("x")),
            (os("USERPROFILE"), os("x")),
        ];
        let kept: Vec<String> = filter_env(env)
            .into_iter()
            .map(|(k, _)| k.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            kept,
            [
                "HTTPS_PROXY",
                "https_proxy",
                "No_Proxy",
                "SystemRoot",
                "TEMP"
            ]
        );
    }

    #[test]
    fn integrity_low_only() {
        assert!(is_low_integrity(Some(0x1000)));
        assert!(is_low_integrity(Some(0)));
        assert!(!is_low_integrity(Some(0x2000)));
        assert!(!is_low_integrity(Some(0x3000)));
        assert!(!is_low_integrity(None));
    }

    #[test]
    fn transfer_args_round_trip_through_the_parser() {
        let args = TransferArgs {
            url: "https://example.com/m".into(),
            max_size: 10,
            expect_size: Some(7),
        };
        let built = transfer_args(&args);
        assert!(
            !built.iter().any(|a| a == "--out"),
            "場所を渡さない: {built:?}"
        );
        // 書く先が無ければ、子は読まない
        assert!(crate::args::parse(&built).is_err());
        let parsed = crate::args::parse(&with_out_handle(built, 0x1a4)).unwrap();
        assert_eq!(
            parsed,
            crate::args::Command::Transfer(crate::args::TransferCommand {
                args,
                out_handle: 0x1a4,
            })
        );
    }

    #[test]
    fn the_watch_stops_the_child_once_the_file_is_over_the_limit() {
        let size = AtomicU64::new(0);
        let stopped = Cell::new(0);
        let done = AtomicBool::new(false);
        let reads = Cell::new(0u32);
        let len = || {
            reads.set(reads.get() + 1);
            // 3 回目に上限を超える
            if reads.get() == 3 {
                size.store(11, Ordering::Relaxed);
            }
            Ok(size.load(Ordering::Relaxed))
        };
        let result = watch_size(
            &len,
            10,
            Duration::from_millis(1),
            &|| stopped.set(stopped.get() + 1),
            &done,
        );
        assert_eq!(result, SizeWatch::TooLarge);
        assert_eq!(stopped.get(), 1);
        assert_eq!(reads.get(), 3);
    }

    #[test]
    fn the_watch_ends_quietly_when_done_and_unreadable_sizes_do_not_stop_the_child() {
        let done = AtomicBool::new(true);
        let stopped = Cell::new(false);
        let at_limit = watch_size(
            &|| Ok(10),
            10,
            Duration::from_secs(60),
            &|| stopped.set(true),
            &done,
        );
        assert_eq!(at_limit, SizeWatch::Within, "上限ちょうどは超えていない");
        let unreadable = watch_size(
            &|| Err(io::Error::other("x")),
            10,
            Duration::from_secs(60),
            &|| stopped.set(true),
            &done,
        );
        assert_eq!(unreadable, SizeWatch::Within);
        assert!(!stopped.get());
    }

    #[test]
    fn unpark_wakes_the_watch_before_the_interval() {
        let done = AtomicBool::new(false);
        let started = std::time::Instant::now();
        std::thread::scope(|s| {
            let watcher =
                s.spawn(|| watch_size(&|| Ok(0), 10, Duration::from_secs(60), &|| {}, &done));
            done.store(true, Ordering::Release);
            watcher.thread().unpark();
            assert_eq!(watcher.join().unwrap(), SizeWatch::Within);
        });
        assert!(started.elapsed() < Duration::from_secs(30));
    }
}
