//! レーンが共有する型（ADR 0055 §2.2、§5.2）。

use super::WebwordsError;

/// 一覧の 1 行目の先頭の欄
pub const LIST_MAGIC: &str = "#ikari-webwords";
pub const LIST_SCHEMA: u32 = 2;
pub const LICENSE: &str = "CC-BY-SA-4.0";
pub const ATTRIBUTION: &str = "Wikipedia 日本語版の執筆者（https://ja.wikipedia.org/）";
/// 配る一覧に出す df の下限（掃引の最小。ADR 0055 §5.1 の 3）
pub const MIN_DF_FLOOR: u32 = 5;

/// 一覧に出る区分（ADR 0055 §2.2 の入れ方）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WebClass {
    /// `名詞,普通名詞,一般`
    Common,
    /// `名詞,固有名詞,人名,一般`
    ProperPerson,
    /// `名詞,固有名詞,地名,一般`
    ProperPlace,
    /// `名詞,固有名詞,一般`
    ProperGeneral,
}

impl WebClass {
    pub const ALL: [Self; 4] = [
        Self::Common,
        Self::ProperPerson,
        Self::ProperPlace,
        Self::ProperGeneral,
    ];

    /// 一覧の区分の欄の文字列
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Common => "common",
            Self::ProperPerson => "proper.person",
            Self::ProperPlace => "proper.place",
            Self::ProperGeneral => "proper.general",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.as_str() == text)
    }

    pub fn is_proper(self) -> bool {
        self != Self::Common
    }
}

/// 判定の結果（ADR 0055 §2.2 の行 0〜9）。一覧に出るのは `Class` だけで、ほかは理由ごとの件数をレポートに出す。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Division {
    /// 行 0: Sudachi・ABR に同じ表記がある（F1）
    Known,
    /// 行 1: 形の規則（F3）に当たる
    Shape,
    /// 行 2、5〜8
    Class(WebClass),
    /// 行 3、4、9: 迷う語（入れない）
    Uncertain,
}

/// 収集（`collect`）の出力の 1 行。ファイルは `表記<TAB>tf<TAB>df<TAB>cue`（先頭に `#` の行があれば無視する）。
/// cue は本文の連なりの直後に固有名詞の手がかりが付いた回数（ADR 0055 §2.4 の F4b）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectedRow {
    pub surface: String,
    pub tf: u64,
    pub df: u64,
    pub cue: u64,
}

/// 一覧の 1 行目（schema=2。形式は [`super`] の doc）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListHeader {
    /// `jawiki-<日付>`
    pub source: String,
    pub sha256: String,
    pub articles: u64,
    pub sudachi: String,
    /// categories.tsv の sha256
    pub rules: String,
}

/// 一覧の 2 行目以降の 1 行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListRow {
    pub surface: String,
    pub tf: u64,
    pub df: u64,
    pub class: WebClass,
}

fn format_error(message: impl Into<String>) -> WebwordsError {
    WebwordsError::Format(message.into())
}

fn number(text: &str) -> Result<u64, WebwordsError> {
    text.parse::<u64>()
        .map_err(|_| format_error(format!("数ではありません: {text}")))
}

impl ListHeader {
    pub fn format(&self) -> String {
        format!(
            "{LIST_MAGIC}\tschema={LIST_SCHEMA}\tsource={}\tsha256={}\tarticles={}\tsudachi={}\trules={}\tlicense={LICENSE}\tattribution={ATTRIBUTION}",
            self.source, self.sha256, self.articles, self.sudachi, self.rules
        )
    }

