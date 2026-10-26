//! 取り込み（`ikari ingest`）が使う 2 つのファイル（ADR 0002 §11.3, §11.4）。
//!
//! - 取り込み元の台帳 `ingest_sources.tsv`（NFR-26。ファイルは複製せず、パスで参照する）
//! - 登録候補 `ingest_candidates.tsv`（辞書で読みを再現できない語。ユーザーが確認したものだけを辞書へ）
//!
//! 取り込む処理そのものは `ikari-import` にある。ここは保存の形式だけを持つ。

use std::collections::BTreeMap;
use std::path::Path;

use ikari_dict_format::reading::is_indexable;

use super::UserDataError;
use super::util::{
    FIELD_SEPARATOR, check_schema, has_cell_breaker, parse_error, parse_header, read_optional,
    write_atomic,
};

pub const SOURCES_SCHEMA: u32 = 1;
pub const CANDIDATES_SCHEMA: u32 = 1;
pub const INGEST_SOURCES_SECTION_NAME: &str = "ingest_sources";

const SOURCES_HEADER_NAME: &str = "#ikari-ingest-sources";
const SOURCES_COLUMNS: &str =
    "id\tpath\timported_at\tfnv64\tprompt_version\tsentences_ok\tsentences_dropped";
const SOURCES_COLUMN_COUNT: usize = 7;
const CANDIDATES_HEADER_NAME: &str = "#ikari-ingest-candidates";
const CANDIDATES_COLUMNS: &str = "source\treading\tsurface\tcount";
const CANDIDATES_COLUMN_COUNT: usize = 4;
const SECONDS_PER_DAY: u64 = 86_400;

/// 取り込み元 1 件。`imported_at` は UNIX 秒。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IngestSource {
    pub id: u32,
    pub path: String,
    pub imported_at: u64,
    /// ファイル内容の FNV-1a 64（変更の検出用）
    pub fnv64: u64,
    pub prompt_version: String,
    pub sentences_ok: u32,
    pub sentences_dropped: u32,
}

#[derive(Clone, Debug, Default)]
pub struct IngestSources {
    sources: Vec<IngestSource>,
}

impl IngestSources {
    pub fn open(path: &Path) -> Result<Self, UserDataError> {
        match read_optional(path)? {
            Some(text) => Self::parse_in(&text, path),
            None => Ok(Self::default()),
        }
    }

    /// 中身を検証して読む（エクスポートのインポートの事前検証にも使う）。
    pub fn parse(text: &str) -> Result<Self, UserDataError> {
        Self::parse_in(text, Path::new(INGEST_SOURCES_SECTION_NAME))
    }

    fn parse_in(text: &str, file: &Path) -> Result<Self, UserDataError> {
        let mut lines = text.lines();
        let header = lines
            .next()
            .ok_or_else(|| parse_error(file, 1, "空のファイルです"))?;
        let kv = parse_header(header, SOURCES_HEADER_NAME).ok_or_else(|| {
            parse_error(
                file,
                1,
                format!("ヘッダが `{SOURCES_HEADER_NAME}` ではありません"),
            )
        })?;
        check_schema(file, &kv, SOURCES_SCHEMA)?;
        if lines.next() != Some(SOURCES_COLUMNS) {
            return Err(parse_error(file, 2, "列の見出し行が不正です"));
        }
        let mut sources: Vec<IngestSource> = Vec::new();
        for (i, line) in lines.enumerate() {
            let n = i + 3;
            if line.is_empty() {
                continue;
            }
            let f: Vec<&str> = line.split(FIELD_SEPARATOR).collect();
            if f.len() != SOURCES_COLUMN_COUNT {
                return Err(parse_error(
                    file,
                    n,
                    format!("列数が {SOURCES_COLUMN_COUNT} ではありません"),
                ));
            }
            let num = |s: &str, what: &str| {
                s.parse::<u64>()
                    .map_err(|_| parse_error(file, n, format!("{what} が整数ではありません")))
            };
            let id = u32::try_from(num(f[0], "id")?)
                .map_err(|_| parse_error(file, n, "id が大きすぎます"))?;
            let fnv64 = u64::from_str_radix(f[3], 16)
                .map_err(|_| parse_error(file, n, "fnv64 が16進の数ではありません"))?;
            if f[1].is_empty() || f[4].is_empty() {
                return Err(parse_error(file, n, "path か prompt_version が空です"));
            }
            let to_u32 = |v: u64, what: &str| {
                u32::try_from(v).map_err(|_| parse_error(file, n, format!("{what} が大きすぎます")))
            };
            let s = IngestSource {
                id,
                path: f[1].to_string(),
                imported_at: num(f[2], "imported_at")?,
                fnv64,
                prompt_version: f[4].to_string(),
                sentences_ok: to_u32(num(f[5], "sentences_ok")?, "sentences_ok")?,
                sentences_dropped: to_u32(num(f[6], "sentences_dropped")?, "sentences_dropped")?,
            };
            if sources.iter().any(|x| x.id == s.id) {
                return Err(parse_error(file, n, "id が重複しています"));
            }
            sources.push(s);
        }
        Ok(Self { sources })
    }

