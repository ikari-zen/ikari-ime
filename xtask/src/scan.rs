//! `cargo xtask scan-product <ファイル>...`: 配る物（製品のビルドの exe と DLL）の検査（ADR 0067 §4.4・§6）。
//! ファイルのバイト列を読むだけで、道具は要らない（Windows の成果物ができたあとに、CI で流す）。
//!   - 開発用の口の文字列（環境変数の名前、計測の印、デバッグ出力の語）が入っていないこと
//!   - llama.cpp が入っていないこと（LM は別プロセスの `ikari-lm-host` だけが持つ。ADR 0072 §5）
//!   - 測定ツール（`tools/npu-probe`。ADR 0075 §4）の印が入っていないこと（製品の外の道具）
//!   - 通信の DLL（`ws2_32`、`winhttp` など）を取り込んでいないこと（`ikari-data.exe` だけ名前で外す）。PE の import 表の解析は 2 歩目
//!   - 動的な C ランタイムを取り込んでいないこと（`crt-static` が効いていること。取り込む DLL の名前は、
//!     PE の import 表に ASCII で入っているので、名前の有無で見る）
//!
//! 開発用の口を付けたビルド（`dev-knobs`）には流さない（入っていて正しい）。
//!
//! `--stage <フォルダ>` は、インストーラに入れる配置の形（ADR 0067 §3.1）のフォルダを、置き場所ごとの検査で調べる（`stage.rs`）。

mod stage;

use std::path::{Path, PathBuf};

use crate::XtaskError;
use crate::data::run::log;

/// 製品のビルドに入れてはいけない文字列。`check/product.rs` の、ソースの検査の対象と同じ語を持つ
pub const DEV_STRINGS: &[&str] = &[
    "IKARI_USER_DIR",
    "IKARI_TIP_LATENCY",
    "IKARI_LM_GPU",
    "IKARI_LM_LOG",
    "IKARI_LM_LOW_INTEGRITY",
    "latency.on",
    "tip-latency.log",
    "OutputDebugString",
    "ikari-tip: ",
    "--data-dir",
];

/// 製品に入れてはいけない測定ツール（`tools/npu-probe`。ADR 0075 §4）の印。crate の名前（ASCII と UTF-16LE）と、
/// 測定ツールだけが読む OpenVINO の DLL の名前。製品の crate はこの crate に依存しない
pub const PROBE_MARKS: &[&str] = &["ikari-npu-probe", "ikari_npu_probe", "openvino_c"];

/// エンジンに入れてはいけない llama.cpp の印（C の関数名。`ikari-lm-host` には入る。エンジンと TIP の検査にだけ使う）
const LLAMA_MARKS: &[&str] = &["llama_model_load", "ggml_backend"];

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

/// 取り込んではいけない通信の DLL の名前の前置き（ADR 0066 §6.2 の L1。`.dll` で終わる名前のときだけ数える）。
/// 取得の子（`ikari-data.exe`。ADR 0076）だけは、名前で検査から外す
const NETWORK_DLLS: &[&str] = &[
    "ws2_32",
    "wsock32",
    "mswsock",
    "winhttp",
    "wininet",
    "dnsapi",
    "websocket",
    "secur32",
];

/// 通信の DLL の検査から外すファイル名（取得の子だけ。大文字小文字を区別しない）
pub const NETWORK_EXEMPT_FILE: &str = "ikari-data.exe";

/// 製品のビルドの exe と DLL に、開発用の文字列と動的な C ランタイムが無いことを調べる
#[derive(Debug, clap::Args)]
pub struct Args {
    /// 調べるファイル（TIP の DLL、エンジンの exe など）
    #[arg(required_unless_present = "stage")]
    files: Vec<PathBuf>,
    /// インストーラに入れる配置の形（ADR 0067 §3.1）に並べたフォルダ。あるべき物と、置き場所ごとの検査を見る
    #[arg(long, value_name = "フォルダ")]
    stage: Option<PathBuf>,
}

pub fn run(args: Args) -> Result<(), XtaskError> {
    let mut problems = Vec::new();
    if let Some(dir) = &args.stage {
        log(&format!(
            "配置の形 {} を調べる（ADR 0067 §3.1）",
            dir.display()
        ));
        problems.extend(stage::check_stage(dir).map_err(XtaskError)?);
    }
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
    for word in find_llama(bytes) {
        problems.push(format!(
            "{}: llama.cpp の印 `{word}` が入っています（LM は別プロセスだけが持つ。ADR 0072 §5）",
            file.display()
        ));
    }
    for word in find_probe(bytes) {
        problems.push(format!(
            "{}: 測定ツールの印 `{word}` が入っています（tools/npu-probe は製品に入れない。ADR 0075 §4）",
            file.display()
        ));
    }
    for name in find_dynamic_crt(bytes) {
        problems.push(format!(
            "{}: 動的な C ランタイム（`{name}`）を取り込んでいます（crt-static が効いていません）",
            file.display()
        ));
    }
    problems.extend(network_problems(file, bytes));
    problems
}

