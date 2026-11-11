//! LLM が出力した JSONL の検証（ADR 0002 §11.2、NFR-24）。
//!
//! 取り込みファイルは信用しない入力として扱う。ヘッダが正しくなければファイル全体を拒否し、
//! 行ごとの不正（長すぎる行、壊れた JSON、読みの文字種など）は、その行を捨てて理由ごとに数える。

use std::collections::BTreeMap;
use std::fmt;

use ikari_dict_format::reading::normalize_key;
use serde_json::Value;

pub const FORMAT_NAME: &str = "ikari-ingest";
pub const FORMAT_VERSION: u64 = 1;
/// 受け付けるプロンプトの版（同梱のプロンプトの版。NFR-22）
pub const KNOWN_PROMPT_VERSIONS: [&str; 1] = ["ingest-prompt-v1"];
/// 1 行の最大バイト数
pub const MAX_LINE_BYTES: usize = 4096;
/// 表記の最大文字数
pub const MAX_SURFACE_CHARS: usize = 400;
/// ファイルの最大バイト数
pub const MAX_FILE_BYTES: u64 = 50 * 1024 * 1024;

const TYPE_HEADER: &str = "header";
const TYPE_SENTENCE: &str = "sentence";
const TYPE_DOC_BREAK: &str = "doc_break";
const CODE_FENCE: &str = "```";

/// 取り込んだ 1 文。`reading` は `normalize_key` 済み。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sentence {
    pub surface: String,
    pub reading: String,
    /// 文書の番号（`doc_break` で増える。0 始まり）
    pub doc: u32,
}

/// 行を捨てた理由
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DropReason {
    LineTooLong,
    InvalidJson,
    UnknownType,
    MissingField,
    EmptyField,
    SurfaceTooLong,
    ControlChars,
    BadReading,
    DuplicateHeader,
    /// 辞書で読みを再現できなかった（アライメントの失敗）
    NotAlignable,
}

impl DropReason {
    pub fn label(self) -> &'static str {
        match self {
            DropReason::LineTooLong => "1行が長すぎる",
            DropReason::InvalidJson => "JSON として読めない",
            DropReason::UnknownType => "未知の type",
            DropReason::MissingField => "必須の項目がない",
            DropReason::EmptyField => "表記か読みが空",
            DropReason::SurfaceTooLong => "表記が長すぎる",
            DropReason::ControlChars => "制御文字を含む",
            DropReason::BadReading => "読みに使えない文字がある",
            DropReason::DuplicateHeader => "ヘッダの重複",
            DropReason::NotAlignable => "辞書で読みを再現できない",
        }
    }
}

impl fmt::Display for DropReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// 理由ごとの件数
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DropCounts(BTreeMap<DropReason, u32>);

impl DropCounts {
    pub fn add(&mut self, reason: DropReason) {
        let c = self.0.entry(reason).or_default();
        *c = c.saturating_add(1);
    }

    pub fn total(&self) -> u32 {
        self.0.values().fold(0u32, |a, c| a.saturating_add(*c))
    }

    pub fn get(&self, reason: DropReason) -> u32 {
        self.0.get(&reason).copied().unwrap_or(0)
    }

    pub fn iter(&self) -> impl Iterator<Item = (DropReason, u32)> + '_ {
        self.0.iter().map(|(r, c)| (*r, *c))
    }
}

/// ファイル全体を拒否する理由
#[derive(Debug, PartialEq, Eq)]
pub enum SchemaError {
    NotUtf8,
    TooLarge { bytes: u64 },
    NoHeader,
    BadHeader(String),
    UnknownVersion(u64),
    UnknownPromptVersion(String),
}

impl fmt::Display for SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SchemaError::NotUtf8 => write!(f, "UTF-8 のテキストではありません"),
            SchemaError::TooLarge { bytes } => write!(
                f,
                "ファイルが大きすぎます（{bytes} バイト。上限は {MAX_FILE_BYTES} バイト）"
            ),
            SchemaError::NoHeader => write!(f, "先頭にヘッダ行（type=header）がありません"),
            SchemaError::BadHeader(m) => write!(f, "ヘッダが不正です: {m}"),
            SchemaError::UnknownVersion(v) => {
                write!(f, "未知のバージョンです: {v}（対応: {FORMAT_VERSION}）")
            }
            SchemaError::UnknownPromptVersion(v) => write!(
                f,
                "未知のプロンプトの版です: {v}（対応: {}）",
                KNOWN_PROMPT_VERSIONS.join(", ")
            ),
        }
    }
}