    pub fn list(&self) -> &[IngestSource] {
        &self.sources
    }

    pub fn find(&self, id: u32) -> Option<&IngestSource> {
        self.sources.iter().find(|s| s.id == id)
    }

    /// 次に使える番号（過去に使った最大の番号 + 1。消した番号は再利用しない）。
    pub fn next_id(&self) -> u32 {
        self.sources.iter().map(|s| s.id).max().map_or(1, |m| m + 1)
    }

    /// 追加する。パスにタブや改行があるとエラー。
    pub fn add(&mut self, source: IngestSource) -> Result<(), UserDataError> {
        if has_cell_breaker(&source.path) || has_cell_breaker(&source.prompt_version) {
            return Err(UserDataError::Invalid(
                "パスとプロンプトの版にタブと改行は使えません".to_string(),
            ));
        }
        if self.find(source.id).is_some() {
            return Err(UserDataError::Invalid(format!(
                "取り込み元の番号 {} は使用済みです",
                source.id
            )));
        }
        self.sources.push(source);
        Ok(())
    }

    pub fn remove(&mut self, id: u32) -> bool {
        let before = self.sources.len();
        self.sources.retain(|s| s.id != id);
        self.sources.len() != before
    }

    pub fn clear(&mut self) {
        self.sources.clear();
    }

    pub fn to_text(&self) -> String {
        let mut out =
            format!("{SOURCES_HEADER_NAME}\tschema={SOURCES_SCHEMA}\n{SOURCES_COLUMNS}\n");
        for s in &self.sources {
            out.push_str(&format!(
                "{}\t{}\t{}\t{:016x}\t{}\t{}\t{}\n",
                s.id,
                s.path,
                s.imported_at,
                s.fnv64,
                s.prompt_version,
                s.sentences_ok,
                s.sentences_dropped
            ));
        }
        out
    }

    pub fn save(&self, path: &Path) -> Result<(), UserDataError> {
        write_atomic(path, &self.to_text())
    }
}

/// UNIX 秒を `YYYY-MM-DD HH:MM:SS`（UTC）にする（表示用。外部の日付ライブラリを使わない）。
pub fn format_utc(unix_secs: u64) -> String {
    let days = (unix_secs / SECONDS_PER_DAY) as i64;
    let rem = unix_secs % SECONDS_PER_DAY;
    // Howard Hinnant の civil_from_days
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// 登録候補 1 件（取り込み元をまたいで集計したもの）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IngestCandidate {
    pub reading: String,
    pub surface: String,
    pub count: u32,
}

/// 登録候補。(取り込み元, 読み, 表記) ごとの件数。
#[derive(Clone, Debug, Default)]
pub struct IngestCandidates {
    rows: BTreeMap<(u32, String, String), u32>,
}

impl IngestCandidates {
    pub fn open(path: &Path) -> Result<Self, UserDataError> {
        let Some(text) = read_optional(path)? else {
            return Ok(Self::default());
        };
        let mut lines = text.lines();
        let header = lines
            .next()
            .ok_or_else(|| parse_error(path, 1, "空のファイルです"))?;
        let kv = parse_header(header, CANDIDATES_HEADER_NAME).ok_or_else(|| {
            parse_error(
                path,
                1,
                format!("ヘッダが `{CANDIDATES_HEADER_NAME}` ではありません"),
            )
        })?;
        check_schema(path, &kv, CANDIDATES_SCHEMA)?;
        if lines.next() != Some(CANDIDATES_COLUMNS) {
            return Err(parse_error(path, 2, "列の見出し行が不正です"));
        }
        let mut rows = BTreeMap::new();
        for (i, line) in lines.enumerate() {
            let n = i + 3;
            if line.is_empty() {
                continue;
            }
            let f: Vec<&str> = line.split(FIELD_SEPARATOR).collect();
            if f.len() != CANDIDATES_COLUMN_COUNT {
                return Err(parse_error(path, n, "列数が不正です"));
            }
            let source = f[0]
                .parse::<u32>()
                .map_err(|_| parse_error(path, n, "source が整数ではありません"))?;
            let count = f[3]
                .parse::<u32>()
                .ok()
                .filter(|c| *c >= 1)
                .ok_or_else(|| parse_error(path, n, "count は 1 以上の整数です"))?;
            if !is_indexable(f[1]) || f[2].is_empty() {
                return Err(parse_error(path, n, "読みか表記が不正です"));
            }
            if rows
                .insert((source, f[1].to_string(), f[2].to_string()), count)
                .is_some()
            {
                return Err(parse_error(path, n, "行が重複しています"));
            }
        }
        Ok(Self { rows })
    }

    /// 件数を足す。読みが索引に使えない形、表記や読みにタブ・改行があるものは無視する。
    pub fn add(&mut self, source: u32, reading: &str, surface: &str, count: u32) {
        if surface.is_empty() || !is_indexable(reading) || has_cell_breaker(surface) || count == 0 {
            return;
        }
        let c = self
            .rows
            .entry((source, reading.to_string(), surface.to_string()))
            .or_default();
        *c = c.saturating_add(count);
    }