    pub fn parse(line: &str) -> Result<Self, WebwordsError> {
        let mut fields = line.split('\t');
        if fields.next() != Some(LIST_MAGIC) {
            return Err(format_error(format!(
                "1 行目が {LIST_MAGIC} で始まりません"
            )));
        }
        let mut value = |key: &str| -> Result<&str, WebwordsError> {
            fields
                .next()
                .and_then(|f| f.strip_prefix(key)?.strip_prefix('='))
                .ok_or_else(|| format_error(format!("1 行目に {key}= がありません（順序も固定）")))
        };
        let schema = value("schema")?;
        if schema != LIST_SCHEMA.to_string() {
            return Err(format_error(format!(
                "schema={schema} は読めません（schema={LIST_SCHEMA} だけ）"
            )));
        }
        let source = value("source")?.to_owned();
        let sha256 = value("sha256")?.to_owned();
        let articles = number(value("articles")?)?;
        let sudachi = value("sudachi")?.to_owned();
        let rules = value("rules")?.to_owned();
        if value("license")? != LICENSE {
            return Err(format_error(format!("license= は {LICENSE} だけです")));
        }
        value("attribution")?;
        Ok(Self {
            source,
            sha256,
            articles,
            sudachi,
            rules,
        })
    }
}

impl ListRow {
    pub fn format(&self) -> String {
        format!(
            "{}\t{}\t{}\t{}",
            self.surface,
            self.tf,
            self.df,
            self.class.as_str()
        )
    }

    pub fn parse(line: &str) -> Result<Self, WebwordsError> {
        let fields: Vec<&str> = line.split('\t').collect();
        let [surface, tf, df, class] = fields[..] else {
            return Err(format_error(format!("4 列ではありません: {line}")));
        };
        Ok(Self {
            surface: surface.to_owned(),
            tf: number(tf)?,
            df: number(df)?,
            class: WebClass::parse(class)
                .ok_or_else(|| format_error(format!("区分が不正です: {class}")))?,
        })
    }
}

impl CollectedRow {
    pub fn format(&self) -> String {
        format!("{}\t{}\t{}\t{}", self.surface, self.tf, self.df, self.cue)
    }

    pub fn parse(line: &str) -> Result<Self, WebwordsError> {
        let fields: Vec<&str> = line.split('\t').collect();
        let [surface, tf, df, cue] = fields[..] else {
            return Err(format_error(format!("4 列ではありません: {line}")));
        };
        Ok(Self {
            surface: surface.to_owned(),
            tf: number(tf)?,
            df: number(df)?,
            cue: number(cue)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header() -> ListHeader {
        ListHeader {
            source: "jawiki-20261001".into(),
            sha256: "ab".repeat(32),
            articles: 930_000,
            sudachi: "sudachi-20260723".into(),
            rules: "cd".repeat(32),
        }
    }

    #[test]
    fn header_round_trips() {
        let h = header();
        assert_eq!(ListHeader::parse(&h.format()).unwrap(), h);
        assert!(
            h.format()
                .starts_with("#ikari-webwords\tschema=2\tsource=jawiki-20261001\t")
        );
    }

    #[test]
    fn header_rejects_other_schema_and_license() {
        let line = header().format();
        assert!(ListHeader::parse(&line.replace("schema=2", "schema=1")).is_err());
        assert!(ListHeader::parse(&line.replace(LICENSE, "MIT")).is_err());
        assert!(ListHeader::parse("hello").is_err());
    }

    #[test]
    fn row_round_trips_and_rejects_bad_class() {
        let row = ListRow {
            surface: "バージョニング".into(),
            tf: 120,
            df: 30,
            class: WebClass::Common,
        };
        assert_eq!(row.format(), "バージョニング\t120\t30\tcommon");
        assert_eq!(ListRow::parse(&row.format()).unwrap(), row);
        assert!(ListRow::parse("ア\t1\t1\tunknown").is_err());
        assert!(ListRow::parse("ア\t1\t1").is_err());
    }

    #[test]
    fn class_names_are_the_four_in_the_adr() {
        let names: Vec<&str> = WebClass::ALL.iter().map(|c| c.as_str()).collect();
        assert_eq!(
            names,
            ["common", "proper.person", "proper.place", "proper.general"]
        );
        assert!(WebClass::parse("uncertain").is_none());
    }

    #[test]
    fn collected_row_round_trips() {
        let row = CollectedRow {
            surface: "ソロアルバム".into(),
            tf: 9,
            df: 5,
            cue: 1,
        };
        assert_eq!(CollectedRow::parse(&row.format()).unwrap(), row);
    }
}
