//! `webwords classify`（レーン B。ADR 0055 §2、§5.1 の 3）。§2.2 の行 0〜9 を当て、schema=2 の一覧を作る。
//! SQL ダンプの読み込みは同じレーンの `sqldump.rs`。
//!
//! 流れ: Sudachi（F1 と F4c の部品）と収集の一覧を読み、F1・F3 を通った候補だけを残す → page（題 → ID、リダイレクトの印、
//! 括弧の補足つきの題）→ redirect（行き先を題で引く）→ page_props（曖昧さ回避）→ categorylinks（候補の記事の分だけ）
//! → linktarget（その分だけ）の順に読み、候補ごとに区分を決める。
//!
//! 判断（2026-11-06 ユーザー決定、ADR 0055 §0.4）: redirect の行がない記事と、3 段たどっても記事に着かないものは「迷う」（入れない）。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;

use sha2::{Digest, Sha256};

use super::rules::RuleTable;
use super::sqldump::{
    NS_CATEGORY, NS_MAIN, PROP_DISAMBIGUATION, for_each_tuple_line, parse_category_link,
    parse_link_target, parse_page, parse_page_prop, parse_redirect,
};
use super::{CollectedRow, DEFAULT_CATEGORIES_TEXT, Division, ListRow, WebClass, WebwordsError};

use std::path::PathBuf;

use super::ListHeader;

/// リダイレクトをたどる段数の上限（ADR 0055 §2.1）
const MAX_REDIRECT_HOPS: usize = 3;
/// F3: 表記の字数の下限（読みはカタカナのままなので同じ。ADR 0039 §2.3。掃引の軸だが、出力の一覧は下限の最小で作る）
const MIN_SURFACE_CHARS: usize = 3;
/// F4b: 連なりの直後に固有名詞の手がかりが付いた割合（cue / tf）がこれ以上なら目印に当たる。
/// 出発点の値（researcher の試作と同じ）。§4.1 の代理の測定で決め直す
const CUE_RATIO_THRESHOLD: f64 = 0.2;
/// F4c: 固有名詞だけの Sudachi の表記に分けるときの 1 つの部分の最小の字数
const MIN_PROPER_PART_CHARS: usize = 2;
/// 括弧の補足つきの題（「バージョン（曖昧さ回避）」）の括弧
const PAREN_OPEN: char = '（';
const PAREN_CLOSE: char = '）';
/// Sudachi の CSV の品詞の第 2 階層の列（0 始まり）と、固有名詞の名前
const LEX_POS2_COLUMN: usize = 6;
const LEX_PROPER_NOUN: &str = "固有名詞";
/// 形の規則で除く文字: 小書きのカタカナで始まる、ヵ ヶ ヷ〜ヺ を含む
const SMALL_KANA: &str = "ァィゥェォッャュョヮ";
const LONG_MARK: char = 'ー';

/// 区分の理由のキー（レポートの行。ADR 0030 の TownReport と同じく、理由ごとの件数）
pub mod reason {
    pub const BELOW_MIN_DF: &str = "df が下限より小さい（対象外）";
    pub const KNOWN: &str = "0 既知（Sudachi・ABR に同じ表記）";
    pub const SHAPE_SHORT: &str = "1 形: 字数が短い";
    pub const SHAPE_LEAD: &str = "1 形: ー・小書きで始まる";
    pub const SHAPE_BAD_CHAR: &str = "1 形: ヵヶヷ〜ヺ を含む";
    pub const SHAPE_REDUP: &str = "1 形: 反復";
    pub const COMMON_NO_ARTICLE: &str = "2 普通名詞: 記事なし、目印なし";
    pub const NO_ARTICLE_MARKER: &str = "3 迷う: 記事なし、固有名詞の目印あり";
    pub const DISAMBIGUATION: &str = "4 迷う: 曖昧さ回避";
    pub const REDIRECT_NO_ROW: &str = "4 迷う: リダイレクトの行がない";
    pub const REDIRECT_UNRESOLVED: &str = "4 迷う: 3 段で記事に着かない";
    pub const PERSON: &str = "5 固有名詞・人名（カテゴリ）";
    pub const PLACE: &str = "6 固有名詞・地名（カテゴリ）";
    pub const GENERAL: &str = "7 固有名詞・一般（カテゴリ）";
    pub const COMMON_CATEGORY: &str = "8 普通名詞（カテゴリ）";
    pub const MULTI_CLASS: &str = "9 迷う: 複数の区分に当たる";
    pub const NO_CATEGORY_MATCH: &str = "9 迷う: どの規則にも当たらない";
}