/// 通信の DLL を取り込んでいる問題の文（取得の子は外す）
pub fn network_problems(file: &Path, bytes: &[u8]) -> Vec<String> {
    let exempt = file.file_name().is_some_and(|n| {
        n.to_string_lossy()
            .eq_ignore_ascii_case(NETWORK_EXEMPT_FILE)
    });
    if exempt {
        return Vec::new();
    }
    find_network_dll(bytes)
        .into_iter()
        .map(|name| {
            format!(
                "{}: 通信の DLL（`{name}`）を取り込んでいます（通信は取得の子だけが持つ。ADR 0066 §6.2・ADR 0076）",
                file.display()
            )
        })
        .collect()
}

/// 開発用の文字列のうち、`bytes` に（ASCII か UTF-16LE で）入っているもの
pub fn find_dev_strings(bytes: &[u8]) -> Vec<&'static str> {
    DEV_STRINGS
        .iter()
        .copied()
        .filter(|word| contains(bytes, word.as_bytes()) || contains(bytes, &utf16le(word)))
        .collect()
}

/// 測定ツールの印のうち、`bytes` に（ASCII か UTF-16LE で）入っているもの
pub fn find_probe(bytes: &[u8]) -> Vec<&'static str> {
    PROBE_MARKS
        .iter()
        .copied()
        .filter(|word| contains(bytes, word.as_bytes()) || contains(bytes, &utf16le(word)))
        .collect()
}

/// llama.cpp の印のうち、`bytes` に入っているもの
pub fn find_llama(bytes: &[u8]) -> Vec<&'static str> {
    LLAMA_MARKS
        .iter()
        .copied()
        .filter(|word| contains(bytes, word.as_bytes()))
        .collect()
}

/// 動的な C ランタイムの DLL 名（`vcruntime140.dll` など）で、`bytes` に（大文字小文字を区別せず）入っているもの。
/// 名前の前置きに続く名前の文字が `.dll` で終わるときだけ数える（ほかの文字列の一部には当たらない）
pub fn find_dynamic_crt(bytes: &[u8]) -> Vec<String> {
    find_dll_names(bytes, DYNAMIC_CRT)
}

/// 通信の DLL の名前（`ws2_32.dll` など）で、`bytes` に入っているもの（探し方は `find_dynamic_crt` と同じ）
pub fn find_network_dll(bytes: &[u8]) -> Vec<String> {
    find_dll_names(bytes, NETWORK_DLLS)
}

fn find_dll_names(bytes: &[u8], prefixes: &[&str]) -> Vec<String> {
    let is_name_byte = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.');
    let mut found: Vec<String> = Vec::new();
    for prefix in prefixes {
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
    fn llama_marks_are_found() {
        assert!(find_llama(b"MZ plain engine").is_empty());
        assert_eq!(
            find_llama(b"..llama_model_load_from_file..ggml_backend_load.."),
            ["llama_model_load", "ggml_backend"]
        );
        let problems = check_bytes(Path::new("e.exe"), b"llama_model_load_from_file");
        assert_eq!(problems.len(), 1);
    }

    #[test]
    fn probe_marks_are_found() {
        assert!(find_probe(b"MZ ikari-engine-host").is_empty());
        let mut bytes = b"..ikari_npu_probe::main..".to_vec();
        bytes.extend(utf16le("openvino_c.dll"));
        assert_eq!(find_probe(&bytes), ["ikari_npu_probe", "openvino_c"]);
        assert_eq!(check_bytes(Path::new("t.dll"), b"ikari-npu-probe").len(), 1);
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
    fn network_dlls_are_found_ignoring_case() {
        let bytes =
            b"\0WS2_32.dll\0winhttp.dll\0WinInet.DLL\0dnsapi.dll\0secur32.dll\0websocket.dll\0";
        assert_eq!(
            find_network_dll(bytes),
            [
                "ws2_32.dll",
                "winhttp.dll",
                "wininet.dll",
                "dnsapi.dll",
                "websocket.dll",
                "secur32.dll"
            ]
        );
        assert!(find_network_dll(b"\0KERNEL32.dll\0 the winhttp word\0 xws2_32.dll\0").is_empty());
    }

    #[test]
    fn only_the_data_child_may_import_network_dlls() {
        let bytes = b"MZ WS2_32.dll secur32.dll";
        assert_eq!(
            check_bytes(Path::new("ikari-engine-host.exe"), bytes).len(),
            2
        );
        assert!(check_bytes(Path::new("stage/IKARI-DATA.exe"), bytes).is_empty());
        assert_eq!(check_bytes(Path::new("data.exe"), bytes).len(), 2);
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
