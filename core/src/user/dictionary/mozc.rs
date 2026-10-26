use super::pos::mozc_map;
use super::*;

/// Mozc の取り込みで飛ばした行
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkippedLine {
    /// 1 始まり
    pub line: usize,
    pub reason: String,
}

#[derive(Clone, Debug, Default)]
pub struct MozcParse {
    pub entries: Vec<UserDictEntry>,
    pub skipped: Vec<SkippedLine>,
}

impl UserDictionary {
    /// Mozc のユーザー辞書（品詞名は `core/assets/user_pos_mozc.tsv` で対応付ける）を読む。
    /// 対応できない品詞や読みが 1 行でもあればエラー（飛ばして続けるときは [`Self::parse_mozc_report`]）。
    pub fn parse_mozc(tsv: &str) -> Result<Vec<UserDictEntry>, UserDataError> {
        let parsed = Self::parse_mozc_report(tsv)?;
        if let Some(s) = parsed.skipped.first() {
            return Err(parse_error(Path::new("mozc"), s.line, s.reason.clone()));
        }
        Ok(parsed.entries)
    }

    /// `parse_mozc` の、対応できない行を飛ばして報告する版。列が足りない行はエラー。
    pub fn parse_mozc_report(tsv: &str) -> Result<MozcParse, UserDataError> {
        let file = Path::new("mozc");
        let mut out = MozcParse::default();
        for (i, line) in tsv.lines().enumerate() {
            let n = i + 1;
            let line = line.trim_end_matches('\r');
            if line.trim().is_empty() || line.starts_with(COMMENT_PREFIXES) {
                continue;
            }
            let f: Vec<&str> = line.split(FIELD_SEPARATOR).collect();
            if f.len() < 3 {
                return Err(parse_error(file, n, "列は 読み, 表記, 品詞, コメント です"));
            }
            let (reading, surface, mozc_pos) = (f[0].trim(), f[1], f[2].trim());
            let comment = f.get(3).map_or("", |c| c.trim());
            let skip = |reason: String| SkippedLine { line: n, reason };
            let Some(simple) = mozc_map()
                .iter()
                .find(|(m, _)| m == mozc_pos)
                .map(|(_, s)| s.as_str())
            else {
                out.skipped
                    .push(skip(format!("対応しない品詞です: {mozc_pos}")));
                continue;
            };
            let simple = if simple == MOZC_ICHIDAN {
                match infer_ichidan(&normalize_key(reading)) {
                    Some(s) => s,
                    None => {
                        out.skipped.push(skip(format!(
                            "一段活用の型を読み「{reading}」から推定できません"
                        )));
                        continue;
                    }
                }
            } else {
                simple.to_string()
            };
            let entry = UserDictEntry {
                reading: reading.to_string(),
                surface: surface.to_string(),
                pos: simple,
                comment: comment.to_string(),
            };
            match validate_entry(&entry) {
                Ok(e) => out.entries.push(e),
                Err(e) => out.skipped.push(skip(e.to_string())),
            }
        }
        Ok(out)
    }
}

/// Mozc の「動詞一段」の型を、読みの末尾「る」の直前の文字の段と行から推定する。
pub(super) fn infer_ichidan(reading: &str) -> Option<String> {
    let chars: Vec<char> = reading.chars().collect();
    if chars.len() < MIN_ICHIDAN_READING_CHARS || *chars.last()? != 'る' {
        return None;
    }
    let (row, dan) = kana_row_and_dan(chars[chars.len() - 2])?;
    let kind = match dan {
        Dan::I => "上一段",
        Dan::E => "下一段",
        _ => return None,
    };
    Some(format!("{VERB_POS}{VERB_TYPE_SEPARATOR}{kind}-{row}行"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dan {
    A,
    I,
    U,
    E,
    O,
}

fn kana_row_and_dan(c: char) -> Option<(&'static str, Dan)> {
    const ROWS: [(&str, &str); 14] = [
        ("ア", "あいうえお"),
        ("カ", "かきくけこ"),
        ("ガ", "がぎぐげご"),
        ("サ", "さしすせそ"),
        ("ザ", "ざじずぜぞ"),
        ("タ", "たちつてと"),
        ("ダ", "だぢづでど"),
        ("ナ", "なにぬねの"),
        ("ハ", "はひふへほ"),
        ("バ", "ばびぶべぼ"),
        ("パ", "ぱぴぷぺぽ"),
        ("マ", "まみむめも"),
        ("ヤ", "や\0ゆ\0よ"),
        ("ラ", "らりるれろ"),
    ];
    const DANS: [Dan; 5] = [Dan::A, Dan::I, Dan::U, Dan::E, Dan::O];
    ROWS.iter()
        .find_map(|(row, kana)| kana.chars().position(|k| k == c).map(|i| (*row, DANS[i])))
}
