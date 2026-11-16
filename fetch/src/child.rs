//! 通信する子（Windows の `ikari-data transfer`）の起動の、OS に依存しない部分（ADR 0076 §3.2、§3.4）。
//! Mac でもテストする。OS を呼ぶ部分は `spawn_windows.rs`。

#![cfg_attr(not(windows), allow(dead_code))]

use std::ffi::OsString;
use std::io::{self, BufRead, Read};
use std::path::Path;

use ikari_resource::fetch_report::MAX_LINE_BYTES;

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

/// 子の出力から読んだ 1 行
#[derive(Debug, PartialEq, Eq)]
pub enum Line {
    /// 改行を除いた 1 行
    Text(String),
    /// 長すぎる行（`MAX_LINE_BYTES` 超）か、UTF-8 でない行。読み飛ばした
    Dropped,
    Eof,
}

/// 子（低い整合性。信じない）の出力を 1 行読む。1 行の長さを `MAX_LINE_BYTES` で打ち切り、メモリを使い切らせない
pub fn read_line_capped(reader: &mut impl BufRead) -> io::Result<Line> {
    // 改行と、上限を超えたことが分かる 1 バイトの分
    let window = MAX_LINE_BYTES as u64 + 2;
    let mut buf = Vec::new();
    let n = reader.by_ref().take(window).read_until(b'\n', &mut buf)?;
    if n == 0 {
        return Ok(Line::Eof);
    }
    if buf.last() != Some(&b'\n') && n as u64 == window {
        // 長すぎる。次の改行（か終わり）まで読み捨てる
        let mut sink = Vec::new();
        loop {
            sink.clear();
            let m = reader.by_ref().take(window).read_until(b'\n', &mut sink)?;
            if m == 0 || sink.last() == Some(&b'\n') {
                break;
            }
        }
        return Ok(Line::Dropped);
    }
    if buf.last() == Some(&b'\n') {
        buf.pop();
    }
    match String::from_utf8(buf) {
        Ok(text) if text.len() <= MAX_LINE_BYTES => Ok(Line::Text(text)),
        _ => Ok(Line::Dropped),
    }
}

const QUOTE: u16 = b'"' as u16;
const BACKSLASH: u16 = b'\\' as u16;
const SPACE: u16 = b' ' as u16;
const NUL: u16 = 0;

/// 実行ファイルと引数から、終端の NUL つきのコマンドライン（UTF-16）を作る。すべて引用符で囲み、`"` と、その前と
/// 閉じの前の `\` は、`CommandLineToArgvW` で元に戻る形にする（`lm` の子の起動と同じ規則）。
/// 実行ファイルのパスに `"` や NUL があれば `None`
pub fn command_line(program: &Path, args: &[String]) -> Option<Vec<u16>> {
    let program: Vec<u16> = program
        .as_os_str()
        .to_string_lossy()
        .encode_utf16()
        .collect();
    if program.contains(&QUOTE) || program.contains(&NUL) {
        return None;
    }
    let mut out = vec![QUOTE];
    out.extend_from_slice(&program);
    out.push(QUOTE);
    for arg in args {
        let arg: Vec<u16> = arg.encode_utf16().collect();
        if arg.contains(&NUL) {
            return None;
        }
        out.push(SPACE);
        push_quoted(&mut out, &arg);
    }
    out.push(NUL);
    Some(out)
}

fn push_quoted(out: &mut Vec<u16>, arg: &[u16]) {
    out.push(QUOTE);
    let mut backslashes = 0usize;
    for &c in arg {
        if c == BACKSLASH {
            backslashes += 1;
            continue;
        }
        let repeat = if c == QUOTE {
            backslashes * 2 + 1
        } else {
            backslashes
        };
        out.extend(std::iter::repeat_n(BACKSLASH, repeat));
        out.push(c);
        backslashes = 0;
    }
    out.extend(std::iter::repeat_n(BACKSLASH, backslashes * 2));
    out.push(QUOTE);
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

    fn read_all(input: &[u8]) -> Vec<Line> {
        let mut reader = io::BufReader::new(input);
        let mut lines = Vec::new();
        loop {
            match read_line_capped(&mut reader).unwrap() {
                Line::Eof => return lines,
                line => lines.push(line),
            }
        }
    }

    #[test]
    fn reads_lines_and_drops_overlong_or_invalid_ones() {
        let long = "x".repeat(MAX_LINE_BYTES + 1);
        let ok_max = "y".repeat(MAX_LINE_BYTES);
        let mut input = Vec::new();
        input.extend_from_slice(b"progress 1 2\n");
        input.extend_from_slice(long.as_bytes());
        input.extend_from_slice(b"\n");
        input.extend_from_slice(ok_max.as_bytes());
        input.extend_from_slice(b"\n\xff\xfe\n");
        input.extend_from_slice(b"last without newline");
        assert_eq!(
            read_all(&input),
            [
                Line::Text("progress 1 2".into()),
                Line::Dropped,
                Line::Text(ok_max),
                Line::Dropped,
                Line::Text("last without newline".into()),
            ]
        );
    }

    #[test]
    fn an_endless_line_does_not_grow_memory() {
        // 改行のない 1MiB でも、読み終えて 1 つの Dropped になる
        let input = vec![b'a'; 1 << 20];
        assert_eq!(read_all(&input), [Line::Dropped]);
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

    fn line(program: &str, args: &[&str]) -> String {
        let args: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
        let mut out = command_line(Path::new(program), &args).unwrap();
        assert_eq!(out.pop(), Some(NUL));
        String::from_utf16(&out).unwrap()
    }

    #[test]
    fn command_line_quotes_every_word() {
        assert_eq!(
            line(
                r"C:\Program Files\ikari-ime\ikari-data.exe",
                &["transfer", "--out", r"C:\Users\a b\x.bin"]
            ),
            r#""C:\Program Files\ikari-ime\ikari-data.exe" "transfer" "--out" "C:\Users\a b\x.bin""#
        );
        assert_eq!(line("p", &[r"a\"]), r#""p" "a\\""#);
        assert_eq!(line("p", &[r#"a"b"#]), r#""p" "a\"b""#);
        assert_eq!(line("p", &[r#"a\"b"#]), r#""p" "a\\\"b""#);
    }

    #[test]
    fn command_line_refuses_quotes_in_the_program_and_nul_anywhere() {
        assert!(command_line(Path::new("a\"b"), &[]).is_none());
        assert!(command_line(Path::new("p"), &["a\0b".to_owned()]).is_none());
    }
}
