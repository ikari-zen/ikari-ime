//! カテゴリの規則表 `dict-builder/webwords/categories.tsv` の読み込みと照合の型（ADR 0055 §2.3）。
//!
//! 列は `型の種類（完全一致 / 末尾 / 先頭 / 正規表現）<TAB>型<TAB>区分`。区分は `人名`、`地名`、`固有名詞一般`、`普通名詞`。
//! `#` で始まる行と空行は読み飛ばす。`正規表現` は依存（regex）を足すかをレーン B が決めるまで、読み込みの誤りにする。
//!
//! 保守用のカテゴリ（スタブ、Wikidata、ページ、記事、マジックリンクなど）は、照合の前に外す
//! （[`is_maintenance_category`]。どの規則にも当てない）。

use super::{WebClass, WebwordsError};

/// 同梱の規則表（`--categories` を省略したときに使う）
pub const DEFAULT_CATEGORIES_TEXT: &str = include_str!("../../webwords/categories.tsv");

/// カテゴリの名前にこれらを含むものは保守用で、照合から外す（記事の内容の分類ではない）
pub const MAINTENANCE_CATEGORY_MARKERS: [&str; 5] =
    ["スタブ", "Wikidata", "ページ", "記事", "マジックリンク"];

pub fn is_maintenance_category(name: &str) -> bool {
    MAINTENANCE_CATEGORY_MARKERS
        .iter()
        .any(|marker| name.contains(marker))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleKind {
    /// 完全一致
    Exact,
    /// 末尾
    Suffix,
    /// 先頭
    Prefix,
}

/// 1 行の規則。区分の `WebClass::Common` が表の「普通名詞」
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub kind: RuleKind,
    pub pattern: String,
    pub class: WebClass,
}

impl Rule {
    pub fn matches(&self, category: &str) -> bool {
        match self.kind {
            RuleKind::Exact => category == self.pattern,
            RuleKind::Suffix => category.ends_with(&self.pattern),
            RuleKind::Prefix => category.starts_with(&self.pattern),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuleTable {
    pub rules: Vec<Rule>,
}

impl RuleTable {
    pub fn parse(text: &str) -> Result<Self, WebwordsError> {
        let mut rules = Vec::new();
        for (index, line) in text.lines().enumerate() {
            if line.trim().is_empty() || line.starts_with('#') {
                continue;
            }
            let at = |message: String| {
                WebwordsError::Format(format!("規則表 {} 行目: {message}", index + 1))
            };
            let fields: Vec<&str> = line.split('\t').collect();
            let [kind, pattern, class] = fields[..] else {
                return Err(at("3 列ではありません".into()));
            };
            if pattern.is_empty() {
                return Err(at("型が空です".into()));
            }
            let kind = match kind {
                "完全一致" => RuleKind::Exact,
                "末尾" => RuleKind::Suffix,
                "先頭" => RuleKind::Prefix,
                "正規表現" => return Err(at("正規表現は未対応です".into())),
                other => return Err(at(format!("型の種類が不正です: {other}"))),
            };
            let class = match class {
                "人名" => WebClass::ProperPerson,
                "地名" => WebClass::ProperPlace,
                "固有名詞一般" => WebClass::ProperGeneral,
                "普通名詞" => WebClass::Common,
                other => return Err(at(format!("区分が不正です: {other}"))),
            };
            rules.push(Rule {
                kind,
                pattern: pattern.to_owned(),
                class,
            });
        }
        Ok(Self { rules })
    }

    /// 1 つのカテゴリに当たる区分（重複なし）。保守用のカテゴリは空
    pub fn classes_of(&self, category: &str) -> Vec<WebClass> {
        if is_maintenance_category(category) {
            return Vec::new();
        }
        let mut classes: Vec<WebClass> = self
            .rules
            .iter()
            .filter(|rule| rule.matches(category))
            .map(|rule| rule.class)
            .collect();
        classes.sort();
        classes.dedup();
        classes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> RuleTable {
        RuleTable::parse(DEFAULT_CATEGORIES_TEXT).unwrap()
    }

    #[test]
    fn bundled_table_is_not_empty_and_parses() {
        assert!(table().rules.len() >= 20);
    }

    #[test]
    fn five_added_types_are_in_the_table() {
        let t = table();
        assert_eq!(t.classes_of("日本の競走馬"), [WebClass::ProperGeneral]);
        assert_eq!(
            t.classes_of("アメリカ合衆国のサラブレッド"),
            [WebClass::ProperGeneral]
        );
        assert_eq!(t.classes_of("フランスの行政区画"), [WebClass::ProperPlace]);
        assert_eq!(t.classes_of("イタリアのコムーネ"), [WebClass::ProperPlace]);
        assert_eq!(t.classes_of("生没年不詳"), [WebClass::ProperPerson]);
    }

    #[test]
    fn maintenance_categories_are_never_matched() {
        let t = table();
        for name in [
            "スタブ",
            "人物のスタブ",
            "Wikidataにある人物の項目",
            "出典を必要とするページ",
            "存命人物の記事",
            "ISBNマジックリンクを使用しているページ",
        ] {
            assert!(is_maintenance_category(name), "{name}");
            assert!(t.classes_of(name).is_empty(), "{name}");
        }
        // 規則の型そのものが保守用の印を含まない（表の誤りを検出する）
        for rule in &t.rules {
            assert!(
                !is_maintenance_category(&rule.pattern),
                "規則 {} が保守用のカテゴリの印を含む",
                rule.pattern
            );
        }
    }

    #[test]
    fn rejects_bad_rows() {
        assert!(RuleTable::parse("末尾\tX").is_err());
        assert!(RuleTable::parse("中間\tX\t人名").is_err());
        assert!(RuleTable::parse("末尾\tX\t謎").is_err());
        assert!(RuleTable::parse("# c\n\n末尾\tX\t人名\n").is_ok());
    }
}
