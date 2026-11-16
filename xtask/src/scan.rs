//! `cargo xtask scan-product <ファイル>...`: 配る物（製品のビルドの exe と DLL）の検査（ADR 0067 §4.4・§6）。
//! ファイルのバイト列を読むだけで、道具は要らない（Windows の成果物ができたあとに、CI で流す）。
//!   - 開発用の口の文字列（環境変数の名前、計測の印、デバッグ出力の語）が入っていないこと
//!   - 動的な C ランタイムを取り込んでいないこと（`crt-static` が効いていること。取り込む DLL の名前は、
//!     PE の import 表に ASCII で入っているので、名前の有無で見る）
//!
//! 開発用の口を付けたビルド（`dev-knobs`）には流さない（入っていて正しい）。

use std::path::{Path, PathBuf};

use crate::XtaskError;
use crate::data::run::log;

/// 製品のビルドに入れてはいけない文字列。`check/product.rs` の、ソースの検査の対象と同じ語を持つ
pub const DEV_STRINGS: &[&str] = &[
    "IKARI_USER_DIR",
    "IKARI_TIP_LATENCY",
    "IKARI_LM_GPU",
    "IKARI_LM_LOG",
    "latency.on",
    "tip-latency.log",
    "OutputDebugString",
    "ikari-tip: ",
    "--data-dir",
];

/// 取り込んではいけない DLL の名前の前置き（大文字小文字を区別しない。`.dll` で終わる名前のときだけ数える）。`crt-static` なら、どれも入らない
const DYNAMIC_CRT: &[&str] = &[
    "vcruntime",
    "msvcp",
    "msvcr",
    "ucrtbase",
    "api-ms-win-crt-",
    "concrt",
    "vcomp",
];

/// 製品のビルドの exe と DLL に、開発用の文字列と動的な C ランタイムが無いことを調べる
#[derive(Debug, clap::Args)]
pub struct Args {
    /// 調べるファイル（TIP の DLL、エンジンの exe など）
    #[arg(required = true)]
    files: Vec<PathBuf>,
}

pub fn run(args: Args) -> Result<(), XtaskError> {
    let mut problems = Vec::new();
    for file in &args.files {
        log(&format!("{} を調べる", file.display()));
        let bytes = std::fs::read(file)
            .map_err(|e| XtaskError(format!("{} を読めません: {e}", file.display())))?;
        problems.extend(check_bytes(file, &bytes));
    }
    if problems.is_empty() {
        return Ok(());
    }
    Err(XtaskError(problems.join("\n")))
}

fn check_bytes(file: &Path, bytes: &[u8]) -> Vec<String> {
    let mut problems = Vec::new();
    for word in find_dev_strings(bytes) {
        problems.push(format!(
            "{}: 開発用の文字列 `{word}` が入っています（feature `dev-knobs` の外に出ています）",
            file.display()
        ));
    }
    for name in find_dynamic_crt(bytes) {
        problems.push(format!(
            "{}: 動的な C ランタイム（`{name}`）を取り込んでいます（crt-static が効いていません）",
            file.display()
        ));
    }
    problems
}

/// 開発用の文字列のうち、`bytes` に（ASCII か UTF-16LE で）入っているもの
pub fn find_dev_strings(bytes: &[u8]) -> Vec<&'static str> {
    DEV_STRINGS
        .iter()
        .copied()
        .filter(|word| contains(bytes, word.as_bytes()) || contains(bytes, &utf16le(word)))
        .collect()
}

