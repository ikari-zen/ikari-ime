//! エクスポート／インポート（ADR 0002 §10.5）。
//!
//! ```text
//! #ikari-export<TAB>schema=1
//! [profile]
//! …profile.tsv の中身…
//! [user_dict]
//! …
//! [learning]
//! …（ソルトのヘッダを含む）…
//! [ingest_sources]
//! …
//! ```
//! 見出しは行全体が `[名前]` の行だけ。各セクションは、それぞれのファイルのスキーマ（バージョン付き）に従う。
//! 学習は特徴量のハッシュだけで、取り込んだ文の原文は含まない（NFR-19）。
//!
//! インポートは **上書き**（マージしない）。最初に全体を検証し、1 つでも不正なら何も変えない。
//! スキーマのバージョンが違うときは、変換関数ができるまでは「一致しなければエラー」。

use std::fs;
use std::path::PathBuf;

use crate::dict::Grammar;

use super::dictionary::{USER_DICT_SECTION_NAME, UserDictionary};
use super::ingest::{INGEST_SOURCES_SECTION_NAME, IngestSources};
use super::learning::{LEARNING_SECTION_NAME, LearningStore};
use super::profile::{PROFILE_SECTION_NAME, Profile};
use super::util::{check_schema, parse_error, parse_header, prepare_tmp, read_optional};
use super::{UserDataError, UserDir};

pub const EXPORT_SCHEMA: u32 = 1;
const EXPORT_HEADER_NAME: &str = "#ikari-export";
const SECTION_NAMES: [&str; 4] = [
    PROFILE_SECTION_NAME,
    USER_DICT_SECTION_NAME,
    LEARNING_SECTION_NAME,
    INGEST_SOURCES_SECTION_NAME,
];

/// インポートした件数
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportSummary {
    pub profile_domains: usize,
    pub user_dict_entries: usize,
    pub learning_rows: usize,
    pub ingest_sources: usize,
}

fn section_header(name: &str) -> String {
    format!("[{name}]")
}

fn ensure_newline(mut s: String) -> String {
    if !s.ends_with('\n') {
        s.push('\n');
    }
    s
}

/// ユーザーデータをエクスポート用のテキストにする。ファイルがないものは空のデータとして書く。
/// 既存のファイルが不正なら、不正なものを書き出さずにエラーにする。
pub fn export_text(dir: &UserDir) -> Result<String, UserDataError> {
    let profile = match read_optional(&dir.profile_tsv())? {
        Some(t) => {
            Profile::parse_settings(&t)?;
            t
        }
        None => Profile::serialize_settings(&[]),
    };
    let user_dict = match read_optional(&dir.user_dict_tsv())? {
        Some(t) => {
            UserDictionary::parse(&t)?;
            t
        }
        None => UserDictionary::default().to_text(),
    };
    // 開く時に検証される。なければ空のストア（ソルトは新しく作られる）
    let learning = LearningStore::open(&dir.learning_tsv())?.to_text();
    let sources = match read_optional(&dir.ingest_sources_tsv())? {
        Some(t) => {
            IngestSources::parse(&t)?;
            t
        }
        None => IngestSources::default().to_text(),
    };

    let mut out = format!("{EXPORT_HEADER_NAME}\tschema={EXPORT_SCHEMA}\n");
    for (name, body) in SECTION_NAMES
        .iter()
        .zip([profile, user_dict, learning, sources])
    {
        out.push_str(&section_header(name));
        out.push('\n');
        out.push_str(&ensure_newline(body));
    }
    Ok(out)
}

/// 検証済みのセクション（名前の順は `SECTION_NAMES`）
struct Sections {
    bodies: [String; 4],
    summary: ImportSummary,
}

