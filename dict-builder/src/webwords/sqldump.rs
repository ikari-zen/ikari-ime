//! jawiki の SQL ダンプ（mysqldump 形式）から、必要な列だけを読む（ADR 0055 §2.1、§5.1）。
//!
//! 1 行に 1 タプル、行頭が `(` の形だけを読む（`INSERT INTO ... VALUES` の行などは読み飛ばす）。
//! 文字列は `'...'` で、`\'` と `\\`（と `\n` など）のエスケープを戻す。gzip（先頭 2 バイトが 1f 8b）でも平文でも読める。

use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::sync::LazyLock;

use flate2::read::MultiGzDecoder;
use regex::Regex;

use super::WebwordsError;

/// gzip の先頭 2 バイト
const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];

/// 名前空間: 記事
pub const NS_MAIN: i32 = 0;
/// 名前空間: カテゴリ
pub const NS_CATEGORY: i32 = 14;
/// `page_props` の曖昧さ回避の印
pub const PROP_DISAMBIGUATION: &str = "disambiguation";
/// `categorylinks` の種類のうち、記事（ページ）につくもの
const CATEGORY_TYPE_PAGE: &str = "'page'";

/// 文字列の欄: `'` で囲み、中は `'` と `\` 以外か、`\` + 1 文字
const QUOTED: &str = r"'((?:[^'\\]|\\.)*)'";

static PAGE_ROW: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"^\((\d+),(-?\d+),{QUOTED},(\d+),")).expect("PAGE_ROW は固定の正規表現")
});
/// `(数, 数, '文字列'` で始まる行（redirect、linktarget）。続きは `tail`
static TITLE_ROW: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"^\((\d+),(-?\d+),{QUOTED}(.*)$")).expect("TITLE_ROW は固定の正規表現")
});
static PROP_ROW: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"^\((\d+),{QUOTED}")).expect("PROP_ROW は固定の正規表現")
});

/// `page` の 1 行（`page_id, page_namespace, page_title, page_is_redirect`）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageRow {
    pub id: u32,
    pub namespace: i32,
    /// `_` を空白に直した題
    pub title: String,
    pub is_redirect: bool,
}

/// `redirect` の 1 行（`rd_from, rd_namespace, rd_title, rd_interwiki`）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedirectRow {
    pub from: u32,
    pub namespace: i32,
    /// `_` を空白に直した行き先の題
    pub title: String,
    /// 他のウィキへの転送（interwiki が空でない）
    pub interwiki: bool,
}

/// `linktarget` の 1 行（`lt_id, lt_namespace, lt_title`）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkTargetRow {
    pub id: u32,
    pub namespace: i32,
    pub title: String,
}

/// `categorylinks` の 1 行のうち、記事につくもの（`cl_from` と `cl_target_id`）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CategoryLinkRow {
    pub from: u32,
    pub target_id: u32,
}