impl std::error::Error for SchemaError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Parsed {
    pub prompt_version: String,
    pub sentences: Vec<Sentence>,
    pub drops: DropCounts,
}

/// 文字が「読みの中でそのまま書いてよい記号・英数字」か（かな、漢字以外）。
fn is_literal_reading_char(c: char) -> bool {
    !c.is_control() && !c.is_whitespace() && !is_kana(c) && !is_cjk_ideograph(c)
}

pub(crate) fn is_kana(c: char) -> bool {
    matches!(c, '\u{3041}'..='\u{3096}' | '\u{309D}'..='\u{309E}' | '\u{30A1}'..='\u{30FA}'
        | '\u{30FD}'..='\u{30FE}' | '\u{30FC}')
}

pub(crate) fn is_cjk_ideograph(c: char) -> bool {
    matches!(c, '\u{3400}'..='\u{4DBF}' | '\u{4E00}'..='\u{9FFF}' | '\u{F900}'..='\u{FAFF}'
        | '\u{3005}' | '\u{3007}' | '\u{303B}' | '\u{20000}'..='\u{2FA1F}')
}

fn is_hiragana_reading_char(c: char) -> bool {
    matches!(c, '\u{3041}'..='\u{3096}' | '\u{309D}'..='\u{309E}' | '\u{30FC}')
}

/// 読みを正規化し、使える文字だけで構成されているか確かめる。
/// 使える文字: ひらがな、ー、および表記に現れる記号・英数字。
fn normalize_reading(reading: &str, surface: &str) -> Option<String> {
    let normalized = normalize_key(reading.trim());
    let ok = normalized.chars().all(|c| {
        is_hiragana_reading_char(c) || (is_literal_reading_char(c) && surface.contains(c))
    });
    ok.then_some(normalized)
}

fn parse_sentence(v: &Value, doc: u32) -> Result<Sentence, DropReason> {
    let get = |k: &str| match v.get(k) {
        Some(Value::String(s)) => Ok(s.as_str()),
        _ => Err(DropReason::MissingField),
    };
    let surface = get("surface")?.trim();
    let reading = get("reading")?;
    if surface.is_empty() || reading.trim().is_empty() {
        return Err(DropReason::EmptyField);
    }
    if surface.chars().count() > MAX_SURFACE_CHARS {
        return Err(DropReason::SurfaceTooLong);
    }
    if surface.chars().any(char::is_control) || reading.chars().any(char::is_control) {
        return Err(DropReason::ControlChars);
    }
    let reading = normalize_reading(reading, surface).ok_or(DropReason::BadReading)?;
    Ok(Sentence {
        surface: surface.to_string(),
        reading,
        doc,
    })
}

fn parse_header(v: &Value) -> Result<String, SchemaError> {
    if v.get("format").and_then(Value::as_str) != Some(FORMAT_NAME) {
        return Err(SchemaError::BadHeader(format!(
            "format が `{FORMAT_NAME}` ではありません"
        )));
    }
    let version = v
        .get("version")
        .and_then(Value::as_u64)
        .ok_or_else(|| SchemaError::BadHeader("version がありません".to_string()))?;
    if version != FORMAT_VERSION {
        return Err(SchemaError::UnknownVersion(version));
    }
    let prompt = v
        .get("prompt_version")
        .and_then(Value::as_str)
        .ok_or_else(|| SchemaError::BadHeader("prompt_version がありません".to_string()))?;
    if !KNOWN_PROMPT_VERSIONS.contains(&prompt) {
        return Err(SchemaError::UnknownPromptVersion(prompt.to_string()));
    }
    Ok(prompt.to_string())
}

