//! 評価セット（TSV）の読み込みと絞り込み（ADR 0002 §15.2）。

use std::collections::{BTreeMap, HashSet};
use std::fmt;

use ikari_dict_format::Fnv1a64;

/// `expected` に複数の正解を書くときの区切り（表記には使えない）
pub const ALT_SEPARATOR: char = '|';
/// 機械的な検証で「期待値が候補に出ない」と判定されたケースの印。既定では評価から除く
pub const TAG_UNREACHABLE: &str = "unreachable";
/// プロファイルの有無で下がってはいけないケースの種別（ADR 0002 §9.2, §15.2）
pub const GUARD_KINDS: [&str; 2] = ["否定", "文法"];
/// dev / test の分け方: `FNV-1a64(id) % SPLIT_MODULUS == 0` なら test（ADR 0002 §15.2）
const SPLIT_MODULUS: u64 = 5;

/// 評価セットの分割。調整は dev だけで行い、判断は test で行う
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Split {
    Dev,
    Test,
}

impl Split {
    pub fn as_str(self) -> &'static str {
        match self {
            Split::Dev => "dev",
            Split::Test => "test",
        }
    }

    pub fn parse(s: &str) -> Option<Split> {
        match s {
            "dev" => Some(Split::Dev),
            "test" => Some(Split::Test),
            _ => None,
        }
    }
}

/// id から決定的に分割を決める（`FNV-1a64(id) % 5 == 0` が test）
pub fn split_of_id(id: &str) -> Split {
    let mut hash = Fnv1a64::new();
    hash.update(id.as_bytes());
    if hash.finish().is_multiple_of(SPLIT_MODULUS) {
        Split::Test
    } else {
        Split::Dev
    }
}

/// `--split` の指定
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitSel {
    Dev,
    Test,
    All,
}

impl SplitSel {
    pub fn parse(s: &str) -> Option<SplitSel> {
        match s {
            "dev" => Some(SplitSel::Dev),
            "test" => Some(SplitSel::Test),
            "all" => Some(SplitSel::All),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            SplitSel::Dev => "dev",
            SplitSel::Test => "test",
            SplitSel::All => "all",
        }
    }

    /// `split` 列がないケース（None）は、どの指定にも含める
    pub fn includes(self, split: Option<Split>) -> bool {
        match (self, split) {
            (SplitSel::All, _) | (_, None) => true,
            (SplitSel::Dev, Some(s)) => s == Split::Dev,
            (SplitSel::Test, Some(s)) => s == Split::Test,
        }
    }
}

#[derive(Clone, Debug)]
pub struct EvalCase {
    pub id: String,
    pub kind: String,
    pub tags: Vec<String>,
    pub profile: Option<String>,
    pub context: String,
    /// 右文脈（`right` 列。なければ空）
    pub right: String,
    pub reading: String,
    /// 正解の表記。`|` で区切って別解を書ける（`alternatives()`）
    pub expected: String,
    pub note: String,
    /// `split` 列がなければ None（どの `--split` にも含まれる）。列があって空なら id から決める
    pub split: Option<Split>,
}

impl EvalCase {
    /// 正解の表記（別解を含む）
    pub fn alternatives(&self) -> Vec<&str> {
        self.expected.split(ALT_SEPARATOR).collect()
    }

    pub fn is_unreachable(&self) -> bool {
        self.tags.iter().any(|t| t == TAG_UNREACHABLE)
    }

    /// プロファイルで順位が覆ってはいけない種別（否定、文法）
    pub fn is_guard(&self) -> bool {
        GUARD_KINDS.contains(&self.kind.as_str())
    }

    /// 表記が正解（別解を含む）のどれかと一致するか
    pub fn matches(&self, surface: &str) -> bool {
        self.expected.split(ALT_SEPARATOR).any(|a| a == surface)
    }
}

#[derive(Debug)]
pub struct EvalSetError {
    pub line: usize,
    pub message: String,
}

impl fmt::Display for EvalSetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "eval set line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for EvalSetError {}

const COL_ID: &str = "id";
const COL_KIND: &str = "kind";
const COL_TAGS: &str = "tags";
const COL_PROFILE: &str = "profile";
const COL_CONTEXT: &str = "context";
const COL_RIGHT: &str = "right";
const COL_READING: &str = "reading";
const COL_EXPECTED: &str = "expected";
const COL_NOTE: &str = "note";
const COL_SPLIT: &str = "split";
const REQUIRED_COLS: [&str; 5] = [COL_ID, COL_KIND, COL_CONTEXT, COL_READING, COL_EXPECTED];
const COMMENT_PREFIX: char = '#';
const TAG_SEPARATOR: char = ',';

