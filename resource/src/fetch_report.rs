//! `ikari-data` の標準出力の行（取得の報告）。型と、読み書き（ADR 0076 §8）。
//!
//! 核（設定画面）と `ikari-data` が共有する。純粋で、OS と通信に依存しない。
//!
//! ```text
//! progress <受けたバイト> <全体のバイト|->
//! done <束の名前> <sha256>
//! latest <版>
//! error <理由の語>
//! ```
//!
//! 1 行は [`MAX_LINE_BYTES`] まで。読む側（[`ReportReader`]）は、知らない行、長すぎる行、終わりの行
//! （`done`・`latest`・`error`）のあとの行を捨てる。

use std::io::{self, BufRead, Read};

use crate::manifest::check_file_name;
use crate::sha256::is_sha256_hex;

/// 1 行の長さの上限（改行を含まない）
pub const MAX_LINE_BYTES: usize = 256;
/// 全体の大きさが分からないときの印
const UNKNOWN_TOTAL: &str = "-";

/// 終了コード: 成功
pub const EXIT_OK: i32 = 0;
/// 終了コード: `error` の行を出した
pub const EXIT_ERROR: i32 = 1;
/// 終了コード: 引数が不正（行は出さない）
pub const EXIT_USAGE: i32 = 2;

/// 失敗の理由の語（ADR 0076 §8.2）。文は読む側（核）が作る
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchErrorCode {
    Cancelled,
    Network,
    Tls,
    HttpStatus,
    Stalled,
    TooLarge,
    Checksum,
    DiskFull,
    BadUrl,
    BadReply,
    Tool,
    Busy,
    Refused,
    Internal,
}

impl FetchErrorCode {
    pub const ALL: [FetchErrorCode; 14] = [
        Self::Cancelled,
        Self::Network,
        Self::Tls,
        Self::HttpStatus,
        Self::Stalled,
        Self::TooLarge,
        Self::Checksum,
        Self::DiskFull,
        Self::BadUrl,
        Self::BadReply,
        Self::Tool,
        Self::Busy,
        Self::Refused,
        Self::Internal,
    ];

    /// 行に書く語
    pub fn word(self) -> &'static str {
        match self {
            Self::Cancelled => "cancelled",
            Self::Network => "network",
            Self::Tls => "tls",
            Self::HttpStatus => "http-status",
            Self::Stalled => "stalled",
            Self::TooLarge => "too-large",
            Self::Checksum => "checksum",
            Self::DiskFull => "disk-full",
            Self::BadUrl => "bad-url",
            Self::BadReply => "bad-reply",
            Self::Tool => "tool",
            Self::Busy => "busy",
            Self::Refused => "refused",
            Self::Internal => "internal",
        }
    }

    pub fn from_word(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.word() == word)
    }
}

/// 報告の 1 行
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Report {
    Progress { received: u64, total: Option<u64> },
    Done { name: String, sha256: String },
    Latest { version: String },
    Error(FetchErrorCode),
}

impl Report {
    /// 終わりの行か（これのあとの行は読まない）
    pub fn is_final(&self) -> bool {
        !matches!(self, Self::Progress { .. })
    }

    /// 行（改行なし）。`Done` の名前、`Latest` の版は、読み戻せる形のものだけが来る前提（`parse_line` が同じ検査をする）
    pub fn to_line(&self) -> String {
        match self {
            Self::Progress { received, total } => match total {
                Some(total) => format!("progress {received} {total}"),
                None => format!("progress {received} {UNKNOWN_TOTAL}"),
            },
            Self::Done { name, sha256 } => format!("done {name} {sha256}"),
            Self::Latest { version } => format!("latest {version}"),
            Self::Error(code) => format!("error {}", code.word()),
        }
    }
}

