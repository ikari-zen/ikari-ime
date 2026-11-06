//! `webwords collect`（レーン A。ADR 0055 §5.1 の 2）。本文から連なりを数える（ADR 0039 §2.2）。
//!
//! PoC（`eval/scripts/webwords_collect.py`、PR #213）の定義をそのまま Rust に移したもの。
//!
//! - 対象は名前空間 0 で、リダイレクトでない記事。題が「一覧」で終わる記事と、曖昧さ回避のテンプレートを持つ記事は除く
//!   （名前の羅列で df が歪むため。ADR 0055 §5.1 の 2）。
//! - ウィキの記法は正規表現で除く（形態素解析はしない）。リンクは表示される文字だけを残す。
//! - 連なり: U+30A1〜U+30FA と `ー`（U+30FC）。`・` と `＝` は連なりに含めない（区切りになる）。
//! - tf: 連なりの出現回数。df: 連なりが出た記事の数。cue: 連なりの直後に [`CUES`] のどれかが続いた回数（F4b）。
//! - 出力は `表記<TAB>tf<TAB>df<TAB>cue`、表記のコードポイント順。1 行目は `#` で始まる説明の行（読むときは無視する）。

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use bzip2::read::MultiBzDecoder;
use regex::Regex;

use super::{CollectedRow, WebwordsError};

#[derive(Debug, Clone)]
pub struct CollectConfig {
    /// pages-articles の bz2（分割の全ファイル）
    pub pages_articles: Vec<PathBuf>,
    /// 収集の一覧（`CollectedRow` の行）の出力先
    pub out: PathBuf,
    /// 出力する df の下限
    pub min_df: u32,
}

/// 出力の 1 行目の先頭の欄
const HEADER_MAGIC: &str = "#ikari-webwords-collect";
/// 固有名詞の手がかり（直後に続いたら cue に数える。ADR 0039 F4b）
const CUES: [&str; 19] = [
    "さん", "氏", "社", "州", "市", "科", "属", "目", "県", "郡", "町", "村", "区", "川", "山",
    "島", "党", "軍", "王",
];
/// 記事の終わりの印。ダンプを読みながら、これで 1 記事ずつに区切る
const PAGE_END: &[u8] = b"</page>";
/// ダンプを読む 1 回の大きさ
const READ_CHUNK: usize = 1 << 24;
/// テンプレートの入れ子をほどく最大の回数
const TEMPLATE_PASSES: usize = 12;
/// 名前空間 0（標準。記事）
const MAIN_NAMESPACE: &str = "0";
const LIST_TITLE_SUFFIX: &str = "一覧";
const DISAMBIGUATION_MARKERS: [&str; 4] = ["{{Aimai", "{{aimai", "{{Disambig", "{{曖昧さ回避"];
/// 中身ごと除く HTML 風のタグ
const BLOCK_TAGS: [&str; 13] = [
    "ref",
    "math",
    "code",
    "syntaxhighlight",
    "source",
    "pre",
    "nowiki",
    "gallery",
    "score",
    "timeline",
    "imagemap",
    "poem",
    "chem",
];
const BLOCK_TAGS_EXTRA: [&str; 1] = ["templatedata"];

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("正規表現は固定の文字列")
}