    pub fn remove_source(&mut self, source: u32) {
        self.rows.retain(|(s, _, _), _| *s != source);
    }

    pub fn clear(&mut self) {
        self.rows.clear();
    }

    /// (読み, 表記) ごとに取り込み元をまたいで集計し、件数の多い順（同数なら読み、表記の順）に返す。
    pub fn list(&self) -> Vec<IngestCandidate> {
        let mut merged: BTreeMap<(&str, &str), u32> = BTreeMap::new();
        for ((_, reading, surface), count) in &self.rows {
            let c = merged.entry((reading, surface)).or_default();
            *c = c.saturating_add(*count);
        }
        let mut out: Vec<IngestCandidate> = merged
            .into_iter()
            .map(|((reading, surface), count)| IngestCandidate {
                reading: reading.to_string(),
                surface: surface.to_string(),
                count,
            })
            .collect();
        out.sort_by_key(|c| std::cmp::Reverse(c.count));
        out
    }

    /// (読み, 表記) の候補をすべての取り込み元から消す。あれば true。
    pub fn take(&mut self, reading: &str, surface: &str) -> bool {
        let before = self.rows.len();
        self.rows
            .retain(|(_, r, s), _| !(r == reading && s == surface));
        self.rows.len() != before
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn save(&self, path: &Path) -> Result<(), UserDataError> {
        let mut out =
            format!("{CANDIDATES_HEADER_NAME}\tschema={CANDIDATES_SCHEMA}\n{CANDIDATES_COLUMNS}\n");
        for ((source, reading, surface), count) in &self.rows {
            out.push_str(&format!("{source}\t{reading}\t{surface}\t{count}\n"));
        }
        write_atomic(path, &out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src(id: u32) -> IngestSource {
        IngestSource {
            id,
            path: format!("/tmp/doc{id}.jsonl"),
            imported_at: 1_760_000_000,
            fnv64: 0xdead_beef,
            prompt_version: "ingest-prompt-v1".into(),
            sentences_ok: 10,
            sentences_dropped: 2,
        }
    }

    #[test]
    fn sources_roundtrip_and_ids_are_not_reused() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("s.tsv");
        let mut s = IngestSources::default();
        assert_eq!(s.next_id(), 1);
        s.add(src(1)).unwrap();
        s.add(src(2)).unwrap();
        assert!(s.add(src(2)).is_err());
        assert!(s.remove(2));
        assert_eq!(s.next_id(), 2);
        s.save(&p).unwrap();
        let back = IngestSources::open(&p).unwrap();
        assert_eq!(back.list(), s.list());
        assert!(!s.remove(9));
    }

    #[test]
    fn sources_reject_bad_files_and_paths() {
        let mut s = IngestSources::default();
        let mut bad = src(1);
        bad.path = "a\tb".into();
        assert!(s.add(bad).is_err());
        let h = format!("{SOURCES_HEADER_NAME}\tschema=1\n{SOURCES_COLUMNS}\n");
        for text in [
            String::new(),
            "#x\tschema=1\n".to_string(),
            format!("{SOURCES_HEADER_NAME}\tschema=3\n{SOURCES_COLUMNS}\n"),
            format!("{h}1\tp\tx\t0\tv\t1\t1\n"),
            format!("{h}1\tp\t1\tzz\tv\t1\t1\n"),
            format!("{h}1\tp\t1\t0\tv\t1\n"),
            format!("{h}1\tp\t1\t0\tv\t1\t1\n1\tq\t1\t0\tv\t1\t1\n"),
        ] {
            assert!(IngestSources::parse(&text).is_err(), "{text:?}");
        }
        assert!(IngestSources::parse(&h).unwrap().list().is_empty());
    }

    #[test]
    fn utc_formatting() {
        assert_eq!(format_utc(0), "1970-01-01 00:00:00");
        assert_eq!(format_utc(1_760_000_000), "2025-10-09 08:53:20");
        assert_eq!(format_utc(951_782_400), "2000-02-29 00:00:00");
    }

    #[test]
    fn candidates_aggregate_across_sources_and_remove_by_source() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("c.tsv");
        let mut c = IngestCandidates::default();
        c.add(1, "いかり", "怒里", 2);
        c.add(2, "いかり", "怒里", 3);
        c.add(1, "あんず", "杏子", 1);
        c.add(1, "bad", "x", 1);
        c.add(1, "あ", "", 1);
        let l = c.list();
        assert_eq!(l.len(), 2);
        assert_eq!((l[0].surface.as_str(), l[0].count), ("怒里", 5));
        c.save(&p).unwrap();
        let mut back = IngestCandidates::open(&p).unwrap();
        assert_eq!(back.list(), l);
        back.remove_source(2);
        assert_eq!(back.list()[0].count, 2);
        assert!(back.take("いかり", "怒里"));
        assert!(!back.take("いかり", "怒里"));
        assert_eq!(back.list().len(), 1);
    }
}
