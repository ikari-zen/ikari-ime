//! Windows の子のコマンドライン（UTF-16）を作る。OS に依存しないので、Mac でもテストする（`lm` の `host/os/cmdline.rs` と同じ規則）。
//!
//! 規則（ADR 0067 §4.3）: 実行ファイルのパスも引数も、すべて引用符で囲む。引数の中の `"` と、その前の `\` は、
//! Microsoft の C 実行時ライブラリ（`CommandLineToArgvW` と同じ）の読み方で元に戻る形にする。

use std::io;

const QUOTE: u16 = b'"' as u16;
const BACKSLASH: u16 = b'\\' as u16;
const SPACE: u16 = b' ' as u16;
const NUL: u16 = 0;

/// `program` と `args`（どちらも NUL を含まない UTF-16）から、終端の NUL つきのコマンドラインを作る。
/// 実行ファイルのパスに `"` があれば失敗にする（Windows のパスには使えない文字。先頭の語は `\"` で戻せない）
pub fn command_line(program: &[u16], args: &[Vec<u16>]) -> io::Result<Vec<u16>> {
    if program.contains(&QUOTE) {
        return Err(invalid("program path has a quote"));
    }
    if program.contains(&NUL) || args.iter().any(|a| a.contains(&NUL)) {
        return Err(invalid("nul in command line"));
    }
    let mut out = Vec::with_capacity(program.len() + 2);
    out.push(QUOTE);
    out.extend_from_slice(program);
    out.push(QUOTE);
    for arg in args {
        out.push(SPACE);
        push_quoted(&mut out, arg);
    }
    out.push(NUL);
    Ok(out)
}

/// 引数 1 つを引用符で囲んで足す。`"` の直前の `\` の並びと、閉じの `"` の直前の `\` の並びは 2 倍にする
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

fn invalid(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, what)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    fn line(program: &str, args: &[&str]) -> String {
        let args: Vec<Vec<u16>> = args.iter().map(|a| wide(a)).collect();
        let mut out = command_line(&wide(program), &args).unwrap();
        assert_eq!(out.pop(), Some(NUL));
        String::from_utf16(&out).unwrap()
    }

    #[test]
    fn every_word_is_quoted() {
        assert_eq!(
            line(
                r"C:\Program Files\ikari\ikari-lm-host.exe",
                &["--model", r"C:\Users\a b\models\m.gguf", "--gpu", "on"]
            ),
            r#""C:\Program Files\ikari\ikari-lm-host.exe" "--model" "C:\Users\a b\models\m.gguf" "--gpu" "on""#
        );
    }

    #[test]
    fn backslashes_before_quotes_and_the_end_are_doubled() {
        assert_eq!(line("p", &[r"a\"]), r#""p" "a\\""#);
        assert_eq!(line("p", &[r#"a"b"#]), r#""p" "a\"b""#);
        assert_eq!(line("p", &[r#"a\"b"#]), r#""p" "a\\\"b""#);
        assert_eq!(line("p", &[r"a\b"]), r#""p" "a\b""#);
        assert_eq!(line("p", &[""]), r#""p" """#);
    }

    #[test]
    fn quotes_in_the_program_and_nul_anywhere_are_refused() {
        assert!(command_line(&wide(r#"C:\a"b.exe"#), &[]).is_err());
        assert!(command_line(&wide("p\0"), &[]).is_err());
        assert!(command_line(&wide("p"), &[wide("a\0b")]).is_err());
    }
}