#[derive(Debug, Clone)]
pub struct ClassifyConfig {
    /// `collect` の出力
    pub collected: PathBuf,
    /// jawiki の SQL ダンプ（同じ日付）。page、redirect、page_props、categorylinks、linktarget
    pub page_sql: PathBuf,
    pub redirect_sql: PathBuf,
    pub page_props_sql: PathBuf,
    pub categorylinks_sql: PathBuf,
    pub linktarget_sql: PathBuf,
    /// カテゴリの規則表。None なら同梱の `DEFAULT_CATEGORIES_TEXT`
    pub categories: Option<PathBuf>,
    /// Sudachi（small、core、notcore）と ABR の lex.csv（F1 の照合）
    pub lex: Vec<PathBuf>,
    /// 一覧の 1 行目に書く値（`rules` は規則表から計算するのでここでは使わない）
    pub header: ListHeader,
    /// df の下限（既定は `MIN_DF_FLOOR`）
    pub min_df: u32,
    /// schema=2 の一覧の出力先
    pub out: PathBuf,
    /// レポート（Markdown。理由ごとの件数と §4.1 の代理の測定）の出力先
    pub report: Option<PathBuf>,
}

/// 判定の結果の件数（ADR 0030 の TownReport と同じ型: 理由ごとの件数と、書いた数）
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClassifyReport {
    /// 読んだ収集の行数（表記の種類）
    pub rows_read: u64,
    /// 理由ごとの件数（キーは [`reason`]）
    pub reasons: BTreeMap<&'static str, u64>,
    /// 一覧に書いた行数
    pub written: u64,
    /// 一覧の区分ごとの行数
    pub by_class: BTreeMap<&'static str, u64>,
}

impl ClassifyReport {
    fn count(&mut self, key: &'static str) {
        *self.reasons.entry(key).or_insert(0) += 1;
    }

    /// Markdown のレポート
    pub fn to_markdown(&self, header: &ListHeader) -> String {
        let mut text = String::new();
        let _ = writeln!(text, "# webwords classify のレポート\n");
        let _ = writeln!(
            text,
            "- source: {}\n- sudachi: {}\n- rules: {}\n- 読んだ表記: {}\n- 一覧に書いた行: {}\n",
            header.source, header.sudachi, header.rules, self.rows_read, self.written
        );
        let _ = writeln!(text, "## 理由ごとの件数\n\n| 理由 | 件数 |\n| --- | ---: |");
        for (key, count) in &self.reasons {
            let _ = writeln!(text, "| {key} | {count} |");
        }
        let _ = writeln!(
            text,
            "\n## 一覧の区分ごとの行数\n\n| 区分 | 行数 |\n| --- | ---: |"
        );
        for (key, count) in &self.by_class {
            let _ = writeln!(text, "| {key} | {count} |");
        }
        text
    }
}

fn io_error(path: &Path, error: impl std::fmt::Display) -> WebwordsError {
    WebwordsError::Io(format!("{}: {error}", path.display()))
}

fn is_katakana_run(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|c| ('ァ'..='ヺ').contains(&c) || c == LONG_MARK)
}

/// Sudachi・ABR の lex.csv から、カタカナだけの表記の集合（F1）と、固有名詞の行だけがある表記の集合（F4c）。
#[derive(Debug, Default)]
struct Lexicon {
    all: HashSet<String>,
    proper_only: HashSet<String>,
}

impl Lexicon {
    fn load(paths: &[PathBuf]) -> Result<Self, WebwordsError> {
        let mut all = HashSet::new();
        let mut proper = HashSet::new();
        let mut other = HashSet::new();
        for path in paths {
            let file = File::open(path).map_err(|e| io_error(path, e))?;
            for line in BufReader::new(file).lines() {
                let line = line.map_err(|e| io_error(path, e))?;
                // 先頭の列がカタカナだけなら、品詞までの列に引用符はない
                let mut fields = line.splitn(LEX_POS2_COLUMN + 2, ',');
                let Some(surface) = fields.next() else {
                    continue;
                };
                if !is_katakana_run(surface) {
                    continue;
                }
                let Some(pos2) = fields.nth(LEX_POS2_COLUMN - 1) else {
                    continue;
                };
                all.insert(surface.to_owned());
                if pos2 == LEX_PROPER_NOUN {
                    proper.insert(surface.to_owned());
                } else {
                    other.insert(surface.to_owned());
                }
            }
        }
        let proper_only = proper.difference(&other).cloned().collect();
        Ok(Self { all, proper_only })
    }
}

/// F3 形の規則（ADR 0039 §2.3）。除くなら理由。
fn shape_reason(surface: &str) -> Option<&'static str> {
    let chars: Vec<char> = surface.chars().collect();
    if chars.len() < MIN_SURFACE_CHARS {
        return Some(reason::SHAPE_SHORT);
    }
    if chars[0] == LONG_MARK || SMALL_KANA.contains(chars[0]) {
        return Some(reason::SHAPE_LEAD);
    }
    if chars.iter().any(|c| matches!(c, 'ヵ' | 'ヶ' | 'ヷ'..='ヺ')) {
        return Some(reason::SHAPE_BAD_CHAR);
    }
    if is_reduplication(&chars) {
        return Some(reason::SHAPE_REDUP);
    }
    None
}