fn err(line: usize, message: impl Into<String>) -> EvalSetError {
    EvalSetError {
        line,
        message: message.into(),
    }
}

/// `expected` の別解が空でないことを確かめる（`a||b`、末尾の `|` は誤り）
pub(crate) fn validate_expected(expected: &str) -> Result<(), String> {
    if expected.split(ALT_SEPARATOR).any(str::is_empty) {
        return Err(format!(
            "empty alternative in expected `{expected}` (`{ALT_SEPARATOR}` separates alternatives and cannot be part of a surface)"
        ));
    }
    Ok(())
}

/// 評価セット（TSV）を読む。コメント行と空行を除いた最初の行がヘッダで、列は名前で対応付ける。
/// 知らない列は無視する。データ行のセルがヘッダより少ない場合、足りない末尾の列は空として扱う。
///
/// - `expected` は `|` で区切って別解を書ける。
/// - `split` 列（`dev` / `test` / 空）があれば読む。列があって空なら id のハッシュで決める。
pub fn load_cases(tsv: &str) -> Result<Vec<EvalCase>, EvalSetError> {
    let mut header: Option<BTreeMap<String, usize>> = None;
    let mut width = 0;
    let mut cases = Vec::new();
    let mut ids = HashSet::new();

    for (i, raw) in tsv.lines().enumerate() {
        let line_no = i + 1;
        let line = raw.trim_end_matches('\r');
        if line.trim().is_empty() || line.starts_with(COMMENT_PREFIX) {
            continue;
        }
        let cells: Vec<&str> = line.split('\t').collect();

        let Some(cols) = &header else {
            let mut map = BTreeMap::new();
            for (idx, name) in cells.iter().enumerate() {
                let name = name.trim();
                if map.insert(name.to_owned(), idx).is_some() {
                    return Err(err(line_no, format!("duplicate column `{name}`")));
                }
            }
            for req in REQUIRED_COLS {
                if !map.contains_key(req) {
                    return Err(err(line_no, format!("missing required column `{req}`")));
                }
            }
            width = cells.len();
            header = Some(map);
            continue;
        };

        if cells.len() > width {
            return Err(err(
                line_no,
                format!("{} cells but header has {width} columns", cells.len()),
            ));
        }
        let get = |name: &str| -> &str {
            cols.get(name)
                .and_then(|&idx| cells.get(idx))
                .copied()
                .unwrap_or("")
        };
        for req in [COL_ID, COL_READING, COL_EXPECTED] {
            if get(req).is_empty() {
                return Err(err(line_no, format!("empty required value `{req}`")));
            }
        }
        let id = get(COL_ID);
        if !ids.insert(id.to_owned()) {
            return Err(err(line_no, format!("duplicate id `{id}`")));
        }
        validate_expected(get(COL_EXPECTED)).map_err(|m| err(line_no, m))?;
        let split = if cols.contains_key(COL_SPLIT) {
            match get(COL_SPLIT).trim() {
                "" => Some(split_of_id(id)),
                s => Some(
                    Split::parse(s)
                        .ok_or_else(|| err(line_no, format!("bad split `{s}` (dev or test)")))?,
                ),
            }
        } else {
            None
        };
        cases.push(EvalCase {
            id: id.to_owned(),
            kind: get(COL_KIND).to_owned(),
            tags: split_tags(get(COL_TAGS)),
            profile: Some(get(COL_PROFILE).trim())
                .filter(|p| !p.is_empty())
                .map(str::to_owned),
            context: get(COL_CONTEXT).to_owned(),
            right: get(COL_RIGHT).to_owned(),
            reading: get(COL_READING).to_owned(),
            expected: get(COL_EXPECTED).to_owned(),
            note: get(COL_NOTE).to_owned(),
            split,
        });
    }

    if header.is_none() {
        return Err(err(1, "missing header line"));
    }
    Ok(cases)
}

fn split_tags(cell: &str) -> Vec<String> {
    cell.split(TAG_SEPARATOR)
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .collect()
}

/// `filter_cases` の結果
#[derive(Clone, Debug, Default)]
pub struct Filtered {
    pub cases: Vec<EvalCase>,
    /// `unreachable` の印で除いた件数
    pub excluded_unreachable: usize,
    /// `--split` で除いた件数
    pub excluded_by_split: usize,
}

