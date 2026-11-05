//! `webwords lex`（レーン C。ADR 0055 §5.3 の 2）。一覧から辞書に足す lex.csv を作る。
//! ID は固定の数値を持たず、Sudachi の行から数える（§4.2）。
//!
//! 流れ: 一覧（schema=2）を読む → Sudachi・ABR の lex を 1 回走査して、(1) F1 の表記の集合、(2) F5 の部品（普通名詞のカタカナ語）、
//! (3) 区分ごとの左右 ID の最頻の組を作る → 一覧の各行に df の下限、区分の有効・無効、カタカナ、F1、形の規則（F2）、F5 を順に当てる
//! → 区分とコストを付けて Sudachi と同じ形の行にする。同じ入力と引数からは同じバイト列ができる。
//!
//! 段 1（ADR 0055 §4.2）は普通名詞（common）だけを出す。固有名詞の区分は、`proper_offsets` に区分を入れたときだけ出す（段 2。既定は無効）。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use super::{ListHeader, ListRow, MIN_DF_FLOOR, WebClass, WebwordsError};
use crate::lex::unescape_unicode;

#[derive(Debug, Clone)]
pub struct LexConfig {
    /// schema=2 の一覧
    pub list: PathBuf,
    /// Sudachi（small、core、notcore）と ABR の lex.csv（F1 をその場でもう一度当てる。ID も数える）
    pub lex: Vec<PathBuf>,
    /// 出力する `webwords_lex.csv`
    pub out: PathBuf,
    /// レポート（Markdown）の出力先
    pub report: Option<PathBuf>,
    /// df の下限（F2）。None なら `MIN_DF_FLOOR`
    pub min_df: Option<u32>,
    /// 普通名詞のコスト（掃引で決める。None なら既定）
    pub cost: Option<i16>,
    /// 区分ごとの出発点からの差（掃引用。§4.3）
    pub proper_offsets: Vec<(WebClass, i16)>,
    /// F5（連結を足すかの規則。0039 Q4）を当てない
    pub no_f5: bool,
}

/// 普通名詞のコストの既定（暫定）。PoC の候補 13000〜17000（守る群が動かない範囲。ADR 0055 §0.1）の中央付近。
/// 掃引（ADR 0055 §4.2）で決めるまでの仮の値で、ユーザーの確認のあとで置き換える。
pub const DEFAULT_COMMON_COST: i16 = 15000;

/// df の段ごとのコストの差（基準のコストからの差。暫定。PoC の 17000 / 14000 / 11000 の型 = 基準 15000 に対して +2000 / -1000 / -4000）。
/// `(その段の df の下限, 差)` を df の下限の昇順に並べる。df がその下限以上の語のうち、最も高い段の差を使う。
/// df が大きいほどよく使われる語とみなして安くする。値は掃引で決める。
const COST_TIERS: [(u32, i32); 3] = [(0, 2000), (20, -1000), (100, -4000)];

/// 形の規則（ADR 0039 §2.3 F3）: 表記の字数の下限（掃引の軸。PoC の既定は 3）
const MIN_SURFACE_CHARS: usize = 3;

/// Sudachi の lex.csv の列
const COLUMNS_MIN: usize = 19;
const COL_LEFT: usize = 1;
const COL_RIGHT: usize = 2;
const COL_SURFACE: usize = 4;
const COL_POS_FIRST: usize = 5;
const COL_POS_LAST: usize = 10;
const SPLIT_ONLY_ID: &str = "-1";
const CSV_SEPARATOR: char = ',';
const NO_VALUE: &str = "*";
/// 分割の部品にする語の最小の字数（1 字の語をつないだだけの連なりを F5 に当てない）
const MIN_PART_CHARS: usize = 2;

const POS_COMMON: &str = "名詞,普通名詞,一般,*,*,*";
const POS_PERSON: &str = "名詞,固有名詞,人名,一般,*,*";
const POS_PLACE: &str = "名詞,固有名詞,地名,一般,*,*";
const POS_PROPER_GENERAL: &str = "名詞,固有名詞,一般,*,*,*";
/// F5 の部品: 品詞の最初の 2 つ
const POS_COMMON_HEAD: [&str; 2] = ["名詞", "普通名詞"];

