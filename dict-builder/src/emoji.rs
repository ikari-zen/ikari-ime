//! 絵文字の表 `emoji.tsv` を作る（ADR 0027 §5.3）。
//!
//! 入力は iamcal/emoji-data の `emoji.json`。`short_names`（`short_name` を含む別名）を、名前の昇順の
//! `s<TAB>名前<TAB>絵文字` の行にする。肌色の変種（`skin_variations`）と部品（`Component`）は入れない
//! （基本の黄色だけ。ADR 0027 Q11）。同じ入力からは同じバイト列ができる。

use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// `emoji.tsv` の先頭行（版）
pub const EMOJI_TABLE_HEADER: &str = "# ikari-ime emoji table v1";
/// ショートコードの行の種類
pub const KIND_SHORTCODE: &str = "s";
/// source 行に書くデータの名前
pub const SOURCE_EMOJI_DATA: &str = "emoji-data";
const COLUMNS_COMMENT: &str = "# 列: 種類\tキー\t絵文字";
/// 基本の絵文字に入れない部品の分類（肌色の見本、髪の部品）
const CATEGORY_COMPONENT: &str = "Component";
const CODEPOINT_SEPARATOR: char = '-';
const CODEPOINT_RADIX: u32 = 16;
const TMP_SUFFIX: &str = ".tmp";

#[derive(Debug)]
pub enum EmojiError {
    Io(String),
    Json(String),
    /// 入力の形が想定と違う
    Schema(String),
}

impl fmt::Display for EmojiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(m) => write!(f, "io: {m}"),
            Self::Json(m) => write!(f, "json: {m}"),
            Self::Schema(m) => write!(f, "emoji.json: {m}"),
        }
    }
}

impl std::error::Error for EmojiError {}

/// 作った表の件数
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmojiReport {
    /// 行の数（名前の数）
    pub names: usize,
    /// 絵文字の数（重複を除く）
    pub emojis: usize,
}

impl fmt::Display for EmojiReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "emoji: names={} emojis={}", self.names, self.emojis)
    }
}

/// `unified`（"1F468-200D-1F4BB"）を文字列にする
fn unified_to_string(unified: &str) -> Result<String, EmojiError> {
    unified
        .split(CODEPOINT_SEPARATOR)
        .map(|hex| {
            u32::from_str_radix(hex, CODEPOINT_RADIX)
                .ok()
                .and_then(char::from_u32)
                .ok_or_else(|| EmojiError::Schema(format!("bad code point in {unified:?}")))
        })
        .collect()
}

fn str_field<'a>(entry: &'a Value, key: &str) -> Result<&'a str, EmojiError> {
    entry
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| EmojiError::Schema(format!("missing string field {key:?}")))
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && !name.chars().any(|c| c.is_control() || c.is_whitespace())
}

/// `emoji.json` のテキストと取得元のコミットから、`emoji.tsv` のテキストを作る。
pub fn build_emoji_table(json: &str, commit: &str) -> Result<(String, EmojiReport), EmojiError> {
    if commit.is_empty() || commit.chars().any(char::is_whitespace) {
        return Err(EmojiError::Schema(format!("bad commit {commit:?}")));
    }
    let root: Value = serde_json::from_str(json).map_err(|e| EmojiError::Json(e.to_string()))?;
    let entries = root
        .as_array()
        .ok_or_else(|| EmojiError::Schema("top level is not an array".into()))?;

    // (名前, 絵文字) の順に並ぶ。重複する行は 1 つにする
    let mut rows: BTreeSet<(String, String)> = BTreeSet::new();
    for entry in entries {
        if entry.get("category").and_then(Value::as_str) == Some(CATEGORY_COMPONENT) {
            continue;
        }
        let emoji = unified_to_string(str_field(entry, "unified")?)?;
        let names = entry
            .get("short_names")
            .and_then(Value::as_array)
            .ok_or_else(|| EmojiError::Schema("missing short_names".into()))?;
        for name in names {
            let name = name
                .as_str()
                .ok_or_else(|| EmojiError::Schema("short_names has a non-string".into()))?;
            if !valid_name(name) {
                return Err(EmojiError::Schema(format!("bad short name {name:?}")));
            }
            rows.insert((name.to_owned(), emoji.clone()));
        }
    }

    // 1 つの名前が別々の絵文字を指すと、前方一致の確定が曖昧になる
    let mut prev: Option<&str> = None;
    for (name, _) in &rows {
        if prev == Some(name.as_str()) {
            return Err(EmojiError::Schema(format!(
                "short name {name:?} maps to more than one emoji"
            )));
        }
        prev = Some(name);
    }

    let mut out = String::new();
    out.push_str(EMOJI_TABLE_HEADER);
    out.push('\n');
    out.push_str(&format!("# source\t{SOURCE_EMOJI_DATA}\t{commit}\n"));
    out.push_str(COLUMNS_COMMENT);
    out.push('\n');
    for (name, emoji) in &rows {
        out.push_str(&format!("{KIND_SHORTCODE}\t{name}\t{emoji}\n"));
    }
    let emojis = rows
        .iter()
        .map(|(_, e)| e.as_str())
        .collect::<BTreeSet<_>>()
        .len();
    Ok((
        out,
        EmojiReport {
            names: rows.len(),
            emojis,
        },
    ))
}