fn split_sections(text: &str) -> Result<[String; 4], UserDataError> {
    let file = PathBuf::from("export");
    let mut lines = text.lines();
    let header = lines
        .next()
        .ok_or_else(|| parse_error(&file, 1, "空のファイルです"))?;
    let kv = parse_header(header, EXPORT_HEADER_NAME).ok_or_else(|| {
        parse_error(
            &file,
            1,
            format!("ヘッダが `{EXPORT_HEADER_NAME}` ではありません（ikari のエクスポートではありません）"),
        )
    })?;
    check_schema(&file, &kv, EXPORT_SCHEMA)?;

    let mut bodies: [Option<String>; 4] = [None, None, None, None];
    let mut current: Option<usize> = None;
    for (i, line) in lines.enumerate() {
        let n = i + 2;
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            let idx = SECTION_NAMES
                .iter()
                .position(|s| *s == name)
                .ok_or_else(|| parse_error(&file, n, format!("未知のセクションです: [{name}]")))?;
            if bodies[idx].is_some() {
                return Err(parse_error(
                    &file,
                    n,
                    format!("セクションが重複しています: [{name}]"),
                ));
            }
            bodies[idx] = Some(String::new());
            current = Some(idx);
            continue;
        }
        match current {
            Some(idx) => {
                let body = bodies[idx].get_or_insert_with(String::new);
                body.push_str(line);
                body.push('\n');
            }
            None if line.is_empty() => {}
            None => {
                return Err(parse_error(
                    &file,
                    n,
                    "最初のセクションの前に内容があります",
                ));
            }
        }
    }
    let [profile, user_dict, learning, sources] = bodies;
    let missing = |name: &str| UserDataError::Invalid(format!("セクション [{name}] がありません"));
    Ok([
        profile.ok_or_else(|| missing(SECTION_NAMES[0]))?,
        user_dict.ok_or_else(|| missing(SECTION_NAMES[1]))?,
        learning.ok_or_else(|| missing(SECTION_NAMES[2]))?,
        sources.ok_or_else(|| missing(SECTION_NAMES[3]))?,
    ])
}

/// 検証の失敗。どのセクションで起きたか（ヘッダや構造そのものなら None）を添える。
type Tagged = (Option<&'static str>, UserDataError);

fn validate(text: &str) -> Result<Sections, UserDataError> {
    validate_tagged(text, None).map_err(|(_, e)| e)
}

fn validate_tagged(text: &str, grammar: Option<&dyn Grammar>) -> Result<Sections, Tagged> {
    let tag = |index: usize| move |e: UserDataError| (Some(SECTION_NAMES[index]), e);
    let bodies = split_sections(text).map_err(|e| (None, e))?;
    let profile = Profile::parse_settings(&bodies[0]).map_err(tag(0))?;
    let user_dict = UserDictionary::parse(&bodies[1]).map_err(tag(1))?;
    if let Some(grammar) = grammar {
        // 展開できない辞書を取り込むと、ユーザー辞書全体が使われなくなる（組み立て直しで警告になるだけ）
        user_dict.build(grammar).map_err(tag(1))?;
    }
    LearningStore::validate_text(&bodies[2]).map_err(tag(2))?;
    let sources = IngestSources::parse(&bodies[3]).map_err(tag(3))?;
    let learning_rows = bodies[2].lines().skip(2).filter(|l| !l.is_empty()).count();
    Ok(Sections {
        summary: ImportSummary {
            profile_domains: profile.len(),
            user_dict_entries: user_dict.len(),
            learning_rows,
            ingest_sources: sources.list().len(),
        },
        bodies,
    })
}

/// エクスポートのテキストを検証だけする（何も書かない）。
pub fn validate_export(text: &str) -> Result<ImportSummary, UserDataError> {
    validate(text).map(|s| s.summary)
}

/// 検証の失敗の種類（設定画面に理由として出すための分類）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportIssueKind {
    /// ikari のエクスポートではない（空、ヘッダが違う）
    NotIkariExport,
    /// schema が対応していない
    SchemaUnsupported,
    /// 形式が壊れている（行の列数、未知のセクション、足りないセクションなど）
    Parse,
    /// 値が不正（品詞が文法で展開できないなど）
    InvalidEntry,
    Io,
}

/// 検証の失敗の説明。`detail` には利用者のデータ（行の内容）が入りうるので、ログに書かないこと。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportIssue {
    pub kind: ExportIssueKind,
    /// 起きたセクション（`profile` / `user_dict` / `learning` / `ingest_sources`）。ファイル全体なら None
    pub section: Option<&'static str>,
    /// セクションの中の行番号（1 始まり）。不明なら 0
    pub line: usize,
    /// `SchemaUnsupported` のとき、見つかった版と対応している版
    pub found_schema: u32,
    pub supported_schema: u32,
    pub detail: String,
}