/// 区分ごとの品詞（lex.csv の 5〜10 列目。`名詞,…,*` の 6 つ）
fn pos_of(class: WebClass) -> &'static str {
    match class {
        WebClass::Common => POS_COMMON,
        WebClass::ProperPerson => POS_PERSON,
        WebClass::ProperPlace => POS_PLACE,
        WebClass::ProperGeneral => POS_PROPER_GENERAL,
    }
}

type IdPair = (u16, u16);

/// 理由ごとの件数（レポート用。ADR 0030 の TownReport と同じ型）
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LexReport {
    pub list_rows: u64,
    pub below_min_df: u64,
    pub class_disabled: BTreeMap<&'static str, u64>,
    pub not_katakana: u64,
    pub known_f1: u64,
    pub shape_f2: BTreeMap<&'static str, u64>,
    pub known_parts_f5: u64,
    pub duplicate: u64,
    pub written: BTreeMap<&'static str, u64>,
    pub ids: BTreeMap<&'static str, IdPair>,
    pub cost_tiers: BTreeMap<i16, u64>,
}

/// lex.csv の走査の結果
#[derive(Debug, Default)]
struct Scan {
    /// F1: カタカナだけの表記（品詞によらない）
    known: HashSet<String>,
    /// F5 の部品: 普通名詞のカタカナ語
    common_parts: HashSet<String>,
    /// 区分ごとの、カタカナだけの行の左右 ID の頻度
    id_counts: HashMap<WebClass, HashMap<IdPair, u64>>,
}

fn is_katakana_char(c: char) -> bool {
    ('ァ'..='ヺ').contains(&c) || c == 'ー'
}

fn is_katakana_run(text: &str) -> bool {
    !text.is_empty() && text.chars().all(is_katakana_char)
}

fn io_error(path: &Path, what: &str, e: impl std::fmt::Display) -> WebwordsError {
    WebwordsError::Io(format!("{what} {}: {e}", path.display()))
}

fn read_lines(path: &Path, mut each: impl FnMut(&str)) -> Result<(), WebwordsError> {
    let file = File::open(path).map_err(|e| io_error(path, "開けません", e))?;
    let mut reader = BufReader::with_capacity(1 << 20, file);
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader
            .read_line(&mut line)
            .map_err(|e| io_error(path, "読めません", e))?;
        if n == 0 {
            return Ok(());
        }
        each(line.trim_end_matches(['\n', '\r']));
    }
}

impl Scan {
    /// 1 行を数える。列が足りない行と、辞書に入らない行（左 ID が -1）は ID の数に入れない（F1 の表記には入れる）。
    fn add_line(&mut self, line: &str) {
        let cols: Vec<&str> = line.split(CSV_SEPARATOR).collect();
        if cols.len() < COLUMNS_MIN {
            return;
        }
        let surface = unescape_unicode(cols[COL_SURFACE]);
        if !is_katakana_run(&surface) {
            return;
        }
        self.known.insert(surface.clone().into_owned());
        let pos = &cols[COL_POS_FIRST..=COL_POS_LAST];
        if pos[..2] == POS_COMMON_HEAD {
            self.common_parts.insert(surface.into_owned());
        }
        if cols[COL_LEFT] == SPLIT_ONLY_ID {
            return;
        }
        let (Ok(left), Ok(right)) = (
            cols[COL_LEFT].parse::<u16>(),
            cols[COL_RIGHT].parse::<u16>(),
        ) else {
            return;
        };
        for class in WebClass::ALL {
            if pos.iter().copied().eq(pos_of(class).split(CSV_SEPARATOR)) {
                *self
                    .id_counts
                    .entry(class)
                    .or_default()
                    .entry((left, right))
                    .or_insert(0) += 1;
            }
        }
    }

    /// 区分の左右 ID。カタカナだけの行で最も多い組（同数なら小さい ID の組）。行がなければ None。
    fn ids(&self, class: WebClass) -> Option<IdPair> {
        self.id_counts
            .get(&class)?
            .iter()
            .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))
            .map(|(ids, _)| *ids)
    }
}