/// 反復（ドドドド、ワクワク）: 前半と後半が同じ（4 字以上の偶数）か、同じ字だけ（3 字以上）
fn is_reduplication(chars: &[char]) -> bool {
    let n = chars.len();
    if n >= 4 && n.is_multiple_of(2) && chars[..n / 2] == chars[n / 2..] {
        return true;
    }
    n >= 3 && chars.iter().all(|c| *c == chars[0])
}

/// `surface` を `parts` の語 2 つ以上に分けられるか（全体で 1 語なのは数えない）
fn segmentable(surface: &str, parts: &HashSet<String>, min_part: usize) -> bool {
    let chars: Vec<char> = surface.chars().collect();
    let n = chars.len();
    let mut ok = vec![false; n + 1];
    ok[0] = true;
    for end in min_part..=n {
        for start in 0..=end.saturating_sub(min_part) {
            if ok[start]
                && !(start == 0 && end == n)
                && parts.contains(&chars[start..end].iter().collect::<String>())
            {
                ok[end] = true;
                break;
            }
        }
    }
    ok[n]
}

/// 括弧の補足を外した題（「バージョン（曖昧さ回避）」→「バージョン」）。括弧で終わらない題は None
fn strip_paren_suffix(title: &str) -> Option<&str> {
    let body = title.strip_suffix(PAREN_CLOSE)?;
    let open = body.rfind(PAREN_OPEN)?;
    (!body[open..].contains(PAREN_CLOSE)).then_some(&body[..open])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Resolution {
    /// リダイレクトでない記事に着いた
    Article(u32),
    /// リダイレクトの印があるのに redirect の行がない
    NoRow,
    /// 3 段たどっても記事に着かない（ループを含む）。行き先が記事でない（題がない、他の名前空間・他のウィキ）ものは行がないのと同じ扱い（`NoRow`）
    Unresolved,
}

/// page + redirect + page_props から作る、記事の索引。
#[derive(Debug, Default)]
struct Wiki {
    /// 名前空間 0 の題 → ID
    id_by_title: HashMap<String, u32>,
    /// 名前空間 0 の ID → page のリダイレクトの印
    redirect_flag: HashMap<u32, bool>,
    /// リダイレクトの ID → 行き先の ID
    redirect_to: HashMap<u32, u32>,
    /// 曖昧さ回避の印がある ID
    disambiguation: HashSet<u32>,
    /// 候補の表記 → その表記を括弧の補足で外した題の記事の ID
    paren_ids: HashMap<String, Vec<u32>>,
}

impl Wiki {
    fn is_redirect(&self, id: u32) -> bool {
        self.redirect_flag.get(&id).copied().unwrap_or(false) || self.redirect_to.contains_key(&id)
    }

    fn resolve(&self, id: u32) -> Resolution {
        let mut current = id;
        for _ in 0..MAX_REDIRECT_HOPS {
            if !self.is_redirect(current) {
                return Resolution::Article(current);
            }
            match self.redirect_to.get(&current) {
                Some(next) => current = *next,
                None => return Resolution::NoRow,
            }
        }
        if self.is_redirect(current) {
            Resolution::Unresolved
        } else {
            Resolution::Article(current)
        }
    }

    fn load(
        config: &ClassifyConfig,
        candidates: &HashMap<String, CollectedRow>,
    ) -> Result<Self, WebwordsError> {
        let mut wiki = Self::default();
        for_each_tuple_line(&config.page_sql, |line| {
            let Some(row) = parse_page(line) else { return };
            if row.namespace != NS_MAIN {
                return;
            }
            if let Some(base) = strip_paren_suffix(&row.title)
                && candidates.contains_key(base)
            {
                wiki.paren_ids
                    .entry(base.to_owned())
                    .or_default()
                    .push(row.id);
            }
            wiki.redirect_flag.insert(row.id, row.is_redirect);
            wiki.id_by_title.insert(row.title, row.id);
        })?;
        for_each_tuple_line(&config.redirect_sql, |line| {
            let Some(row) = parse_redirect(line) else {
                return;
            };
            if row.namespace != NS_MAIN || row.interwiki {
                return;
            }
            if !wiki.redirect_flag.contains_key(&row.from) {
                return;
            }
            if let Some(target) = wiki.id_by_title.get(&row.title) {
                wiki.redirect_to.insert(row.from, *target);
            }
        })?;
        for_each_tuple_line(&config.page_props_sql, |line| {
            if let Some((id, name)) = parse_page_prop(line)
                && name == PROP_DISAMBIGUATION
                && wiki.redirect_flag.contains_key(&id)
            {
                wiki.disambiguation.insert(id);
            }
        })?;
        Ok(wiki)
    }
}

/// 候補ごとの、記事の手がかり
struct ArticleRefs {
    /// 同じ表記の題の記事（リダイレクトは行き先を解いたもの）
    own: Option<Resolution>,
    /// 括弧の補足つきの題の記事（同じく解いたもの）
    paren: Vec<Resolution>,
}

impl ArticleRefs {
    fn find(wiki: &Wiki, surface: &str) -> Self {
        Self {
            own: wiki.id_by_title.get(surface).map(|id| wiki.resolve(*id)),
            paren: wiki
                .paren_ids
                .get(surface)
                .map(|ids| ids.iter().map(|id| wiki.resolve(*id)).collect())
                .unwrap_or_default(),
        }
    }

    fn has_article(&self) -> bool {
        self.own.is_some() || !self.paren.is_empty()
    }

    fn resolutions(&self) -> impl Iterator<Item = Resolution> + '_ {
        self.own.into_iter().chain(self.paren.iter().copied())
    }

    fn article_ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.resolutions().filter_map(|r| match r {
            Resolution::Article(id) => Some(id),
            _ => None,
        })
    }
}