/// SQL の文字列のエスケープを戻す（`\'` `\\` `\"` `\n` `\t` `\r` `\0`。ほかは `\` を外すだけ）。
pub fn unescape(text: &str) -> String {
    if !text.contains('\\') {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('0') => out.push('\0'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// 題の正規形: エスケープを戻し、`_` を空白にする（redirect の行き先を page の題で引くため）。
pub fn normalize_title(raw: &str) -> String {
    unescape(raw).replace('_', " ")
}

fn parse_u32(text: &str) -> Option<u32> {
    text.parse().ok()
}

pub fn parse_page(line: &str) -> Option<PageRow> {
    let caps = PAGE_ROW.captures(line)?;
    Some(PageRow {
        id: parse_u32(&caps[1])?,
        namespace: caps[2].parse().ok()?,
        title: normalize_title(&caps[3]),
        is_redirect: &caps[4] != "0",
    })
}

pub fn parse_redirect(line: &str) -> Option<RedirectRow> {
    let caps = TITLE_ROW.captures(line)?;
    // 続きは `,'interwiki','fragment')` か `,NULL,NULL)`。interwiki が空（`''` か NULL）なら同じウィキの中
    let tail = &caps[4];
    let local = tail.starts_with(",''") || tail.starts_with(",NULL");
    Some(RedirectRow {
        from: parse_u32(&caps[1])?,
        namespace: caps[2].parse().ok()?,
        title: normalize_title(&caps[3]),
        interwiki: !local,
    })
}

pub fn parse_link_target(line: &str) -> Option<LinkTargetRow> {
    let caps = TITLE_ROW.captures(line)?;
    Some(LinkTargetRow {
        id: parse_u32(&caps[1])?,
        namespace: caps[2].parse().ok()?,
        title: normalize_title(&caps[3]),
    })
}

/// `page_props` の 1 行から (page_id, 属性の名前)
pub fn parse_page_prop(line: &str) -> Option<(u32, String)> {
    let caps = PROP_ROW.captures(line)?;
    Some((parse_u32(&caps[1])?, unescape(&caps[2])))
}

/// `categorylinks` の 1 行。種類が `page` でない（`subcat`、`file`）行は `None`。
/// 列の並び: `cl_from, cl_sortkey, cl_timestamp, cl_sortkey_prefix, cl_type, cl_collation_id, cl_target_id`。
/// 前の列の文字列に `,` が入りうるので、先頭の `cl_from` と、末尾の 3 列だけを読む。
pub fn parse_category_link(line: &str) -> Option<CategoryLinkRow> {
    let body = line.strip_prefix('(')?;
    let from = parse_u32(&body[..body.find(',')?])?;
    let trimmed = body.trim_end_matches([',', ';', ')']);
    let mut tail = trimmed.rsplitn(4, ',');
    let target_id = parse_u32(tail.next()?)?;
    let _collation = tail.next()?;
    let kind = tail.next()?;
    tail.next()?;
    (kind == CATEGORY_TYPE_PAGE).then_some(CategoryLinkRow { from, target_id })
}

/// ダンプの行（行頭が `(`）を 1 行ずつ `each` に渡す。
pub fn for_each_tuple_line(path: &Path, mut each: impl FnMut(&str)) -> Result<(), WebwordsError> {
    let io = |error: std::io::Error| WebwordsError::Io(format!("{}: {error}", path.display()));
    let mut file = File::open(path).map_err(io)?;
    let mut magic = [0u8; 2];
    let read = file.read(&mut magic).map_err(io)?;
    let file = File::open(path).map_err(io)?;
    let mut reader: Box<dyn BufRead> = if read == 2 && magic == GZIP_MAGIC {
        Box::new(BufReader::with_capacity(1 << 20, MultiGzDecoder::new(file)))
    } else {
        Box::new(BufReader::with_capacity(1 << 20, file))
    };
    let mut buf = Vec::new();
    loop {
        buf.clear();
        if reader.read_until(b'\n', &mut buf).map_err(io)? == 0 {
            break;
        }
        if buf.first() != Some(&b'(') {
            continue;
        }
        let line = String::from_utf8_lossy(&buf);
        each(line.trim_end_matches(['\n', '\r']));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unescapes_quotes_and_backslashes() {
        assert_eq!(unescape(r"It\'s"), "It's");
        assert_eq!(unescape(r"a\\b"), r"a\b");
        assert_eq!(unescape(r#"say \"hi\""#), r#"say "hi""#);
        assert_eq!(unescape(r"a\nb"), "a\nb");
        assert_eq!(normalize_title("A_B_C"), "A B C");
    }

    #[test]
    fn parses_page_rows_with_escapes_and_commas() {
        let line =
            r"(10,0,'It\'s_a_(test),_ok\\',1,0,0.5,'20260101000000',NULL,5,6,'wikitext',NULL),";
        let row = parse_page(line).unwrap();
        assert_eq!(row.id, 10);
        assert_eq!(row.namespace, 0);
        assert_eq!(row.title, r"It's a (test), ok\");
        assert!(row.is_redirect);
        let row = parse_page("(11,14,'カテゴリ',0,0,0,'x',NULL,1,2,'w',NULL);").unwrap();
        assert_eq!((row.namespace, row.is_redirect), (14, false));
        assert!(parse_page("INSERT INTO `page` VALUES").is_none());
    }

    #[test]
    fn parses_redirect_and_interwiki() {
        let row = parse_redirect("(5,0,'バージョン_管理','','見出し'),").unwrap();
        assert_eq!((row.from, row.namespace), (5, 0));
        assert_eq!(row.title, "バージョン 管理");
        assert!(!row.interwiki);
        assert!(!parse_redirect("(5,0,'A',NULL,NULL);").unwrap().interwiki);
        assert!(parse_redirect("(5,0,'A','en','');").unwrap().interwiki);
    }

    #[test]
    fn parses_page_props_and_link_targets() {
        assert_eq!(
            parse_page_prop("(7,'disambiguation',NULL,NULL),"),
            Some((7, "disambiguation".to_owned()))
        );
        assert_eq!(
            parse_page_prop("(7,'wikibase_item','Q1',NULL),").unwrap().1,
            "wikibase_item"
        );
        let row = parse_link_target(r"(3,14,'ネコ科_\'x\''),").unwrap();
        assert_eq!((row.id, row.namespace), (3, 14));
        assert_eq!(row.title, "ネコ科 'x'");
    }

    #[test]
    fn parses_category_links_only_for_pages() {
        let line = r"(100,'SORT, KEY\'S','2026-01-01 00:00:00','','page',0,42),";
        assert_eq!(
            parse_category_link(line),
            Some(CategoryLinkRow {
                from: 100,
                target_id: 42
            })
        );
        assert_eq!(
            parse_category_link("(101,'k','t','','page',0,43);"),
            Some(CategoryLinkRow {
                from: 101,
                target_id: 43
            })
        );
        assert!(parse_category_link("(102,'k','t','','subcat',0,44),").is_none());
        assert!(parse_category_link("(102,'k','t','','file',0,44),").is_none());
    }

    #[test]
    fn reads_plain_and_gzip_and_skips_non_tuple_lines() {
        use flate2::Compression;
        use flate2::write::GzEncoder;
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let text = "-- dump\nINSERT INTO `t` VALUES\n(1,'a'),\n(2,'b');\n";
        let plain = dir.path().join("p.sql");
        std::fs::write(&plain, text).unwrap();
        let gz = dir.path().join("p.sql.gz");
        let mut enc = GzEncoder::new(File::create(&gz).unwrap(), Compression::fast());
        enc.write_all(text.as_bytes()).unwrap();
        enc.finish().unwrap();
        for path in [plain, gz] {
            let mut lines = Vec::new();
            for_each_tuple_line(&path, |l| lines.push(l.to_owned())).unwrap();
            assert_eq!(lines, ["(1,'a'),", "(2,'b');"]);
        }
        assert!(for_each_tuple_line(&dir.path().join("none"), |_| {}).is_err());
    }
}