/// 読みが同じ塊の 2 回以上の繰り返し（ぷんぷん、ぽかぽか、ふーふー、ふふふ）か。
/// core の `katakana_like::is_reduplication` と同じ規則（dict-builder は core に依存しないので写す）。
fn is_reduplication(text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    (1..=n / 2)
        .any(|unit| n.is_multiple_of(unit) && chars.chunks(unit).all(|c| c == &chars[..unit]))
}

/// 形の規則（F2。ADR 0039 §2.3 の F3）。当たったら理由を返す。
fn shape_reject(surface: &str) -> Option<&'static str> {
    const SMALL_KANA: &str = "ァィゥェォッャュョヮ";
    let first = surface.chars().next()?;
    if surface.chars().count() < MIN_SURFACE_CHARS {
        return Some("short");
    }
    if first == 'ー' || SMALL_KANA.contains(first) {
        return Some("leading_prolonged_or_small");
    }
    if surface
        .chars()
        .any(|c| matches!(c, 'ヵ' | 'ヶ' | 'ヷ'..='ヺ'))
    {
        return Some("unsupported_char");
    }
    if is_reduplication(surface) {
        return Some("reduplication");
    }
    None
}

/// `surface` が、`parts` の語 2 個以上に分けられるか（F5。全体で 1 語になる分け方は数えない）。
fn segmentable(surface: &str, parts: &HashSet<String>) -> bool {
    let chars: Vec<char> = surface.chars().collect();
    let n = chars.len();
    if n == 0 {
        return false;
    }
    let mut reachable = vec![false; n + 1];
    reachable[0] = true;
    for end in MIN_PART_CHARS..=n {
        for start in 0..=end - MIN_PART_CHARS {
            if !reachable[start] || (start == 0 && end == n) {
                continue;
            }
            let part: String = chars[start..end].iter().collect();
            if parts.contains(&part) {
                reachable[end] = true;
                break;
            }
        }
    }
    reachable[n]
}

/// df から決める普通名詞のコスト（ADR 0055 §4.2 の段 1。暫定の式）。`base` を基準に [`COST_TIERS`] の差を足す。
fn common_cost(df: u64, base: i16) -> i16 {
    let delta = COST_TIERS
        .iter()
        .rev()
        .find(|(floor, _)| df >= u64::from(*floor))
        .map_or(0, |(_, delta)| *delta);
    clamp_cost(i32::from(base) + delta)
}

fn clamp_cost(cost: i32) -> i16 {
    i16::try_from(cost.clamp(i32::from(i16::MIN), i32::from(i16::MAX))).unwrap_or(i16::MAX)
}

/// Sudachi の lex.csv と同じ列の 1 行（表記・読みはカタカナのまま。ABR の `town` の行と同じ形）
fn lex_line(surface: &str, ids: IdPair, cost: i16, pos: &str) -> String {
    let (left, right) = ids;
    format!(
        "{surface},{left},{right},{cost},{surface},{pos},{surface},{surface},{NO_VALUE},A,{NO_VALUE},{NO_VALUE},{NO_VALUE},{NO_VALUE}\n"
    )
}

fn read_list(path: &Path) -> Result<(ListHeader, Vec<ListRow>), WebwordsError> {
    let mut header = None;
    let mut rows = Vec::new();
    let mut error = None;
    read_lines(path, |line| {
        if error.is_some() || line.is_empty() {
            return;
        }
        let result = if header.is_none() {
            ListHeader::parse(line).map(|h| header = Some(h))
        } else {
            ListRow::parse(line).map(|r| rows.push(r))
        };
        if let Err(e) = result {
            error = Some(e);
        }
    })?;
    if let Some(e) = error {
        return Err(e);
    }
    let header = header.ok_or_else(|| {
        WebwordsError::Format(format!(
            "{} に 1 行目（ヘッダ）がありません",
            path.display()
        ))
    })?;
    Ok((header, rows))
}