static RUN: LazyLock<Regex> = LazyLock::new(|| re("[\u{30A1}-\u{30FA}\u{30FC}]+"));
static TITLE: LazyLock<Regex> = LazyLock::new(|| re(r"<title>(.*?)</title>"));
static NS: LazyLock<Regex> = LazyLock::new(|| re(r"<ns>(\d+)</ns>"));
static TEXT: LazyLock<Regex> = LazyLock::new(|| re(r"(?s)<text[^>]*>(.*?)</text>"));
static COMMENT: LazyLock<Regex> = LazyLock::new(|| re(r"(?s)<!--.*?-->"));
// Rust の regex は後方参照を持たないので、タグごとに 1 本ずつ作る
static BLOCKS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    BLOCK_TAGS
        .iter()
        .chain(BLOCK_TAGS_EXTRA.iter())
        .map(|tag| re(&format!(r"(?is)<{tag}\b[^>]*?(?:/>|>.*?</{tag}\s*>)")))
        .collect()
});
static TABLE: LazyLock<Regex> = LazyLock::new(|| re(r"(?s)\{\|.*?\|\}"));
static TEMPLATE: LazyLock<Regex> = LazyLock::new(|| re(r"\{\{[^{}]*\}\}"));
static FILE_LINK: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"(?i)\[\[(?:ファイル|File|Image|画像|カテゴリ|Category):[^\[\]]*(?:\[\[[^\[\]]*\]\][^\[\]]*)*\]\]",
    )
});
static LINK_PIPE: LazyLock<Regex> = LazyLock::new(|| re(r"\[\[[^\[\]|]*\|([^\[\]]*)\]\]"));
static LINK: LazyLock<Regex> = LazyLock::new(|| re(r"\[\[([^\[\]]*)\]\]"));
static EXTERNAL: LazyLock<Regex> = LazyLock::new(|| re(r"\[(?:https?:)?//[^\s\]]+\s*([^\]]*)\]"));
static URL: LazyLock<Regex> = LazyLock::new(|| re(r"https?://\S+"));
static TAG: LazyLock<Regex> = LazyLock::new(|| re(r"</?[A-Za-z][^>]*>"));
static ENTITY: LazyLock<Regex> = LazyLock::new(|| re(r"&[#A-Za-z0-9]+;"));
static MAGIC_WORD: LazyLock<Regex> = LazyLock::new(|| re(r"__[A-Z]+__"));
static CHAR_REFERENCE: LazyLock<Regex> =
    LazyLock::new(|| re(r"&(#[0-9]+|#[xX][0-9A-Fa-f]+|lt|gt|amp|quot|apos|nbsp);"));

/// 連なりごとの数え上げ
#[derive(Debug, Default)]
struct Counts {
    /// 表記 -> (tf, df, cue)
    words: HashMap<String, (u64, u64, u64)>,
    articles: u64,
    skipped: u64,
}

/// XML の文字参照（`&lt;` や `&#12450;`）を戻す。1 回だけ（`&amp;lt;` は `&lt;` になる）。
/// 知らない参照は残し、あとの [`ENTITY`] で空白にする。
fn unescape(text: &str) -> String {
    CHAR_REFERENCE
        .replace_all(text, |caps: &regex::Captures| {
            let name = &caps[1];
            let decoded = match name {
                "lt" => Some('<'),
                "gt" => Some('>'),
                "amp" => Some('&'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some('\u{a0}'),
                _ => {
                    let number = match name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok(),
                        None => name[1..].parse::<u32>().ok(),
                    };
                    number.and_then(char::from_u32)
                }
            };
            decoded.map_or_else(|| caps[0].to_owned(), String::from)
        })
        .into_owned()
}

/// ウィキの記法を除いて、表示される文字だけを残す。
fn strip_markup(raw: &str) -> String {
    let mut text = unescape(raw);
    text = COMMENT.replace_all(&text, "").into_owned();
    for block in BLOCKS.iter() {
        text = block.replace_all(&text, "").into_owned();
    }
    text = TABLE.replace_all(&text, "").into_owned();
    for _ in 0..TEMPLATE_PASSES {
        let next = TEMPLATE.replace_all(&text, "");
        if next == text {
            break;
        }
        text = next.into_owned();
    }
    text = FILE_LINK.replace_all(&text, "").into_owned();
    text = LINK_PIPE.replace_all(&text, "${1}").into_owned();
    text = LINK.replace_all(&text, "${1}").into_owned();
    text = EXTERNAL.replace_all(&text, "${1}").into_owned();
    text = URL.replace_all(&text, "").into_owned();
    text = TAG.replace_all(&text, "").into_owned();
    text = ENTITY.replace_all(&text, " ").into_owned();
    text = MAGIC_WORD.replace_all(&text, "").into_owned();
    text.replace("'''", "").replace("''", "")
}

impl Counts {
    /// 1 記事（`<page>…</page>`）を数える。対象外なら何もしない。
    fn add_page(&mut self, page: &str) {
        let Some(ns) = NS.captures(page) else { return };
        if &ns[1] != MAIN_NAMESPACE || page.contains("<redirect") {
            return;
        }
        let title = TITLE
            .captures(page)
            .map_or("", |c| c.get(1).map_or("", |m| m.as_str()));
        if title.ends_with(LIST_TITLE_SUFFIX) {
            self.skipped += 1;
            return;
        }
        let Some(text) = TEXT.captures(page) else {
            return;
        };
        let raw = &text[1];
        if DISAMBIGUATION_MARKERS.iter().any(|m| raw.contains(m)) {
            self.skipped += 1;
            return;
        }
        let text = strip_markup(raw);
        self.articles += 1;
        let mut seen: HashSet<&str> = HashSet::new();
        for found in RUN.find_iter(&text) {
            let word = found.as_str();
            let after = &text[found.end()..];
            let entry = match self.words.get_mut(word) {
                Some(entry) => entry,
                None => self.words.entry(word.to_owned()).or_default(),
            };
            entry.0 += 1;
            if CUES.iter().any(|cue| after.starts_with(cue)) {
                entry.2 += 1;
            }
            seen.insert(word);
        }
        for word in seen {
            if let Some(entry) = self.words.get_mut(word) {
                entry.1 += 1;
            }
        }
    }