/// F4b・F4c: 記事のない語に当てる、固有名詞の目印（§2.4）
fn has_proper_marker(row: &CollectedRow, lexicon: &Lexicon) -> bool {
    let cue_ratio = if row.tf == 0 {
        0.0
    } else {
        row.cue as f64 / row.tf as f64
    };
    cue_ratio >= CUE_RATIO_THRESHOLD
        || segmentable(&row.surface, &lexicon.proper_only, MIN_PROPER_PART_CHARS)
}

fn read_collected(path: &Path) -> Result<HashMap<String, CollectedRow>, WebwordsError> {
    let file = File::open(path).map_err(|e| io_error(path, e))?;
    let mut rows: HashMap<String, CollectedRow> = HashMap::new();
    for line in BufReader::new(file).lines() {
        let line = line.map_err(|e| io_error(path, e))?;
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let row = CollectedRow::parse(&line)?;
        match rows.get_mut(&row.surface) {
            Some(existing) => {
                existing.tf += row.tf;
                existing.df += row.df;
                existing.cue += row.cue;
            }
            None => {
                rows.insert(row.surface.clone(), row);
            }
        }
    }
    Ok(rows)
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// カテゴリで区分を決める（行 5〜9）。理由つき
fn classify_by_categories(categories: &[&str], rules: &RuleTable) -> (Division, &'static str) {
    let mut proper: Vec<WebClass> = Vec::new();
    let mut common = false;
    for category in categories {
        for class in rules.classes_of(category) {
            if class == WebClass::Common {
                common = true;
            } else if !proper.contains(&class) {
                proper.push(class);
            }
        }
    }
    match (proper.as_slice(), common) {
        ([], false) => (Division::Uncertain, reason::NO_CATEGORY_MATCH),
        ([], true) => (Division::Class(WebClass::Common), reason::COMMON_CATEGORY),
        ([WebClass::ProperPerson], false) => {
            (Division::Class(WebClass::ProperPerson), reason::PERSON)
        }
        ([WebClass::ProperPlace], false) => (Division::Class(WebClass::ProperPlace), reason::PLACE),
        ([WebClass::ProperGeneral], false) => {
            (Division::Class(WebClass::ProperGeneral), reason::GENERAL)
        }
        _ => (Division::Uncertain, reason::MULTI_CLASS),
    }
}

/// 判定を行い、一覧に出した行数を返す。
pub fn classify(config: &ClassifyConfig) -> Result<u64, WebwordsError> {
    classify_with_report(config).map(|report| report.written)
}

/// [`classify`] と同じ。理由ごとの件数も返す。
pub fn classify_with_report(config: &ClassifyConfig) -> Result<ClassifyReport, WebwordsError> {
    let mut report = ClassifyReport::default();

    let rules_text = match &config.categories {
        Some(path) => std::fs::read_to_string(path).map_err(|e| io_error(path, e))?,
        None => DEFAULT_CATEGORIES_TEXT.to_owned(),
    };
    let rules = RuleTable::parse(&rules_text)?;
    let mut header = config.header.clone();
    header.rules = sha256_hex(rules_text.as_bytes());

    let lexicon = Lexicon::load(&config.lex)?;
    let collected = read_collected(&config.collected)?;
    report.rows_read = collected.len() as u64;

    // 行 0（F1）、行 1（F3）を当て、残りを候補にする
    let mut candidates: HashMap<String, CollectedRow> = HashMap::new();
    for (surface, row) in collected {
        if row.df < u64::from(config.min_df) {
            report.count(reason::BELOW_MIN_DF);
        } else if lexicon.all.contains(&surface) {
            report.count(reason::KNOWN);
        } else if let Some(why) = shape_reason(&surface) {
            report.count(why);
        } else {
            candidates.insert(surface, row);
        }
    }

    let wiki = Wiki::load(config, &candidates)?;
    let refs: HashMap<&str, ArticleRefs> = candidates
        .keys()
        .map(|surface| (surface.as_str(), ArticleRefs::find(&wiki, surface)))
        .collect();

    // 曖昧さ回避でなく、解けた記事のカテゴリだけを引く
    let wanted_pages: HashSet<u32> = refs
        .values()
        .filter(|r| !r.article_ids().any(|id| wiki.disambiguation.contains(&id)))
        .flat_map(|r| r.article_ids())
        .collect();
    let mut targets_of: HashMap<u32, Vec<u32>> = HashMap::new();
    for_each_tuple_line(&config.categorylinks_sql, |line| {
        if let Some(row) = parse_category_link(line)
            && wanted_pages.contains(&row.from)
        {
            targets_of.entry(row.from).or_default().push(row.target_id);
        }
    })?;
    let wanted_targets: HashSet<u32> = targets_of.values().flatten().copied().collect();
    let mut category_title: HashMap<u32, String> = HashMap::new();
    for_each_tuple_line(&config.linktarget_sql, |line| {
        if let Some(row) = parse_link_target(line)
            && row.namespace == NS_CATEGORY
            && wanted_targets.contains(&row.id)
        {
            category_title.insert(row.id, row.title);
        }
    })?;

    let mut rows: Vec<ListRow> = Vec::new();
    for (surface, row) in &candidates {
        let article = &refs[surface.as_str()];
        let (division, why) = decide(row, article, &wiki, &lexicon, &rules, |page| {
            targets_of
                .get(&page)
                .map(|ids| {
                    ids.iter()
                        .filter_map(|id| category_title.get(id).map(String::as_str))
                        .collect()
                })
                .unwrap_or_default()
        });
        report.count(why);
        if let Division::Class(class) = division {
            rows.push(ListRow {
                surface: surface.clone(),
                tf: row.tf,
                df: row.df,
                class,
            });
        }
    }
    rows.sort_by(|a, b| a.surface.cmp(&b.surface));

    write_list(&config.out, &header, &rows)?;
    report.written = rows.len() as u64;
    for row in &rows {
        *report.by_class.entry(row.class.as_str()).or_insert(0) += 1;
    }
    if let Some(path) = &config.report {
        std::fs::write(path, report.to_markdown(&header)).map_err(|e| io_error(path, e))?;
    }
    Ok(report)
}

/// 行 2〜9。0・1 は済んでいる
fn decide<'a>(
    row: &CollectedRow,
    article: &ArticleRefs,
    wiki: &Wiki,
    lexicon: &Lexicon,
    rules: &RuleTable,
    categories_of: impl Fn(u32) -> Vec<&'a str>,
) -> (Division, &'static str) {
    if !article.has_article() {
        return if has_proper_marker(row, lexicon) {
            (Division::Uncertain, reason::NO_ARTICLE_MARKER)
        } else {
            (Division::Class(WebClass::Common), reason::COMMON_NO_ARTICLE)
        };
    }
    // リダイレクトの行がない、3 段で着かない: 迷う（ユーザー決定 2026-11-06）
    if article.resolutions().any(|r| r == Resolution::NoRow) {
        return (Division::Uncertain, reason::REDIRECT_NO_ROW);
    }
    if article.resolutions().any(|r| r == Resolution::Unresolved) {
        return (Division::Uncertain, reason::REDIRECT_UNRESOLVED);
    }
    let ids: Vec<u32> = article.article_ids().collect();
    if ids.iter().any(|id| wiki.disambiguation.contains(id)) {
        return (Division::Uncertain, reason::DISAMBIGUATION);
    }
    let categories: Vec<&str> = ids.iter().flat_map(|id| categories_of(*id)).collect();
    classify_by_categories(&categories, rules)
}