/// 行を作り、出力した行数を返す。
pub fn lex(config: &LexConfig) -> Result<u64, WebwordsError> {
    if config.lex.is_empty() {
        return Err(WebwordsError::Format(
            "--lex が 1 つも渡されていません（F1 と ID の数え方に要ります）".into(),
        ));
    }
    let min_df = u64::from(config.min_df.unwrap_or(MIN_DF_FLOOR));
    let base_cost = config.cost.unwrap_or(DEFAULT_COMMON_COST);
    let offsets: HashMap<WebClass, i16> = config.proper_offsets.iter().copied().collect();

    let (header, rows) = read_list(&config.list)?;

    let mut scan = Scan::default();
    for path in &config.lex {
        read_lines(path, |line| scan.add_line(line))?;
    }

    let mut report = LexReport::default();
    // 出す区分の ID。普通名詞は常に要る。固有名詞は有効にした区分だけ
    let mut ids: HashMap<WebClass, IdPair> = HashMap::new();
    for class in WebClass::ALL {
        if class.is_proper() && !offsets.contains_key(&class) {
            continue;
        }
        let pair = scan.ids(class).ok_or_else(|| {
            WebwordsError::Format(format!(
                "Sudachi の lex に `{}` のカタカナだけの行がなく、ID を数えられません",
                pos_of(class)
            ))
        })?;
        ids.insert(class, pair);
        report.ids.insert(class.as_str(), pair);
    }

    let mut sorted: Vec<&ListRow> = rows.iter().collect();
    sorted.sort_by(|a, b| a.surface.cmp(&b.surface));
    let mut seen: HashSet<&str> = HashSet::new();
    let mut text = String::new();
    for row in sorted {
        report.list_rows += 1;
        if row.df < min_df {
            report.below_min_df += 1;
            continue;
        }
        let Some(&class_ids) = ids.get(&row.class) else {
            *report.class_disabled.entry(row.class.as_str()).or_insert(0) += 1;
            continue;
        };
        if !is_katakana_run(&row.surface) {
            report.not_katakana += 1;
            continue;
        }
        if !seen.insert(&row.surface) {
            report.duplicate += 1;
            continue;
        }
        if scan.known.contains(&row.surface) {
            report.known_f1 += 1;
            continue;
        }
        if let Some(reason) = shape_reject(&row.surface) {
            *report.shape_f2.entry(reason).or_insert(0) += 1;
            continue;
        }
        if !config.no_f5 && segmentable(&row.surface, &scan.common_parts) {
            report.known_parts_f5 += 1;
            continue;
        }
        let common = common_cost(row.df, base_cost);
        let cost = match offsets.get(&row.class) {
            Some(offset) => clamp_cost(i32::from(common) + i32::from(*offset)),
            None => common,
        };
        text.push_str(&lex_line(&row.surface, class_ids, cost, pos_of(row.class)));
        *report.written.entry(row.class.as_str()).or_insert(0) += 1;
        *report.cost_tiers.entry(cost).or_insert(0) += 1;
    }

    write_atomically(&config.out, &text)?;
    if let Some(path) = &config.report {
        write_atomically(
            path,
            &render_report(config, &header, &report, min_df, base_cost),
        )?;
    }
    Ok(report.written.values().sum())
}

/// 一時ファイルに書いて rename する
fn write_atomically(out: &Path, text: &str) -> Result<(), WebwordsError> {
    let mut tmp = out.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    fs::write(&tmp, text)
        .and_then(|()| fs::rename(&tmp, out))
        .map_err(|e| io_error(out, "書けません", e))
}