/// 1 行（改行なし）を読む。読めない行（知らない、形が違う、長すぎる）は `None`
pub fn parse_line(line: &str) -> Option<Report> {
    if line.len() > MAX_LINE_BYTES {
        return None;
    }
    let mut parts = line.split(' ');
    let kind = parts.next()?;
    let report = match kind {
        "progress" => {
            let received = parse_number(parts.next()?)?;
            let total = match parts.next()? {
                UNKNOWN_TOTAL => None,
                s => Some(parse_number(s)?),
            };
            Report::Progress { received, total }
        }
        "done" => {
            let name = parts.next()?;
            let sha256 = parts.next()?;
            if check_file_name(name).is_err() || !is_sha256_hex(sha256) {
                return None;
            }
            Report::Done {
                name: name.to_owned(),
                sha256: sha256.to_owned(),
            }
        }
        "latest" => {
            let version = parts.next()?;
            if !is_version(version) {
                return None;
            }
            Report::Latest {
                version: version.to_owned(),
            }
        }
        "error" => Report::Error(FetchErrorCode::from_word(parts.next()?)?),
        _ => return None,
    };
    // 余分な語は、形が違う行として捨てる
    parts.next().is_none().then_some(report)
}

/// 10 進の数字だけ（符号、空白、`+` を拒む）
fn parse_number(s: &str) -> Option<u64> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// `数.数.数`（版の確認で読むタグの `v` を除いた形。ADR 0076 §6）
pub fn is_version(s: &str) -> bool {
    let mut count = 0;
    for part in s.split('.') {
        // 桁が多すぎる数は拒む（表示と比較で扱える範囲に収める）
        if part.is_empty()
            || part.len() > crate::app_version::MAX_VERSION_PART_DIGITS
            || parse_number(part).is_none()
        {
            return false;
        }
        count += 1;
    }
    count == 3
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

/// 行を 1 本ずつ流し込み、受け入れる報告だけを返す（終わりの行のあとは、何も返さない）
#[derive(Debug, Default)]
pub struct ReportReader {
    finished: bool,
}

impl ReportReader {
    pub fn new() -> Self {
        Self::default()
    }

    /// 終わりの行を受けたか
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    pub fn feed(&mut self, line: &str) -> Option<Report> {
        if self.finished {
            return None;
        }
        let report = parse_line(line.trim_end_matches(['\r', '\n']))?;
        self.finished = report.is_final();
        Some(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    #[test]
    fn every_report_round_trips() {
        let mut reports = vec![
            Report::Progress {
                received: 0,
                total: Some(10),
            },
            Report::Progress {
                received: 5,
                total: None,
            },
            Report::Done {
                name: "user-ba7816bf8f01".into(),
                sha256: SHA.into(),
            },
            Report::Latest {
                version: "0.3.1".into(),
            },
        ];
        reports.extend(FetchErrorCode::ALL.map(Report::Error));
        for r in reports {
            assert_eq!(parse_line(&r.to_line()), Some(r.clone()), "{r:?}");
        }
    }

    #[test]
    fn malformed_lines_are_dropped() {
        for line in [
            "",
            "progress",
            "progress 1",
            "progress -1 5",
            "progress 1 5 6",
            "progress +1 5",
            "progress 99999999999999999999999 5",
            "done ../x ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            "done name SHORT",
            "done name",
            "latest v0.3.1",
            "latest 0.3",
            "latest 0.3.1.2",
            "latest 0.3.x",
            "error unknown-word",
            "error",
            "error network extra",
            "unknown 1 2",
            " progress 1 2",
        ] {
            assert_eq!(parse_line(line), None, "{line:?}");
        }
    }

    #[test]
    fn long_lines_are_dropped() {
        let ok = format!("done {} {SHA}", "a".repeat(100));
        assert!(parse_line(&ok).is_some());
        let long = format!("done {} {SHA}", "a".repeat(MAX_LINE_BYTES));
        assert_eq!(parse_line(&long), None);
    }

    #[test]
    fn reader_ignores_everything_after_a_final_line() {
        let mut r = ReportReader::new();
        assert!(r.feed("garbage").is_none());
        assert!(r.feed("progress 1 2\n").is_some());
        assert!(!r.is_finished());
        assert_eq!(
            r.feed("error network\r\n"),
            Some(Report::Error(FetchErrorCode::Network))
        );
        assert!(r.is_finished());
        assert!(r.feed("progress 2 2").is_none());
        assert!(r.feed(&format!("done x {SHA}")).is_none());
    }

    #[test]
    fn error_words_are_unique_and_known() {
        let mut words: Vec<_> = FetchErrorCode::ALL.map(FetchErrorCode::word).to_vec();
        words.sort_unstable();
        words.dedup();
        assert_eq!(words.len(), FetchErrorCode::ALL.len());
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
}