fn write_list(path: &Path, header: &ListHeader, rows: &[ListRow]) -> Result<(), WebwordsError> {
    let file = File::create(path).map_err(|e| io_error(path, e))?;
    let mut out = BufWriter::new(file);
    writeln!(out, "{}", header.format()).map_err(|e| io_error(path, e))?;
    for row in rows {
        writeln!(out, "{}", row.format()).map_err(|e| io_error(path, e))?;
    }
    out.flush().map_err(|e| io_error(path, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 合成のダンプ 1 式
    struct Fixture {
        dir: tempfile::TempDir,
        collected: String,
        page: String,
        redirect: String,
        props: String,
        catlinks: String,
        linktarget: String,
        lex: String,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                dir: tempfile::tempdir().unwrap(),
                collected: String::new(),
                page: String::new(),
                redirect: String::new(),
                props: String::new(),
                catlinks: String::new(),
                linktarget: String::new(),
                lex: String::new(),
            }
        }

        fn word(&mut self, surface: &str, tf: u64, df: u64, cue: u64) {
            self.collected
                .push_str(&format!("{surface}\t{tf}\t{df}\t{cue}\n"));
        }

        fn page(&mut self, id: u32, ns: i32, title: &str, redirect: bool) {
            self.page.push_str(&format!(
                "({id},{ns},'{}',{},0,0.1,'20260101000000',NULL,1,2,'wikitext',NULL),\n",
                title.replace('\'', "\\'"),
                u8::from(redirect)
            ));
        }

        fn redirect(&mut self, from: u32, title: &str) {
            self.redirect
                .push_str(&format!("({from},0,'{title}','',''),\n"));
        }

        fn category(&mut self, page: u32, target: u32, name: &str) {
            self.catlinks.push_str(&format!(
                "({page},'SORT','2026-01-01 00:00:00','','page',0,{target}),\n"
            ));
            self.linktarget
                .push_str(&format!("({target},14,'{name}'),\n"));
        }

        fn run(&self) -> (ClassifyReport, Vec<String>) {
            let d = self.dir.path();
            let write = |name: &str, text: &str| {
                let path = d.join(name);
                std::fs::write(&path, text).unwrap();
                path
            };
            let config = ClassifyConfig {
                collected: write("collected.tsv", &format!("#h\n{}", self.collected)),
                page_sql: write("page.sql", &self.page),
                redirect_sql: write("redirect.sql", &self.redirect),
                page_props_sql: write("props.sql", &self.props),
                categorylinks_sql: write("cl.sql", &self.catlinks),
                linktarget_sql: write("lt.sql", &self.linktarget),
                categories: None,
                lex: vec![write("lex.csv", &self.lex)],
                header: ListHeader {
                    source: "jawiki-test".into(),
                    sha256: "0".repeat(64),
                    articles: 1,
                    sudachi: "test".into(),
                    rules: String::new(),
                },
                min_df: 5,
                out: d.join("out.tsv"),
                report: Some(d.join("report.md")),
            };
            let report = classify_with_report(&config).unwrap();
            let text = std::fs::read_to_string(&config.out).unwrap();
            (report, text.lines().map(str::to_owned).collect())
        }
    }

    fn lex_row(surface: &str, pos2: &str) -> String {
        format!(
            "{surface},1,1,100,{surface},名詞,{pos2},一般,*,*,*,{surface},{surface},*,A,*,*,*,*\n"
        )
    }

    fn classes(lines: &[String]) -> BTreeMap<String, String> {
        lines[1..]
            .iter()
            .map(|l| {
                let f: Vec<&str> = l.split('\t').collect();
                (f[0].to_owned(), f[3].to_owned())
            })
            .collect()
    }

    fn count(report: &ClassifyReport, key: &str) -> u64 {
        report.reasons.get(key).copied().unwrap_or(0)
    }

    #[test]
    fn rows_zero_to_two_known_shape_below_df_and_no_article() {
        let mut f = Fixture::new();
        f.lex.push_str(&lex_row("サーバー", "普通名詞"));
        f.word("サーバー", 100, 50, 0); // 既知
        f.word("アイ", 100, 50, 0); // 短い
        f.word("ーバージョン", 100, 50, 0); // ー で始まる
        f.word("ドドドド", 100, 50, 0); // 反復
        f.word("ヴァヷー", 100, 50, 0); // ヷ
        f.word("レア語", 100, 3, 0); // df が下限未満
        f.word("バージョニング", 100, 50, 1); // 記事なし
        f.word("ソロアルバム", 100, 50, 30); // 記事なし、手がかり 30%
        f.page(1, 0, "別の記事", false);
        let (report, lines) = f.run();
        assert!(lines[0].starts_with("#ikari-webwords\tschema=2\t"));
        assert_eq!(
            classes(&lines),
            BTreeMap::from([("バージョニング".to_owned(), "common".to_owned())])
        );
        assert_eq!(count(&report, reason::KNOWN), 1);
        assert_eq!(count(&report, reason::SHAPE_SHORT), 1);
        assert_eq!(count(&report, reason::SHAPE_LEAD), 1);
        assert_eq!(count(&report, reason::SHAPE_REDUP), 1);
        assert_eq!(count(&report, reason::SHAPE_BAD_CHAR), 1);
        assert_eq!(count(&report, reason::BELOW_MIN_DF), 1);
        assert_eq!(count(&report, reason::COMMON_NO_ARTICLE), 1);
        assert_eq!(count(&report, reason::NO_ARTICLE_MARKER), 1);
        assert_eq!(report.written, 1);
        let md = std::fs::read_to_string(f.dir.path().join("report.md")).unwrap();
        assert!(md.contains("| 0 既知（Sudachi・ABR に同じ表記） | 1 |"));
        // rules は規則表の sha256 が入る
        let header = ListHeader::parse(&lines[0]).unwrap();
        assert_eq!(header.rules.len(), 64);
        assert_eq!(header.rules, sha256_hex(DEFAULT_CATEGORIES_TEXT.as_bytes()));
    }

    #[test]
    fn proper_only_parts_are_a_marker_for_words_without_article() {
        let mut f = Fixture::new();
        f.lex.push_str(&lex_row("ジョン", "固有名詞"));
        f.lex.push_str(&lex_row("スミス", "固有名詞"));
        f.lex.push_str(&lex_row("ベース", "普通名詞"));
        f.lex.push_str(&lex_row("ベース", "固有名詞")); // 普通名詞の行もある → 固有名詞だけではない
        f.word("ジョンスミス", 10, 10, 0); // 固有名詞 + 固有名詞
        f.word("ジョンベース", 10, 10, 0); // ベース は固有名詞だけではない
        let (report, lines) = f.run();
        assert_eq!(
            classes(&lines),
            BTreeMap::from([("ジョンベース".to_owned(), "common".to_owned())])
        );
        assert_eq!(count(&report, reason::NO_ARTICLE_MARKER), 1);
    }

    #[test]
    fn redirect_hops_one_three_four_and_missing_row() {
        let mut f = Fixture::new();
        // 目的の記事 100（カテゴリは普通名詞）
        f.page(100, 0, "本体", false);
        f.category(100, 900, "ソフトウェアの技術");
        // 1 段: 101 -> 100
        f.page(101, 0, "ワン段", true);
        f.redirect(101, "本体");
        // 3 段: 103 -> 102 -> 101 -> 100
        f.page(102, 0, "ツー段", true);
        f.redirect(102, "ワン段");
        f.page(103, 0, "スリー段", true);
        f.redirect(103, "ツー段");
        // 4 段: 104 -> 103 -> 102 -> 101 -> 100
        f.page(104, 0, "フォー段", true);
        f.redirect(104, "スリー段");
        // 印があって行がない
        f.page(105, 0, "ノーロウ", true);
        // 行き先がない記事へ
        f.page(106, 0, "ミッシング", true);
        f.redirect(106, "存在しない");
        // 自分自身へ（ループ）
        f.page(107, 0, "ループ", true);
        f.redirect(107, "ループ");
        for w in [
            "本体",
            "ワン段",
            "スリー段",
            "フォー段",
            "ノーロウ",
            "ミッシング",
            "ループ",
        ] {
            // 「本体」は 2 字なので長さを足す
            let surface = if w == "本体" { "ホンタイ" } else { w };
            f.word(surface, 10, 10, 0);
        }
        // 同名の「ホンタイ」を本体の記事と同じ題にする
        f.page(108, 0, "ホンタイ", true);
        f.redirect(108, "本体");
        let (report, lines) = f.run();
        let got = classes(&lines);
        assert_eq!(got.get("ホンタイ").map(String::as_str), Some("common"));
        assert_eq!(got.get("ワン段").map(String::as_str), Some("common"));
        assert_eq!(got.get("スリー段").map(String::as_str), Some("common"));
        assert!(!got.contains_key("フォー段"), "4 段は迷う");
        assert!(!got.contains_key("ノーロウ"), "行がないは迷う");
        assert!(!got.contains_key("ミッシング"));
        assert!(!got.contains_key("ループ"));
        assert_eq!(count(&report, reason::COMMON_CATEGORY), 3);
        assert_eq!(count(&report, reason::REDIRECT_UNRESOLVED), 2);
        assert_eq!(count(&report, reason::REDIRECT_NO_ROW), 2);
    }

    #[test]
    fn disambiguation_pages_are_uncertain_even_through_redirect() {
        let mut f = Fixture::new();
        f.page(1, 0, "ジェネリクス", false);
        f.props.push_str("(1,'disambiguation',NULL,NULL),\n");
        f.category(1, 900, "ソフトウェアの技術");
        f.page(2, 0, "エイリアス", true);
        f.redirect(2, "ジェネリクス");
        f.word("ジェネリクス", 10, 10, 0);
        f.word("エイリアス", 10, 10, 0);
        let (report, lines) = f.run();
        assert!(classes(&lines).is_empty());
        assert_eq!(count(&report, reason::DISAMBIGUATION), 2);
    }

    #[test]
    fn categories_decide_the_class_and_conflicts_are_uncertain() {
        let mut f = Fixture::new();
        let cases = [
            ("ジョンスミス", "存命人物", None, Some("proper.person")),
            ("ミュンヘン", "ドイツの都市", None, Some("proper.place")),
            ("ソニー", "日本の企業", None, Some("proper.general")),
            (
                "リファクタリング",
                "ソフトウェアの技術",
                None,
                Some("common"),
            ),
            ("ジャガー", "日本の企業", Some("ソフトウェアの技術"), None),
            ("ルーク", "存命人物", Some("ドイツの都市"), None),
            ("ライオン", "ネコ科", None, Some("proper.general")),
            ("オペレ", "外科", None, None),
            ("ホゲラ", "ほげ", None, None),
        ];
        let mut next_target = 900;
        for (index, (surface, first, second, _)) in cases.iter().enumerate() {
            let id = 10 + index as u32;
            f.page(id, 0, surface, false);
            f.category(id, next_target, first);
            next_target += 1;
            if let Some(second) = second {
                f.category(id, next_target, second);
                next_target += 1;
            }
            f.word(surface, 10, 10, 0);
        }
        // 保守用のカテゴリは見ない（スタブは「…の企業のスタブ」でも外す）
        f.page(50, 0, "メンテナンス", false);
        f.category(50, 990, "日本の企業のスタブ");
        f.category(50, 991, "ソフトウェアの技術");
        f.word("メンテナンス", 10, 10, 0);
        let (report, lines) = f.run();
        let got = classes(&lines);
        for (surface, _, _, want) in cases {
            assert_eq!(got.get(surface).map(String::as_str), want, "{surface}");
        }
        assert_eq!(got.get("メンテナンス").map(String::as_str), Some("common"));
        assert_eq!(count(&report, reason::MULTI_CLASS), 2);
        assert_eq!(count(&report, reason::NO_CATEGORY_MATCH), 2);
        assert_eq!(count(&report, reason::PERSON), 1);
        assert_eq!(count(&report, reason::PLACE), 1);
        assert_eq!(count(&report, reason::GENERAL), 2);
        assert_eq!(count(&report, reason::COMMON_CATEGORY), 2);
        assert_eq!(report.by_class["proper.general"], 2);
    }

    #[test]
    fn paren_title_counts_as_an_article_and_conflicts_with_a_plain_one() {
        let mut f = Fixture::new();
        // 「ブリッジ」自身の記事はなく、「ブリッジ（技術）」だけがある → 記事あり扱い（同じ語の別の意味）
        f.page(1, 0, "ブリッジ（技術）", false);
        f.category(1, 900, "ソフトウェアの技術");
        f.word("ブリッジ", 10, 10, 0);
        // 自身は普通名詞、括弧つきは固有名詞 → 両方に当たる → 迷う
        f.page(2, 0, "クイーン", false);
        f.category(2, 901, "ソフトウェアの技術");
        f.page(3, 0, "クイーン（バンド）", false);
        f.category(3, 902, "イギリスの企業");
        f.word("クイーン", 10, 10, 0);
        let (report, lines) = f.run();
        assert_eq!(
            classes(&lines),
            BTreeMap::from([("ブリッジ".to_owned(), "common".to_owned())])
        );
        assert_eq!(count(&report, reason::MULTI_CLASS), 1);
    }

    #[test]
    fn escapes_in_titles_and_categories_are_resolved() {
        let mut f = Fixture::new();
        f.page(1, 0, "ラ'イト", false); // 題にアポストロフィ（\' でエスケープされる）
        f.category(1, 900, "ソフトウェアの技術");
        f.word("ラ'イト", 10, 10, 0);
        // is_katakana でなくても候補になるので、収集の側がカタカナだけを出す前提とは別に、題の照合を確かめる
        let (_, lines) = f.run();
        assert_eq!(
            classes(&lines).get("ラ'イト").map(String::as_str),
            Some("common")
        );
    }

    #[test]
    fn output_is_sorted_by_code_point_and_header_is_valid() {
        let mut f = Fixture::new();
        for w in ["ワークフロー", "アーキテクチャ", "コンテナ"] {
            f.word(w, 10, 10, 0);
        }
        let (_, lines) = f.run();
        let surfaces: Vec<&str> = lines[1..]
            .iter()
            .map(|l| l.split('\t').next().unwrap())
            .collect();
        assert_eq!(surfaces, ["アーキテクチャ", "コンテナ", "ワークフロー"]);
        assert!(ListHeader::parse(&lines[0]).is_ok());
    }

    #[test]
    fn segmentable_needs_two_or_more_parts() {
        let parts: HashSet<String> = ["ジョン", "スミス"].iter().map(|s| s.to_string()).collect();
        assert!(segmentable("ジョンスミス", &parts, 2));
        assert!(!segmentable("ジョン", &parts, 2));
        assert!(!segmentable("ジョンブル", &parts, 2));
    }

    #[test]
    fn strip_paren_suffix_handles_plain_and_nested() {
        assert_eq!(strip_paren_suffix("A（B）"), Some("A"));
        assert_eq!(strip_paren_suffix("A"), None);
        assert_eq!(strip_paren_suffix("A（B）C"), None);
    }
}