impl ExportIssue {
    /// `section` で起きた `err` を分類する。
    pub fn from_error(section: Option<&'static str>, err: &UserDataError) -> ExportIssue {
        let mut issue = ExportIssue {
            kind: ExportIssueKind::Parse,
            section,
            line: 0,
            found_schema: 0,
            supported_schema: 0,
            detail: err.to_string(),
        };
        match err {
            UserDataError::Io(_) => issue.kind = ExportIssueKind::Io,
            UserDataError::Parse { line, message, .. } => {
                issue.line = *line;
                issue.detail.clone_from(message);
                // ファイルの先頭で、どのセクションにも入っていない
                if section.is_none() && *line == 1 {
                    issue.kind = ExportIssueKind::NotIkariExport;
                }
            }
            UserDataError::Schema { found, expected } => {
                issue.kind = ExportIssueKind::SchemaUnsupported;
                issue.found_schema = *found;
                issue.supported_schema = *expected;
            }
            // セクションが足りない（section は None）ときは形式の問題のまま
            UserDataError::Invalid(message) => {
                issue.detail.clone_from(message);
                if section.is_some() {
                    issue.kind = ExportIssueKind::InvalidEntry;
                }
            }
        }
        issue
    }
}

/// `validate_export` の、失敗の理由（セクションと分類）を返す版。`grammar` があれば、
/// ユーザー辞書の品詞が文法で展開できることも確かめる。何も書かない。
pub fn check_export(
    text: &str,
    grammar: Option<&dyn Grammar>,
) -> Result<ImportSummary, ExportIssue> {
    validate_tagged(text, grammar)
        .map(|s| s.summary)
        .map_err(|(section, e)| ExportIssue::from_error(section, &e))
}

/// エクスポートのテキストで、ユーザーデータを上書きする。
/// 検証に失敗したら何も変えない。書き込みは全ファイルを一時ファイルに書いてから rename する。
/// 取り込み元の登録候補（`ingest_candidates.tsv`）はエクスポートに含まれないので、そのまま残す。
pub fn import_text(dir: &UserDir, text: &str) -> Result<ImportSummary, UserDataError> {
    let sections = validate(text)?;
    let targets = [
        dir.profile_tsv(),
        dir.user_dict_tsv(),
        dir.learning_tsv(),
        dir.ingest_sources_tsv(),
    ];
    let mut tmps: Vec<PathBuf> = Vec::new();
    for (target, body) in targets.iter().zip(&sections.bodies) {
        match prepare_tmp(target, body) {
            Ok(tmp) => tmps.push(tmp),
            Err(e) => {
                for t in &tmps {
                    let _ = fs::remove_file(t);
                }
                return Err(e);
            }
        }
    }
    for (tmp, target) in tmps.iter().zip(&targets) {
        fs::rename(tmp, target)?;
    }
    Ok(sections.summary)
}

#[cfg(test)]
mod tests {
    use super::super::dictionary::UserDictEntry;
    use super::super::profile::ProfileWeight;
    use super::*;

    fn populated(dir: &UserDir) {
        let mut d = UserDictionary::default();
        d.add(UserDictEntry {
            reading: "いかり".into(),
            surface: "ikari-ime".into(),
            pos: "短縮よみ".into(),
            comment: "自作IME".into(),
        })
        .unwrap();
        d.save(&dir.user_dict_tsv()).unwrap();
        Profile::from_settings(&[("it".into(), ProfileWeight::OFTEN)])
            .unwrap()
            .save(dir)
            .unwrap();
        let store = LearningStore::open(&dir.learning_tsv()).unwrap();
        store.save().unwrap();
    }

    fn snapshot(dir: &UserDir) -> Vec<Option<String>> {
        [
            dir.profile_tsv(),
            dir.user_dict_tsv(),
            dir.learning_tsv(),
            dir.ingest_sources_tsv(),
        ]
        .iter()
        .map(|p| fs::read_to_string(p).ok())
        .collect()
    }