/// `--split` と `unreachable` の印でケースを絞る。
pub fn filter_cases(cases: Vec<EvalCase>, split: SplitSel, include_unreachable: bool) -> Filtered {
    let mut out = Filtered::default();
    for c in cases {
        if !split.includes(c.split) {
            out.excluded_by_split += 1;
        } else if !include_unreachable && c.is_unreachable() {
            out.excluded_unreachable += 1;
        } else {
            out.cases.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
# comment
id\tkind\ttags\tprofile\tcontext\treading\texpected\tnote\textra
h-01\t話題\t\t\t辞書に登録するときは\tひんし\t品詞

a-01\t話題\t怒り, 学習\tit\t\tひんし\t瀕死\tメモ\tignored
";

    #[test]
    fn load_cases_maps_columns_by_name() {
        let cases = load_cases(SAMPLE).unwrap();
        assert_eq!(cases.len(), 2);
        let c = &cases[0];
        assert_eq!((c.id.as_str(), c.kind.as_str()), ("h-01", "話題"));
        assert_eq!(c.context, "辞書に登録するときは");
        assert_eq!(
            (c.reading.as_str(), c.expected.as_str()),
            ("ひんし", "品詞")
        );
        assert!(c.tags.is_empty());
        assert_eq!(c.profile, None);
        assert_eq!(c.note, "");
        assert_eq!(c.split, None, "no split column -> None");
        let c = &cases[1];
        assert_eq!(c.tags, ["怒り", "学習"]);
        assert_eq!(c.profile.as_deref(), Some("it"));
        assert_eq!(c.context, "");
        assert_eq!(c.note, "メモ");
    }

    #[test]
    fn load_cases_accepts_any_column_order_and_crlf() {
        let tsv = "expected\treading\tid\tcontext\tkind\r\n品詞\tひんし\tx-1\tc\t話題\r\n";
        let c = load_cases(tsv).unwrap();
        assert_eq!(c[0].id, "x-1");
        assert_eq!(c[0].expected, "品詞");
        assert_eq!(c[0].context, "c");
        assert_eq!(c[0].kind, "話題");
    }

    #[test]
    fn load_cases_rejects_bad_input() {
        let e = load_cases("id\tkind\treading\texpected\nx\tk\tr\te\n").unwrap_err();
        assert!(e.message.contains("context"), "{e}");
        assert_eq!(e.line, 1);

        let head = "id\tkind\tcontext\treading\texpected\n";
        let e = load_cases(&format!("# c\n{head}x\tk\tc\t\te\n")).unwrap_err();
        assert_eq!(e.line, 3);
        assert!(e.message.contains("reading"));

        let e = load_cases(&format!("{head}x\tk\tc\tr\te\tTOO\tMANY\n")).unwrap_err();
        assert_eq!(e.line, 2);

        let e = load_cases(&format!("{head}x\tk\tc\tr\te\nx\tk\tc\tr\te\n")).unwrap_err();
        assert_eq!(e.line, 3);
        assert!(e.message.contains("duplicate id"));

        assert!(load_cases("").is_err());
        assert!(load_cases("# only comment\n").is_err());
        assert!(load_cases("id\tid\tkind\tcontext\treading\texpected\n").is_err());
    }

    #[test]
    fn expected_accepts_alternatives_and_rejects_empty_ones() {
        let head = "id\tkind\tcontext\treading\texpected\n";
        let c = load_cases(&format!("{head}x\tk\tc\tr\t品詞|瀕死|貧死\n")).unwrap();
        assert_eq!(c[0].alternatives(), ["品詞", "瀕死", "貧死"]);
        assert!(c[0].matches("瀕死"));
        assert!(!c[0].matches("品詞|瀕死"));
        assert!(!c[0].matches("品"));
        for bad in ["a||b", "|a", "a|", "|"] {
            let e = load_cases(&format!("{head}x\tk\tc\tr\t{bad}\n")).unwrap_err();
            assert_eq!(e.line, 2, "{bad}");
            assert!(e.message.contains("alternative"), "{e}");
        }
    }

    #[test]
    fn split_column_explicit_empty_and_invalid() {
        let head = "id\tkind\tcontext\treading\texpected\tsplit\n";
        let tsv = format!(
            "{head}a\tk\tc\tr\te\tdev\nb\tk\tc\tr\te\ttest\nc\tk\tc\tr\te\t\nd\tk\tc\tr\te\n"
        );
        let c = load_cases(&tsv).unwrap();
        assert_eq!(c[0].split, Some(Split::Dev));
        assert_eq!(c[1].split, Some(Split::Test));
        // 列があって空なら id のハッシュで決める
        assert_eq!(c[2].split, Some(split_of_id("c")));
        assert_eq!(c[3].split, Some(split_of_id("d")));
        let e = load_cases(&format!("{head}a\tk\tc\tr\te\tfoo\n")).unwrap_err();
        assert!(e.message.contains("split"), "{e}");
    }

    #[test]
    fn split_of_id_is_deterministic_and_about_one_in_five() {
        assert_eq!(split_of_id("h-01"), split_of_id("h-01"));
        let n = 2000;
        let test = (0..n)
            .filter(|i| split_of_id(&format!("ext-{i}")) == Split::Test)
            .count();
        assert!((300..=500).contains(&test), "test share: {test}/{n}");
    }

    /// 同梱の評価セットの split 列は `split_of_id` で決めた値。ハッシュの実装を変えても割り当てが変わらないことを固定する。
    #[test]
    fn split_of_id_matches_the_bundled_sets() {
        let mut checked = 0;
        for name in ["homophones.tsv", "homophones-ext.tsv", "hiragana.tsv"] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("sets")
                .join(name);
            let text = std::fs::read_to_string(&path).unwrap();
            let mut rows = text.lines().filter(|l| !l.starts_with('#'));
            let header: Vec<&str> = rows.next().unwrap().split('\t').collect();
            let id_col = header.iter().position(|c| *c == "id").unwrap();
            let Some(split_col) = header.iter().position(|c| *c == "split") else {
                continue;
            };
            for row in rows {
                let cells: Vec<&str> = row.split('\t').collect();
                let declared = cells.get(split_col).copied().unwrap_or("");
                if declared.is_empty() {
                    continue;
                }
                assert_eq!(
                    split_of_id(cells[id_col]).as_str(),
                    declared,
                    "{name}: {row}"
                );
                checked += 1;
            }
        }
        assert!(checked > 500, "checked only {checked} rows");
        // FNV-1a64 の既知の値: "a" = 0xaf63dc4c8601ec8c, "foobar" = 0x85944171f73967e8
        for (input, expected) in [
            (&b"a"[..], 0xaf63_dc4c_8601_ec8c_u64),
            (&b"foobar"[..], 0x8594_4171_f739_67e8),
        ] {
            let mut h = Fnv1a64::new();
            h.update(input);
            assert_eq!(h.finish(), expected);
        }
    }

    #[test]
    fn filter_cases_by_split_and_unreachable() {
        let head = "id\tkind\ttags\tcontext\treading\texpected\tsplit\n";
        let tsv = format!(
            "{head}a\tk\t\tc\tr\te\tdev\nb\tk\tllm, unreachable\tc\tr\te\tdev\nc\tk\t\tc\tr\te\ttest\n"
        );
        let all = load_cases(&tsv).unwrap();

        let f = filter_cases(all.clone(), SplitSel::Dev, false);
        assert_eq!(ids(&f.cases), ["a"]);
        assert_eq!((f.excluded_unreachable, f.excluded_by_split), (1, 1));

        let f = filter_cases(all.clone(), SplitSel::Dev, true);
        assert_eq!(ids(&f.cases), ["a", "b"]);

        let f = filter_cases(all.clone(), SplitSel::Test, false);
        assert_eq!(ids(&f.cases), ["c"]);

        let f = filter_cases(all, SplitSel::All, false);
        assert_eq!(ids(&f.cases), ["a", "c"]);
    }

    #[test]
    fn filter_cases_keeps_cases_without_split_column() {
        let tsv = "id\tkind\tcontext\treading\texpected\na\tk\tc\tr\te\n";
        let cases = load_cases(tsv).unwrap();
        for sel in [SplitSel::Dev, SplitSel::Test, SplitSel::All] {
            assert_eq!(filter_cases(cases.clone(), sel, false).cases.len(), 1);
        }
    }

    fn ids(cases: &[EvalCase]) -> Vec<&str> {
        cases.iter().map(|c| c.id.as_str()).collect()
    }

    #[test]
    fn split_sel_parse() {
        assert_eq!(SplitSel::parse("dev"), Some(SplitSel::Dev));
        assert_eq!(SplitSel::parse("all"), Some(SplitSel::All));
        assert_eq!(SplitSel::parse("Test"), None);
        assert_eq!(SplitSel::Test.as_str(), "test");
    }

    #[test]
    fn bundled_homophones_set_parses() {
        let tsv = include_str!("../sets/homophones.tsv");
        let cases = load_cases(tsv).unwrap();
        assert_eq!(cases.len(), 14);
        let a01 = cases.iter().find(|c| c.id == "a-01").unwrap();
        assert_eq!(a01.tags, ["怒り"]);
        assert_eq!(a01.expected, "品詞");
        let h08 = cases.iter().find(|c| c.id == "h-08").unwrap();
        assert_eq!(h08.profile.as_deref(), Some("it"));
        assert_eq!(h08.kind, "プロファイル");
        let angry = cases
            .iter()
            .filter(|c| c.tags.contains(&"怒り".to_owned()))
            .count();
        assert_eq!(angry, 5);
        // 否定ケースはプロファイルの有無で比べられるよう、profile 列を持つ（ADR 0002 §9.2）
        for id in ["h-07", "a-05"] {
            let c = cases.iter().find(|c| c.id == id).unwrap();
            assert!(c.is_guard(), "{id}");
            assert!(c.profile.is_some(), "{id} needs a profile for --profile-ab");
        }
    }

    #[test]
    fn bundled_hiragana_set_is_consistent() {
        let cases = load_cases(include_str!("../sets/hiragana.tsv")).unwrap();
        assert!((100..=150).contains(&cases.len()), "{}", cases.len());
        let mut ids = HashSet::new();
        for c in &cases {
            assert!(ids.insert(c.id.clone()), "duplicate id {}", c.id);
            assert_eq!(c.kind, "表記", "{}", c.id);
            assert!(c.tags.contains(&"llm".to_owned()), "{}", c.id);
            assert!(!c.note.is_empty(), "{} needs a rule in note", c.id);
            assert_eq!(c.split, Some(split_of_id(&c.id)), "{}", c.id);
        }
        // (a) ひらがなが正解、(b) 漢字が正解、(c) 不要な分割の検出 の3種がそろう
        for tag in ["ひらがな", "漢字", "分割"] {
            let n = cases
                .iter()
                .filter(|c| c.tags.contains(&tag.to_owned()))
                .count();
            assert!(n >= 15, "{tag}: {n}");
        }
        let test = cases
            .iter()
            .filter(|c| c.split == Some(Split::Test))
            .count();
        assert!((15..=45).contains(&test), "test share: {test}");
    }

    #[test]
    fn bundled_extended_set_is_consistent() {
        let cases = load_cases(include_str!("../sets/homophones-ext.tsv")).unwrap();
        assert!(cases.len() >= 800, "{}", cases.len());
        assert!(cases.iter().all(|c| c.tags.contains(&"llm".to_owned())));
        // split 列は id のハッシュと一致する（調整は dev だけで行う）
        assert!(cases.iter().all(|c| c.split == Some(split_of_id(&c.id))));
        let test = cases
            .iter()
            .filter(|c| c.split == Some(Split::Test))
            .count();
        assert!((cases.len() / 6..cases.len() / 4).contains(&test), "{test}");
        // 否定ケースと文法ケースは各20件以上で、profile 列を持つ
        for kind in GUARD_KINDS {
            let guards: Vec<_> = cases.iter().filter(|c| c.kind == kind).collect();
            assert!(guards.len() >= 20, "{kind}: {}", guards.len());
            assert!(guards.iter().all(|c| c.profile.is_some()));
        }
        // 同音異義語は、読みごとに複数の表記を持つ
        let mut by_reading: BTreeMap<&str, HashSet<&str>> = BTreeMap::new();
        for c in cases.iter().filter(|c| c.kind == "話題") {
            by_reading
                .entry(&c.reading)
                .or_default()
                .insert(&c.expected);
        }
        assert!(by_reading.len() >= 30, "{}", by_reading.len());
        assert!(by_reading.values().all(|s| s.len() >= 2));
    }

    #[test]
    fn right_column_is_optional() {
        let with =
            "id\tkind\tcontext\tright\treading\texpected\nr-1\tk\t川に架かる\tを渡る\tはし\t橋\n";
        assert_eq!(load_cases(with).unwrap()[0].right, "を渡る");
        let without = "id\tkind\tcontext\treading\texpected\nr-1\tk\t川に架かる\tはし\t橋\n";
        assert_eq!(load_cases(without).unwrap()[0].right, "");
    }
}