/// 動的な C ランタイムの DLL 名（`vcruntime140.dll` など）で、`bytes` に（大文字小文字を区別せず）入っているもの。
/// 名前の前置きに続く名前の文字が `.dll` で終わるときだけ数える（ほかの文字列の一部には当たらない）
pub fn find_dynamic_crt(bytes: &[u8]) -> Vec<String> {
    let is_name_byte = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.');
    let mut found: Vec<String> = Vec::new();
    for prefix in DYNAMIC_CRT {
        let needle = prefix.as_bytes();
        if bytes.len() < needle.len() {
            continue;
        }
        for i in 0..=bytes.len() - needle.len() {
            if !bytes[i..i + needle.len()].eq_ignore_ascii_case(needle) {
                continue;
            }
            // 名前の途中から始まる当たり（`xvcomp...`）は数えない
            if i > 0 && is_name_byte(bytes[i - 1]) {
                continue;
            }
            let run_end = bytes[i..]
                .iter()
                .take(64)
                .position(|&b| !is_name_byte(b))
                .unwrap_or((bytes.len() - i).min(64));
            let token = &bytes[i..i + run_end];
            let lower = String::from_utf8_lossy(token).to_ascii_lowercase();
            if let Some(end) = lower.find(".dll") {
                let name = lower[..end + 4].to_string();
                if !found.contains(&name) {
                    found.push(name);
                }
            }
        }
    }
    found
}

fn utf16le(word: &str) -> Vec<u8> {
    word.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    find(haystack, needle, false)
}

fn find(haystack: &[u8], needle: &[u8], ignore_case: bool) -> bool {
    let Some(&first) = needle.first() else {
        return true;
    };
    let eq = |a: u8, b: u8| {
        if ignore_case {
            a.eq_ignore_ascii_case(&b)
        } else {
            a == b
        }
    };
    if haystack.len() < needle.len() {
        return false;
    }
    let last_start = haystack.len() - needle.len();
    (0..=last_start).any(|i| {
        eq(haystack[i], first)
            && haystack[i..i + needle.len()]
                .iter()
                .zip(needle)
                .all(|(&a, &b)| eq(a, b))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_binary_passes() {
        let bytes = b"MZ\x90\x00 KERNEL32.dll ntdll.dll some text";
        assert!(find_dev_strings(bytes).is_empty());
        assert!(find_dynamic_crt(bytes).is_empty());
    }

    #[test]
    fn ascii_and_wide_dev_strings_are_found() {
        let mut bytes = b"xx IKARI_USER_DIR yy".to_vec();
        bytes.extend(utf16le("latency.on"));
        assert_eq!(find_dev_strings(&bytes), ["IKARI_USER_DIR", "latency.on"]);
    }

    #[test]
    fn dynamic_crt_is_found_ignoring_case() {
        let bytes = b"\0VCRUNTIME140.dll\0api-ms-win-crt-runtime-l1-1-0.dll\0";
        assert_eq!(
            find_dynamic_crt(bytes),
            ["vcruntime140.dll", "api-ms-win-crt-runtime-l1-1-0.dll"]
        );
    }

    #[test]
    fn dll_names_only_not_other_strings_that_contain_the_prefix() {
        // 名前の一部にたまたま当たる文字列や、`.dll` で終わらない語は数えない
        let bytes = b"\0vcompare\0 the msvcrt-like word\0 concrete\0 xvcomp140.dll\0";
        assert!(
            find_dynamic_crt(bytes).is_empty(),
            "{:?}",
            find_dynamic_crt(bytes)
        );
        assert_eq!(find_dynamic_crt(b"\0VCOMP140.DLL\0"), ["vcomp140.dll"]);
        assert_eq!(find_dynamic_crt(b"\0msvcp140_1.dll\0"), ["msvcp140_1.dll"]);
    }

    #[test]
    fn short_haystack_and_boundaries() {
        assert!(!contains(b"abc", b"abcd"));
        assert!(contains(b"abc", b"bc"));
        assert!(contains(b"abc", b""));
    }

    #[test]
    fn problems_name_the_file() {
        let problems = check_bytes(Path::new("a.dll"), b"--data-dir msvcp140.dll");
        assert_eq!(problems.len(), 2);
        assert!(problems.iter().all(|p| p.starts_with("a.dll")));
    }
}