fn render_report(
    config: &LexConfig,
    header: &ListHeader,
    report: &LexReport,
    min_df: u64,
    base_cost: i16,
) -> String {
    let mut out = String::new();
    out.push_str("# webwords lex のレポート（ADR 0055 §5.3 の 2）\n\n");
    out.push_str(&format!(
        "- 一覧: {}（sha256 {}、記事数 {}、F1 に使った Sudachi の版 {}、規則表 {}）\n",
        header.source, header.sha256, header.articles, header.sudachi, header.rules
    ));
    out.push_str(&format!(
        "- 引数: df の下限 {min_df}、普通名詞のコストの基準 {base_cost}（暫定。段は df に応じて {:?} の差）、F5 {}\n",
        COST_TIERS.map(|(floor, delta)| (floor, delta)),
        if config.no_f5 { "なし" } else { "あり" }
    ));
    out.push_str(&format!(
        "- 固有名詞の区分: {}\n\n",
        if config.proper_offsets.is_empty() {
            "無効（段 1）".to_owned()
        } else {
            config
                .proper_offsets
                .iter()
                .map(|(c, v)| format!("{}={v}", c.as_str()))
                .collect::<Vec<_>>()
                .join("、")
        }
    ));
    out.push_str("## 左右 ID（Sudachi のカタカナだけの行の最頻の組）\n\n| 区分 | 左 | 右 |\n| --- | --- | --- |\n");
    for (class, (left, right)) in &report.ids {
        out.push_str(&format!("| {class} | {left} | {right} |\n"));
    }
    out.push_str("\n## 理由ごとの件数\n\n| 理由 | 件数 |\n| --- | --- |\n");
    let mut line = |name: &str, n: u64| out.push_str(&format!("| {name} | {n} |\n"));
    line("一覧の行", report.list_rows);
    line("df が下限未満（F2）", report.below_min_df);
    for (class, n) in &report.class_disabled {
        line(&format!("無効の区分 {class}"), *n);
    }
    line("カタカナだけではない", report.not_katakana);
    line("一覧の重複", report.duplicate);
    line("辞書にある（F1）", report.known_f1);
    for (reason, n) in &report.shape_f2 {
        line(&format!("形の規則（{reason}）"), *n);
    }
    line("既知の普通名詞の連なり（F5）", report.known_parts_f5);
    for (class, n) in &report.written {
        line(&format!("出力 {class}"), *n);
    }
    out.push_str("\n## コストごとの行数\n\n| コスト | 行数 |\n| --- | --- |\n");
    for (cost, n) in &report.cost_tiers {
        out.push_str(&format!("| {cost} | {n} |\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const SUDACHI_COMMON: IdPair = (5144, 5144);

    fn sudachi_line(surface: &str, ids: IdPair, pos: &str) -> String {
        format!(
            "{surface},{},{},2000,{surface},{pos},{surface},{surface},*,A,*,*,*,*",
            ids.0, ids.1
        )
    }

    fn sudachi_text() -> String {
        [
            // 普通名詞のカタカナ（最頻は 5144/5144。2 行）と別の組（1 行）
            sudachi_line("サーバー", SUDACHI_COMMON, POS_COMMON),
            sudachi_line("アーキテクチャ", SUDACHI_COMMON, POS_COMMON),
            sudachi_line("マイクロ", (5139, 5139), POS_COMMON),
            // 漢字の行は ID の数に入れない
            sudachi_line("漢字", (1, 1), POS_COMMON),
            // 固有名詞（人名は 4787/4788、地名は 4791/4792、一般は 4785）
            sudachi_line("ジョン", (4787, 4788), POS_PERSON),
            sudachi_line("ロンドン", (4791, 4792), POS_PLACE),
            sudachi_line("ソニー", (4785, 4785), POS_PROPER_GENERAL),
            // 左 ID が -1 の行は ID に数えないが、F1 の表記には入る
            sudachi_line("ブロック", (0, 0), POS_COMMON).replacen(",0,0,", ",-1,-1,", 1),
            // 品詞が違う行（ID の数に入らない）
            sudachi_line("ミナト", (9, 9), "名詞,普通名詞,サ変可能,*,*,*"),
        ]
        .join("\n")
            + "\n"
    }

    fn header_line() -> String {
        ListHeader {
            source: "jawiki-20261001".into(),
            sha256: "00".repeat(32),
            articles: 100,
            sudachi: "20260723".into(),
            rules: "11".repeat(32),
        }
        .format()
    }

    struct Fixture {
        dir: tempfile::TempDir,
    }

    impl Fixture {
        fn new(list_rows: &[(&str, u64, WebClass)]) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let mut list = header_line() + "\n";
            for (surface, df, class) in list_rows {
                list.push_str(
                    &ListRow {
                        surface: (*surface).to_owned(),
                        tf: df * 2,
                        df: *df,
                        class: *class,
                    }
                    .format(),
                );
                list.push('\n');
            }
            fs::write(dir.path().join("list.tsv"), list).unwrap();
            fs::write(dir.path().join("sudachi.csv"), sudachi_text()).unwrap();
            Self { dir }
        }

        fn config(&self) -> LexConfig {
            LexConfig {
                list: self.dir.path().join("list.tsv"),
                lex: vec![self.dir.path().join("sudachi.csv")],
                out: self.dir.path().join("out.csv"),
                report: Some(self.dir.path().join("report.md")),
                min_df: None,
                cost: None,
                proper_offsets: vec![],
                no_f5: false,
            }
        }

        fn out_lines(&self) -> Vec<String> {
            fs::read_to_string(self.dir.path().join("out.csv"))
                .unwrap()
                .lines()
                .map(str::to_owned)
                .collect()
        }
    }

    #[test]
    fn writes_common_rows_with_counted_ids_and_df_cost() {
        let fixture = Fixture::new(&[
            ("バージョニング", 16, WebClass::Common),
            ("シャーディング", 150, WebClass::Common),
            ("オブザーバビリティ", 25, WebClass::Common),
        ]);
        let written = lex(&fixture.config()).unwrap();
        assert_eq!(written, 3);
        assert_eq!(
            fixture.out_lines(),
            [
                "オブザーバビリティ,5144,5144,14000,オブザーバビリティ,名詞,普通名詞,一般,*,*,*,オブザーバビリティ,オブザーバビリティ,*,A,*,*,*,*",
                "シャーディング,5144,5144,11000,シャーディング,名詞,普通名詞,一般,*,*,*,シャーディング,シャーディング,*,A,*,*,*,*",
                "バージョニング,5144,5144,17000,バージョニング,名詞,普通名詞,一般,*,*,*,バージョニング,バージョニング,*,A,*,*,*,*",
            ]
        );
        // 19 列で、Sudachi と同じ列数
        assert!(
            fixture
                .out_lines()
                .iter()
                .all(|l| l.split(',').count() == 19)
        );
        let report = fs::read_to_string(fixture.dir.path().join("report.md")).unwrap();
        assert!(report.contains("| common | 5144 | 5144 |"));
        assert!(report.contains("暫定"));
    }

    #[test]
    fn id_is_counted_from_the_lex_not_fixed() {
        // 最頻の組が 5139 になるような Sudachi では、5139 が付く
        let fixture = Fixture::new(&[("バージョニング", 16, WebClass::Common)]);
        let mut sudachi = String::new();
        for surface in ["アルファ", "ベータ", "ガンマ"] {
            sudachi.push_str(&sudachi_line(surface, (5139, 5139), POS_COMMON));
            sudachi.push('\n');
        }
        sudachi.push_str(&sudachi_line("デルタ", (5144, 5144), POS_COMMON));
        fs::write(fixture.dir.path().join("sudachi.csv"), sudachi).unwrap();
        lex(&fixture.config()).unwrap();
        assert!(fixture.out_lines()[0].starts_with("バージョニング,5139,5139,"));
    }

    #[test]
    fn id_tie_goes_to_the_smaller_pair() {
        let mut scan = Scan::default();
        scan.add_line(&sudachi_line("アルファ", (5144, 5144), POS_COMMON));
        scan.add_line(&sudachi_line("ベータ", (5139, 5139), POS_COMMON));
        assert_eq!(scan.ids(WebClass::Common), Some((5139, 5139)));
        assert_eq!(scan.ids(WebClass::ProperPerson), None);
    }

    #[test]
    fn proper_classes_are_not_written_by_default() {
        let fixture = Fixture::new(&[
            ("バージョニング", 16, WebClass::Common),
            ("マイケルジョーダン", 50, WebClass::ProperPerson),
            ("ブルックリン", 50, WebClass::ProperPlace),
            ("プレイステーション", 50, WebClass::ProperGeneral),
        ]);
        let written = lex(&fixture.config()).unwrap();
        assert_eq!(written, 1);
        assert_eq!(fixture.out_lines().len(), 1);
        let report = fs::read_to_string(fixture.dir.path().join("report.md")).unwrap();
        assert!(report.contains("無効の区分 proper.person"));
        assert!(report.contains("無効（段 1）"));
    }

    #[test]
    fn proper_classes_are_written_only_when_enabled_with_their_own_ids_and_offset() {
        let fixture = Fixture::new(&[
            ("マイケルジョーダン", 16, WebClass::ProperPerson),
            ("ブルックリン", 16, WebClass::ProperPlace),
            ("プレイステーション", 16, WebClass::ProperGeneral),
        ]);
        let mut config = fixture.config();
        config.proper_offsets = vec![
            (WebClass::ProperPerson, 2000),
            (WebClass::ProperPlace, -500),
        ];
        let written = lex(&config).unwrap();
        // proper.general は有効にしていないので出ない
        assert_eq!(written, 2);
        let lines = fixture.out_lines();
        assert_eq!(
            lines[0],
            "ブルックリン,4791,4792,16500,ブルックリン,名詞,固有名詞,地名,一般,*,*,ブルックリン,ブルックリン,*,A,*,*,*,*"
        );
        assert!(lines[1].starts_with("マイケルジョーダン,4787,4788,19000,"));
        assert!(lines[1].contains(",名詞,固有名詞,人名,一般,*,*,"));
    }

    #[test]
    fn enabling_a_class_sudachi_has_no_rows_for_is_an_error() {
        let fixture = Fixture::new(&[("バージョニング", 16, WebClass::Common)]);
        fs::write(
            fixture.dir.path().join("sudachi.csv"),
            sudachi_line("サーバー", SUDACHI_COMMON, POS_COMMON) + "\n",
        )
        .unwrap();
        let mut config = fixture.config();
        config.proper_offsets = vec![(WebClass::ProperPerson, 0)];
        assert!(lex(&config).is_err());
        config.proper_offsets = vec![];
        assert_eq!(lex(&config).unwrap(), 1);
    }

    #[test]
    fn f1_drops_words_in_sudachi_including_split_only_rows() {
        let fixture = Fixture::new(&[
            ("サーバー", 90, WebClass::Common),
            ("ブロック", 90, WebClass::Common),
            ("ミナト", 90, WebClass::Common),
            ("バージョニング", 90, WebClass::Common),
        ]);
        lex(&fixture.config()).unwrap();
        assert_eq!(fixture.out_lines().len(), 1);
        let report = fs::read_to_string(fixture.dir.path().join("report.md")).unwrap();
        assert!(report.contains("| 辞書にある（F1） | 3 |"));
    }

    #[test]
    fn f2_drops_low_df_and_bad_shapes() {
        let fixture = Fixture::new(&[
            ("バージョニング", 4, WebClass::Common),
            ("ビビ", 9, WebClass::Common),
            ("ーテスト", 9, WebClass::Common),
            ("ァテスト", 9, WebClass::Common),
            ("ドドドド", 9, WebClass::Common),
            ("プンプン", 9, WebClass::Common),
            ("テヵスト", 9, WebClass::Common),
            ("ヷイオリン", 9, WebClass::Common),
            ("コンテナオーケストレーション", 5, WebClass::Common),
        ]);
        lex(&fixture.config()).unwrap();
        let lines = fixture.out_lines();
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].starts_with("コンテナオーケストレーション,"));
        let report = fs::read_to_string(fixture.dir.path().join("report.md")).unwrap();
        assert!(report.contains("| df が下限未満（F2） | 1 |"));
        assert!(report.contains("形の規則（reduplication）"));
        assert!(report.contains("形の規則（short） | 1 |"));
    }

    #[test]
    fn min_df_can_be_lowered_for_sweeps() {
        let fixture = Fixture::new(&[("バージョニング", 3, WebClass::Common)]);
        let mut config = fixture.config();
        assert_eq!(lex(&config).unwrap(), 0);
        config.min_df = Some(3);
        assert_eq!(lex(&config).unwrap(), 1);
    }

    #[test]
    fn f5_drops_words_made_of_known_common_nouns_unless_disabled() {
        let fixture = Fixture::new(&[
            // サーバー + アーキテクチャ（普通名詞 2 個）
            ("サーバーアーキテクチャ", 40, WebClass::Common),
            // 部品が 1 個だけ足りない
            ("サーバーデプロイ", 40, WebClass::Common),
            // ジョンは固有名詞なので部品にならない
            ("ジョンサーバー", 40, WebClass::Common),
        ]);
        let mut config = fixture.config();
        lex(&config).unwrap();
        let surfaces: Vec<String> = fixture
            .out_lines()
            .iter()
            .map(|l| l.split(',').next().unwrap().to_owned())
            .collect();
        assert_eq!(surfaces, ["サーバーデプロイ", "ジョンサーバー"]);
        config.no_f5 = true;
        assert_eq!(lex(&config).unwrap(), 3);
    }

    #[test]
    fn segmentable_needs_two_or_more_parts() {
        let parts: HashSet<String> = ["サーバー", "アーキテクチャ"].map(String::from).into();
        assert!(segmentable("サーバーアーキテクチャ", &parts));
        assert!(!segmentable("サーバー", &parts));
        assert!(!segmentable("サーバーアー", &parts));
        assert!(!segmentable("", &parts));
    }

    #[test]
    fn cost_follows_the_df_tiers_and_the_base() {
        assert_eq!(common_cost(5, 15000), 17000);
        assert_eq!(common_cost(19, 15000), 17000);
        assert_eq!(common_cost(20, 15000), 14000);
        assert_eq!(common_cost(99, 15000), 14000);
        assert_eq!(common_cost(100, 15000), 11000);
        assert_eq!(common_cost(100_000, 13000), 9000);
        // 基準を変えると全段が動く。i16 を超えない
        assert_eq!(common_cost(5, i16::MAX), i16::MAX);
        assert_eq!(common_cost(100, i16::MIN), i16::MIN);
        assert_eq!(DEFAULT_COMMON_COST, 15000);
    }

    #[test]
    fn non_katakana_and_duplicate_rows_are_skipped_and_output_is_deterministic() {
        let fixture = Fixture::new(&[
            ("バージョニング", 16, WebClass::Common),
            ("ばーじょにんぐ", 16, WebClass::Common),
            ("バージョニング", 16, WebClass::Common),
        ]);
        assert_eq!(lex(&fixture.config()).unwrap(), 1);
        let first = fs::read(fixture.dir.path().join("out.csv")).unwrap();
        assert_eq!(lex(&fixture.config()).unwrap(), 1);
        assert_eq!(first, fs::read(fixture.dir.path().join("out.csv")).unwrap());
    }

    #[test]
    fn bad_inputs_are_errors() {
        let fixture = Fixture::new(&[("バージョニング", 16, WebClass::Common)]);
        let mut config = fixture.config();
        config.lex = vec![];
        assert!(lex(&config).is_err());
        let mut config = fixture.config();
        config.list = fixture.dir.path().join("missing.tsv");
        assert!(lex(&config).is_err());
        fs::write(fixture.dir.path().join("bad.tsv"), "not a header\n").unwrap();
        let mut config = fixture.config();
        config.list = fixture.dir.path().join("bad.tsv");
        assert!(lex(&config).is_err());
        // ID が数えられない Sudachi（普通名詞のカタカナ行がない）
        fs::write(fixture.dir.path().join("empty.csv"), "").unwrap();
        let mut config = fixture.config();
        config.lex = vec![fixture.dir.path().join("empty.csv")];
        assert!(lex(&config).is_err());
    }

    #[test]
    fn escaped_unicode_in_sudachi_surfaces_is_matched() {
        let mut scan = Scan::default();
        scan.add_line(&sudachi_line(
            "\\u30b5\\u30fc\\u30d0\\u30fc",
            SUDACHI_COMMON,
            POS_COMMON,
        ));
        assert!(scan.known.contains("サーバー"));
    }
}