    /// bz2 の XML を 1 つ読んで数える。
    fn add_dump(&mut self, reader: impl Read) -> std::io::Result<()> {
        let mut reader = reader;
        let mut buf: Vec<u8> = Vec::new();
        let mut chunk = vec![0u8; READ_CHUNK];
        loop {
            let n = reader.read(&mut chunk)?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            let mut start = 0;
            while let Some(offset) = find(&buf[start..], PAGE_END) {
                let end = start + offset + PAGE_END.len();
                self.add_page(&String::from_utf8_lossy(&buf[start..end]));
                start = end;
            }
            buf.drain(..start);
        }
        Ok(())
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn io_error(path: &Path, error: std::io::Error) -> WebwordsError {
    WebwordsError::Io(format!("{}: {error}", path.display()))
}

/// 収集を行い、出力した行数を返す。
pub fn collect(config: &CollectConfig) -> Result<u64, WebwordsError> {
    if config.pages_articles.is_empty() {
        return Err(WebwordsError::Format(
            "pages-articles のファイルが 1 つもありません".into(),
        ));
    }
    let mut counts = Counts::default();
    for path in &config.pages_articles {
        let file = File::open(path).map_err(|e| io_error(path, e))?;
        counts
            .add_dump(MultiBzDecoder::new(file))
            .map_err(|e| io_error(path, e))?;
    }
    write_list(&counts, config)
}

fn write_list(counts: &Counts, config: &CollectConfig) -> Result<u64, WebwordsError> {
    let mut rows: Vec<CollectedRow> = counts
        .words
        .iter()
        .filter(|(_, (_, df, _))| *df >= u64::from(config.min_df))
        .map(|(surface, &(tf, df, cue))| CollectedRow {
            surface: surface.clone(),
            tf,
            df,
            cue,
        })
        .collect();
    rows.sort_by(|a, b| a.surface.cmp(&b.surface));
    let file = File::create(&config.out).map_err(|e| io_error(&config.out, e))?;
    let mut out = BufWriter::new(file);
    let write = |out: &mut BufWriter<File>| -> std::io::Result<()> {
        writeln!(
            out,
            "{HEADER_MAGIC}\tfiles={}\tarticles={}\tskipped={}\tmin_df={}",
            config.pages_articles.len(),
            counts.articles,
            counts.skipped,
            config.min_df
        )?;
        for row in &rows {
            writeln!(out, "{}", row.format())?;
        }
        out.flush()
    };
    write(&mut out).map_err(|e| io_error(&config.out, e))?;
    Ok(rows.len() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(title: &str, ns: u32, extra: &str, text: &str) -> String {
        format!(
            "<page>\n<title>{title}</title>\n<ns>{ns}</ns>\n<id>1</id>\n{extra}<revision><text xml:space=\"preserve\">{text}</text></revision>\n</page>\n"
        )
    }

    fn counts_of(pages: &[String]) -> Counts {
        let xml = format!(
            "<mediawiki>\n<siteinfo></siteinfo>\n{}</mediawiki>",
            pages.concat()
        );
        let mut counts = Counts::default();
        counts.add_dump(xml.as_bytes()).unwrap();
        counts
    }

    fn row(counts: &Counts, word: &str) -> (u64, u64, u64) {
        counts.words.get(word).copied().unwrap_or_default()
    }

    #[test]
    fn counts_tf_df_and_cue() {
        let counts = counts_of(&[
            page(
                "A",
                0,
                "",
                "ソロアルバムを出した。ソロアルバム。ヤマダ氏とヤマダさんと",
            ),
            page("B", 0, "", "ソロアルバムの話。"),
        ]);
        // tf は出現回数、df は記事数、cue は直後が手がかりの回数
        assert_eq!(row(&counts, "ソロアルバム"), (3, 2, 0));
        assert_eq!(row(&counts, "ヤマダ"), (2, 1, 2));
        assert_eq!(counts.articles, 2);
    }

    #[test]
    fn run_shape_and_separators() {
        let counts = counts_of(&[page("A", 0, "", "ジョン・スミス＝ブラウン、ラーメンヴ")]);
        assert_eq!(row(&counts, "ジョン").1, 1);
        assert_eq!(row(&counts, "スミス").1, 1);
        assert_eq!(row(&counts, "ブラウン").1, 1);
        assert_eq!(row(&counts, "ラーメンヴ").1, 1);
        assert_eq!(counts.words.len(), 4);
    }

    #[test]
    fn skips_other_namespaces_redirects_lists_and_disambiguation() {
        let counts = counts_of(&[
            page("ノート", 1, "", "トーク"),
            page("R", 0, "<redirect title=\"X\" />\n", "リダイレクト"),
            page("日本の駅の一覧", 0, "", "イチラン"),
            page("曖昧", 0, "", "{{aimai}}アイマイ"),
            page("本物", 0, "", "ホンモノ"),
        ]);
        assert_eq!(counts.articles, 1);
        assert_eq!(counts.skipped, 2);
        assert_eq!(
            counts.words.keys().map(String::as_str).collect::<Vec<_>>(),
            ["ホンモノ"]
        );
    }

    #[test]
    fn strips_markup() {
        let text = "{{Infobox|名前=テンプレート}}[[ファイル:a.png|サムネイル|キャプション]]\
            [[リンク先|表示]]と[[ノーマル]]<ref>レフ</ref><!-- コメント -->\
            [https://example.com/ ガイブ]https://example.com/ユーアールエル \
            {|\n|テーブル\n|}&lt;b&gt;エスケープ&lt;/b&gt;'''ボールド'''&#12450;&amp;nbsp;";
        let out = strip_markup(text);
        for gone in [
            "テンプレート",
            "キャプション",
            "レフ",
            "コメント",
            "ユーアールエル",
            "テーブル",
        ] {
            assert!(!out.contains(gone), "{gone} が残った: {out}");
        }
        for kept in ["表示", "ノーマル", "ガイブ", "エスケープ", "ボールド", "ア"]
        {
            assert!(out.contains(kept), "{kept} が消えた: {out}");
        }
        assert!(!out.contains("リンク先"));
    }

    #[test]
    fn nested_templates_are_removed() {
        assert_eq!(strip_markup("{{a|{{b|{{c|ネスト}}}}}}あと"), "あと");
    }

    #[test]
    fn collect_merges_multiple_bz2_files() {
        use bzip2::{Compression, write::BzEncoder};
        let dir = tempfile::tempdir().unwrap();
        let mut paths = Vec::new();
        let files = [
            vec![page(
                "A",
                0,
                "",
                "ソロアルバム。ソロアルバム。バージョニング",
            )],
            vec![
                page("B", 0, "", "ソロアルバム"),
                page("C", 0, "", "バージョニング"),
            ],
        ];
        for (i, pages) in files.iter().enumerate() {
            let path = dir.path().join(format!("dump{i}.xml.bz2"));
            let xml = format!("<mediawiki>{}</mediawiki>", pages.concat());
            let mut enc = BzEncoder::new(File::create(&path).unwrap(), Compression::fast());
            enc.write_all(xml.as_bytes()).unwrap();
            enc.finish().unwrap();
            paths.push(path);
        }
        let out = dir.path().join("out.tsv");
        let written = collect(&CollectConfig {
            pages_articles: paths,
            out: out.clone(),
            min_df: 2,
        })
        .unwrap();
        assert_eq!(written, 2);
        let text = std::fs::read_to_string(&out).unwrap();
        let mut lines = text.lines();
        assert!(
            lines
                .next()
                .unwrap()
                .starts_with("#ikari-webwords-collect\tfiles=2\tarticles=3")
        );
        let rows: Vec<CollectedRow> = lines.map(|l| CollectedRow::parse(l).unwrap()).collect();
        // 並びはコードポイント順（ソ U+30BD < バ U+30D0）
        assert_eq!(
            rows,
            [
                CollectedRow {
                    surface: "ソロアルバム".into(),
                    tf: 3,
                    df: 2,
                    cue: 0
                },
                CollectedRow {
                    surface: "バージョニング".into(),
                    tf: 2,
                    df: 2,
                    cue: 0
                },
            ]
        );
    }

    #[test]
    fn collect_rejects_empty_input_and_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = CollectConfig {
            pages_articles: vec![],
            out: dir.path().join("o.tsv"),
            min_df: 1,
        };
        assert!(collect(&config).is_err());
        config.pages_articles = vec![dir.path().join("none.bz2")];
        assert!(matches!(collect(&config), Err(WebwordsError::Io(_))));
    }
}