/// JSONL を検証して読む。ファイルのサイズ上限もここで確かめる。
pub fn parse_jsonl(bytes: &[u8]) -> Result<Parsed, SchemaError> {
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(SchemaError::TooLarge {
            bytes: bytes.len() as u64,
        });
    }
    let text = std::str::from_utf8(bytes).map_err(|_| SchemaError::NotUtf8)?;
    let text = ikari_textfile::strip_bom(text);

    let mut prompt_version: Option<String> = None;
    let mut sentences = Vec::new();
    let mut drops = DropCounts::default();
    let mut doc = 0u32;
    for line in text.lines() {
        let trimmed = line.trim();
        // チャット画面からのコピーで付きがちなコードフェンスと空行は無視する
        if trimmed.is_empty() || trimmed.starts_with(CODE_FENCE) {
            continue;
        }
        if prompt_version.is_none() {
            // 最初の行はヘッダでなければならない
            let v: Value = serde_json::from_str(trimmed)
                .map_err(|_| SchemaError::NoHeader)
                .and_then(|v: Value| {
                    if v.get("type").and_then(Value::as_str) == Some(TYPE_HEADER) {
                        Ok(v)
                    } else {
                        Err(SchemaError::NoHeader)
                    }
                })?;
            prompt_version = Some(parse_header(&v)?);
            continue;
        }
        if line.len() > MAX_LINE_BYTES {
            drops.add(DropReason::LineTooLong);
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(trimmed) else {
            drops.add(DropReason::InvalidJson);
            continue;
        };
        match v.get("type").and_then(Value::as_str) {
            Some(TYPE_SENTENCE) => match parse_sentence(&v, doc) {
                Ok(s) => sentences.push(s),
                Err(reason) => drops.add(reason),
            },
            Some(TYPE_DOC_BREAK) => doc = doc.saturating_add(1),
            Some(TYPE_HEADER) => drops.add(DropReason::DuplicateHeader),
            Some(_) => drops.add(DropReason::UnknownType),
            None if v.is_object() => drops.add(DropReason::MissingField),
            None => drops.add(DropReason::InvalidJson),
        }
    }
    let prompt_version = prompt_version.ok_or(SchemaError::NoHeader)?;
    Ok(Parsed {
        prompt_version,
        sentences,
        drops,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str = r#"{"type":"header","format":"ikari-ingest","version":1,"prompt_version":"ingest-prompt-v1"}"#;

    fn parse(body: &str) -> Result<Parsed, SchemaError> {
        parse_jsonl(format!("{HEADER}\n{body}").as_bytes())
    }

    fn sentence(surface: &str, reading: &str) -> String {
        serde_json::json!({"type":"sentence","surface":surface,"reading":reading}).to_string()
    }

    #[test]
    fn valid_file_is_parsed_with_doc_numbers() {
        let body = [
            sentence("品詞を指定する。", "ひんしをしていする。"),
            r#"{"type":"doc_break"}"#.to_string(),
            sentence("APIを呼ぶ", "APIをよぶ"),
            sentence("ユーザー", "ユーザー"),
        ]
        .join("\n");
        let p = parse(&body).unwrap();
        assert_eq!(p.prompt_version, "ingest-prompt-v1");
        assert_eq!(p.drops.total(), 0);
        assert_eq!(p.sentences.len(), 3);
        assert_eq!(p.sentences[0].doc, 0);
        assert_eq!(p.sentences[1].doc, 1);
        // 読みは正規化される（カタカナ -> ひらがな）
        assert_eq!(p.sentences[2].reading, "ゆーざー");
        assert_eq!(p.sentences[1].reading, "APIをよぶ");
    }

    #[test]
    fn header_problems_reject_the_whole_file() {
        assert_eq!(parse_jsonl(b"").unwrap_err(), SchemaError::NoHeader);
        assert_eq!(
            parse_jsonl(sentence("あ", "あ").as_bytes()).unwrap_err(),
            SchemaError::NoHeader
        );
        assert_eq!(parse_jsonl(b"not json").unwrap_err(), SchemaError::NoHeader);
        let bad = [
            (
                r#"{"type":"header","format":"other","version":1,"prompt_version":"ingest-prompt-v1"}"#,
                "format",
            ),
            (
                r#"{"type":"header","format":"ikari-ingest","version":2,"prompt_version":"ingest-prompt-v1"}"#,
                "version",
            ),
            (
                r#"{"type":"header","format":"ikari-ingest","version":1,"prompt_version":"ingest-prompt-v9"}"#,
                "prompt",
            ),
            (
                r#"{"type":"header","format":"ikari-ingest","version":1}"#,
                "missing",
            ),
        ];
        for (h, what) in bad {
            assert!(parse_jsonl(h.as_bytes()).is_err(), "{what}");
        }
        assert_eq!(
            parse_jsonl(bad[1].0.as_bytes()).unwrap_err(),
            SchemaError::UnknownVersion(2)
        );
        assert_eq!(
            parse_jsonl(bad[2].0.as_bytes()).unwrap_err(),
            SchemaError::UnknownPromptVersion("ingest-prompt-v9".to_string())
        );
        assert_eq!(
            parse_jsonl(&[0xff, 0xfe, b'{']).unwrap_err(),
            SchemaError::NotUtf8
        );
    }

    #[test]
    fn bad_lines_are_dropped_and_counted_by_reason() {
        let long_surface = "あ".repeat(MAX_SURFACE_CHARS + 1);
        let long_line = sentence(&"あ".repeat(MAX_LINE_BYTES), "あ");
        let body = [
            sentence("ok", "ok"),
            "{broken".to_string(),
            "42".to_string(),
            r#"{"type":"mystery"}"#.to_string(),
            r#"{"surface":"x"}"#.to_string(),
            r#"{"type":"sentence","surface":"x"}"#.to_string(),
            r#"{"type":"sentence","surface":1,"reading":"x"}"#.to_string(),
            sentence("  ", "あ"),
            sentence("あ", ""),
            sentence(&long_surface, "あ"),
            long_line,
            sentence("a\u{0}b", "あ"),
            // 読みに漢字、または表記にない記号
            sentence("漢字", "漢字"),
            sentence("かな", "かな!"),
            HEADER.to_string(),
            sentence("かな", "かな"),
        ]
        .join("\n");
        let p = parse(&body).unwrap();
        let sentences: Vec<&str> = p.sentences.iter().map(|s| s.surface.as_str()).collect();
        assert_eq!(sentences, ["ok", "かな"]);
        assert_eq!(p.drops.get(DropReason::InvalidJson), 2);
        assert_eq!(p.drops.get(DropReason::UnknownType), 1);
        assert_eq!(p.drops.get(DropReason::MissingField), 3);
        assert_eq!(p.drops.get(DropReason::EmptyField), 2);
        assert_eq!(p.drops.get(DropReason::SurfaceTooLong), 1);
        assert_eq!(p.drops.get(DropReason::LineTooLong), 1);
        assert_eq!(p.drops.get(DropReason::ControlChars), 1);
        assert_eq!(p.drops.get(DropReason::BadReading), 2);
        assert_eq!(p.drops.get(DropReason::DuplicateHeader), 1);
        assert_eq!(p.drops.total(), 14);
    }

    #[test]
    fn code_fences_bom_and_crlf_are_tolerated() {
        let text = format!(
            "\u{feff}```jsonl\r\n{HEADER}\r\n{}\r\n```\r\n",
            sentence("かな", "かな")
        );
        let p = parse_jsonl(text.as_bytes()).unwrap();
        assert_eq!(p.sentences.len(), 1);
        assert_eq!(p.drops.total(), 0);
    }

    #[test]
    fn lf_crlf_and_bom_files_read_the_same() {
        let lf = format!("{HEADER}\n{}\n", sentence("かな", "かな"));
        for (v, text) in ikari_textfile::testing::read_variants(&lf) {
            let p = parse_jsonl(text.as_bytes()).unwrap_or_else(|e| panic!("{v:?}: {e:?}"));
            assert_eq!(p.sentences.len(), 1, "{v:?}");
            assert_eq!(p.drops.total(), 0, "{v:?}");
        }
    }

    #[test]
    fn oversized_files_are_rejected_without_parsing() {
        let big = vec![b' '; (MAX_FILE_BYTES + 1) as usize];
        assert!(matches!(
            parse_jsonl(&big),
            Err(SchemaError::TooLarge { .. })
        ));
    }

    #[test]
    fn huge_documents_do_not_panic() {
        let mut body = String::new();
        for i in 0..2000 {
            body.push_str(&sentence(&format!("文{i}"), "ぶん"));
            body.push('\n');
        }
        // 読みと表記が合わない（数字が読みにない）だけで、検証は通る
        assert_eq!(parse(&body).unwrap().sentences.len(), 2000);
    }
}
