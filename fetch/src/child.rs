//! 通信する子（Windows の `ikari-data transfer`）の起動の、OS に依存しない部分（ADR 0076 §3.2、§3.4）。
//! Mac でもテストする。OS を呼ぶ部分は `spawn_windows.rs`（起動の作りとコマンドラインは `ikari-sandbox`）。

#![cfg_attr(not(windows), allow(dead_code))]

use std::ffi::OsString;

use crate::args::TransferArgs;

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

/// `transfer` の引数（子の `parse` が読む形）。全部こちらで作る
pub fn transfer_args(args: &TransferArgs) -> Vec<String> {
    let mut v = vec![
        "transfer".to_owned(),
        "--url".to_owned(),
        args.url.clone(),
        "--out".to_owned(),
        args.out.to_string_lossy().into_owned(),
        "--max-size".to_owned(),
        args.max_size.to_string(),
    ];
    if let Some(expect) = args.expect_size {
        v.push("--expect-size".to_owned());
        v.push(expect.to_string());
    }
    v
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

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
            out: PathBuf::from("/w/download.bin"),
            max_size: 10,
            expect_size: Some(7),
        };
        let parsed = crate::args::parse(&transfer_args(&args)).unwrap();
        assert_eq!(parsed, crate::args::Command::Transfer(args));
    }
}