    #[test]
    fn export_import_roundtrip_across_directories() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let (da, db) = (UserDir::new(a.path()), UserDir::new(b.path()));
        populated(&da);
        let text = export_text(&da).unwrap();
        assert!(text.starts_with("#ikari-export\tschema=1\n[profile]\n"));
        for name in SECTION_NAMES {
            assert!(text.contains(&format!("\n[{name}]\n")), "{name}");
        }
        let summary = import_text(&db, &text).unwrap();
        assert_eq!(summary.user_dict_entries, 1);
        assert_eq!(summary.profile_domains, 1);
        // 往復で内容が一致し、再エクスポートも同じ
        assert_eq!(export_text(&db).unwrap(), text);
        assert_eq!(
            LearningStore::open(&db.learning_tsv()).unwrap().salt(),
            LearningStore::open(&da.learning_tsv()).unwrap().salt()
        );
    }

    #[test]
    fn import_overwrites_instead_of_merging() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let (da, db) = (UserDir::new(a.path()), UserDir::new(b.path()));
        populated(&da);
        // b には別の辞書がある
        let mut other = UserDictionary::default();
        other
            .add(UserDictEntry {
                reading: "ほか".into(),
                surface: "他".into(),
                pos: "名詞".into(),
                comment: String::new(),
            })
            .unwrap();
        other.save(&db.user_dict_tsv()).unwrap();
        import_text(&db, &export_text(&da).unwrap()).unwrap();
        let after = UserDictionary::open(&db.user_dict_tsv()).unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(after.entries()[0].surface, "ikari-ime");
    }

    #[test]
    fn exporting_an_empty_directory_works() {
        let a = tempfile::tempdir().unwrap();
        let text = export_text(&UserDir::new(a.path())).unwrap();
        let summary = validate_export(&text).unwrap();
        assert_eq!(summary, ImportSummary::default());
    }

    #[test]
    fn invalid_imports_change_nothing() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let (da, db) = (UserDir::new(a.path()), UserDir::new(b.path()));
        populated(&da);
        populated(&db);
        let good = export_text(&da).unwrap();
        let before = snapshot(&db);

        let corrupt_learning = good.replacen("schema=1\tsalt=", "schema=1\tsalt=zz", 1);
        let bad_dict = good.replace("ikari-ime\t短縮よみ", "ikari-ime\t謎の品詞");
        let no_sources = good.split("[ingest_sources]").next().unwrap().to_string();
        let dup = format!("{good}[profile]\n#ikari-profile\tschema=1\n");
        let unknown = format!("{good}[extra]\nx\n");
        let wrong_schema = good.replacen("#ikari-export\tschema=1", "#ikari-export\tschema=9", 1);
        let cases = [
            String::new(),
            "hello\n".to_string(),
            corrupt_learning,
            bad_dict,
            no_sources,
            dup,
            unknown,
            wrong_schema,
            format!("junk\n{good}"),
        ];
        for (i, text) in cases.iter().enumerate() {
            assert!(import_text(&db, text).is_err(), "case {i} accepted");
            assert_eq!(snapshot(&db), before, "case {i} changed files");
        }
        let schema_err = import_text(
            &db,
            &good.replacen("#ikari-export\tschema=1", "#ikari-export\tschema=9", 1),
        )
        .unwrap_err();
        assert!(matches!(schema_err, UserDataError::Schema { found: 9, .. }));
        // 一時ファイルが残らない
        let names: Vec<String> = fs::read_dir(b.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(names.iter().all(|n| !n.ends_with(".tmp")), "{names:?}");
    }

    #[test]
    fn export_refuses_a_corrupt_source_file() {
        let a = tempfile::tempdir().unwrap();
        let da = UserDir::new(a.path());
        fs::write(da.user_dict_tsv(), "garbage\n").unwrap();
        assert!(export_text(&da).is_err());
    }

    #[test]
    fn check_export_classifies_failures_by_section() {
        let a = tempfile::tempdir().unwrap();
        let da = UserDir::new(a.path());
        populated(&da);
        let good = export_text(&da).unwrap();
        assert_eq!(check_export(&good, None).unwrap().user_dict_entries, 1);

        let issue = |text: &str| check_export(text, None).unwrap_err();
        assert_eq!(issue("").kind, ExportIssueKind::NotIkariExport);
        assert_eq!(issue("hello\n").kind, ExportIssueKind::NotIkariExport);
        let schema = issue(&good.replacen("#ikari-export\tschema=1", "#ikari-export\tschema=9", 1));
        assert_eq!(schema.kind, ExportIssueKind::SchemaUnsupported);
        assert_eq!((schema.found_schema, schema.supported_schema), (9, 1));
        assert_eq!(schema.section, None);
        // セクションの schema が違う
        let section = issue(&good.replacen(
            "#ikari-user-dict\tschema=1",
            "#ikari-user-dict\tschema=2",
            1,
        ));
        assert_eq!(section.kind, ExportIssueKind::SchemaUnsupported);
        assert_eq!(section.section, Some("user_dict"));
        assert_eq!(section.found_schema, 2);
        // 項目の不正は、セクションと行が分かる
        let bad = issue(&good.replace("ikari-ime\t短縮よみ", "ikari-ime\t謎の品詞"));
        assert_eq!(bad.kind, ExportIssueKind::Parse);
        assert_eq!((bad.section, bad.line), (Some("user_dict"), 2));
        // セクションが足りない
        let missing = issue(good.split("[ingest_sources]").next().unwrap());
        assert_eq!(missing.kind, ExportIssueKind::Parse);
        assert_eq!(missing.section, None);
        let unknown = issue(&format!("{good}[extra]\nx\n"));
        assert_eq!(unknown.kind, ExportIssueKind::Parse);
    }
}