/// `emoji.json` を読んで `out` に `emoji.tsv` を書く（一時ファイルに書いて rename）。
pub fn build_emoji_file(
    json_path: &Path,
    commit: &str,
    out: &Path,
) -> Result<EmojiReport, EmojiError> {
    let json = fs::read_to_string(json_path)
        .map_err(|e| EmojiError::Io(format!("cannot read {}: {e}", json_path.display())))?;
    let (text, report) = build_emoji_table(&json, commit)?;
    let mut tmp = out.as_os_str().to_owned();
    tmp.push(TMP_SUFFIX);
    let tmp = PathBuf::from(tmp);
    fs::write(&tmp, text)
        .and_then(|()| fs::rename(&tmp, out))
        .map_err(|e| EmojiError::Io(format!("cannot write {}: {e}", out.display())))?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"[
      {"unified":"1F440","short_name":"eyes","short_names":["eyes"],"category":"People & Body",
       "skin_variations":{"1F3FB":{"unified":"1F440-1F3FB"}}},
      {"unified":"1F1EF-1F1F5","short_name":"flag-jp","short_names":["flag-jp","jp"],"category":"Flags"},
      {"unified":"1F44D","short_name":"+1","short_names":["+1","thumbsup"],"category":"People & Body"},
      {"unified":"1F3FB","short_name":"skin-tone-2","short_names":["skin-tone-2"],"category":"Component"},
      {"unified":"2764-FE0F","short_name":"heart","short_names":["heart"],"category":"Smileys & Emotion"}
    ]"#;

    #[test]
    fn builds_sorted_table_with_header() {
        let (text, report) = build_emoji_table(SAMPLE, "abc123").unwrap();
        let want = "# ikari-ime emoji table v1\n# source\temoji-data\tabc123\n# 列: 種類\tキー\t絵文字\n\
s\t+1\t👍\ns\teyes\t👀\ns\tflag-jp\t🇯🇵\ns\theart\t❤\u{fe0f}\ns\tjp\t🇯🇵\ns\tthumbsup\t👍\n";
        assert_eq!(text, want);
        assert_eq!(
            report,
            EmojiReport {
                names: 6,
                emojis: 4
            }
        );
    }

    #[test]
    fn is_deterministic_and_skips_components_and_skin_variations() {
        let (a, _) = build_emoji_table(SAMPLE, "c").unwrap();
        let (b, _) = build_emoji_table(SAMPLE, "c").unwrap();
        assert_eq!(a, b);
        assert!(!a.contains("skin-tone"));
        assert!(!a.contains('\u{1F3FB}'));
    }

    #[test]
    fn rejects_name_mapped_to_two_emojis() {
        let json =
            r#"[{"unified":"1F440","short_names":["x"]},{"unified":"1F441","short_names":["x"]}]"#;
        assert!(matches!(
            build_emoji_table(json, "c"),
            Err(EmojiError::Schema(_))
        ));
    }

    #[test]
    fn rejects_bad_input() {
        assert!(matches!(
            build_emoji_table("{}", "c"),
            Err(EmojiError::Schema(_))
        ));
        assert!(matches!(
            build_emoji_table("[", "c"),
            Err(EmojiError::Json(_))
        ));
        let bad_cp = r#"[{"unified":"ZZ","short_names":["x"]}]"#;
        assert!(matches!(
            build_emoji_table(bad_cp, "c"),
            Err(EmojiError::Schema(_))
        ));
        let no_names = r#"[{"unified":"1F440"}]"#;
        assert!(matches!(
            build_emoji_table(no_names, "c"),
            Err(EmojiError::Schema(_))
        ));
        assert!(build_emoji_table("[]", "").is_err());
    }
}
