//! ユーザー辞書（Mozc 互換の 4 列 TSV。ADR 0002 §10）。
//!
//! ```text
//! #ikari-user-dict<TAB>schema=1
//! かきすすめる<TAB>書き進める<TAB>動詞:下一段-マ行<TAB>
//! いかり<TAB>ikari-ime<TAB>短縮よみ<TAB>自作IME
//! ```
//! 列は 読み、表記、品詞（簡略名）、コメント。簡略名と内部の品詞名の対応は
//! `core/assets/user_pos.tsv`、Mozc の品詞名との対応は `core/assets/user_pos_mozc.tsv`。

mod mozc;
mod pack;
mod pos;
#[cfg(test)]
pub(crate) mod tests;

pub use mozc::{MozcParse, SkippedLine};
pub use pack::{
    IT_PACK_ID, PACK_LATIN_OFFSET, PACK_SCHEMA, PackEntry, PackForm, VocabularyPack,
    builtin_it_pack,
};
use pos::{Conj, check_ending, resolve_pos, validate_entry};
pub use pos::{PosCheckReport, PosOption, check_pos_table, pos_options, verb_conj_types};

use std::path::Path;
use std::sync::OnceLock;

use ikari_dict_format::reading::{is_indexable, normalize_key};

use super::UserDataError;
use super::util::{
    FIELD_SEPARATOR, check_schema, has_cell_breaker, parse_error, parse_header, read_optional,
    write_atomic,
};
use crate::dict::{ConjRule, Grammar, InMemoryDictionary, PosInfo};

pub const USER_DICT_SCHEMA: u32 = 1;
pub const USER_DICT_SECTION_NAME: &str = "user_dict";
/// 表記の最大文字数
pub const MAX_SURFACE_CHARS: usize = 100;
/// 代表左右IDで足りているとみなす割合の下限（‰。ADR 0002 §10.4）
pub const MIN_REP_SHARE_PERMILLE: u16 = 950;

const HEADER_NAME: &str = "#ikari-user-dict";
const COMMENT_PREFIXES: [char; 2] = ['#', '!'];
const VERB_POS: &str = "動詞";
const VERB_TYPE_SEPARATOR: char = ':';
const TYPE_PLACEHOLDER: &str = "{type}";
const MOZC_ICHIDAN: &str = "動詞:@一段";
const CONJ_NONE: &str = "none";
const CONJ_REQUIRED: &str = "required";
const CONJ_AUTO: &str = "auto";
const MIN_ICHIDAN_READING_CHARS: usize = 2;
/// 活用の確認で、終止形から導けているべき活用形（`<品詞1>,一般,*,*,<活用型>,<活用形>` の活用形）
const REQUIRED_CONJ_FORMS: [&str; 2] = ["連用形-一般", "未然形-一般"];

const POS_TABLE: &str = include_str!("../../../assets/user_pos.tsv");
const MOZC_TABLE: &str = include_str!("../../../assets/user_pos_mozc.tsv");

/// 読み、表記、品詞（簡略名。動詞は `動詞:<活用型>`）、コメント
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserDictEntry {
    pub reading: String,
    pub surface: String,
    pub pos: String,
    pub comment: String,
}

#[derive(Clone, Debug, Default)]
pub struct UserDictionary {
    entries: Vec<UserDictEntry>,
}

impl UserDictionary {
    /// ファイルがなければ空。
    pub fn open(path: &Path) -> Result<Self, UserDataError> {
        match read_optional(path)? {
            Some(text) => Self::parse_in(&text, path),
            None => Ok(Self::default()),
        }
    }

    /// ikari 形式の中身を検証して読む（エクスポートのインポートの事前検証にも使う）。
    pub fn parse(text: &str) -> Result<Self, UserDataError> {
        Self::parse_in(text, Path::new(USER_DICT_SECTION_NAME))
    }

    fn parse_in(text: &str, file: &Path) -> Result<Self, UserDataError> {
        let mut lines = text.lines();
        let header = lines
            .next()
            .ok_or_else(|| parse_error(file, 1, "空のファイルです"))?;
        let kv = parse_header(header, HEADER_NAME).ok_or_else(|| {
            parse_error(
                file,
                1,
                format!(
                    "ヘッダが `{HEADER_NAME}` ではありません（Mozc の辞書は `import --format mozc` で読み込みます）"
                ),
            )
        })?;
        check_schema(file, &kv, USER_DICT_SCHEMA)?;
        let mut dict = Self::default();
        for (i, line) in lines.enumerate() {
            let n = i + 2;
            if line.is_empty() {
                continue;
            }
            let f: Vec<&str> = line.split(FIELD_SEPARATOR).collect();
            let (reading, surface, pos, comment) = match f[..] {
                [r, s, p] => (r, s, p, ""),
                [r, s, p, c] => (r, s, p, c),
                _ => {
                    return Err(parse_error(
                        file,
                        n,
                        "列は 読み, 表記, 品詞, コメント の4列です",
                    ));
                }
            };
            let entry = validate_entry(&UserDictEntry {
                reading: reading.to_string(),
                surface: surface.to_string(),
                pos: pos.to_string(),
                comment: comment.to_string(),
            })
            .map_err(|e| parse_error(file, n, e.to_string()))?;
            if dict.contains(&entry.reading, &entry.surface) {
                return Err(parse_error(file, n, "同じ (読み, 表記) が重複しています"));
            }
            dict.entries.push(entry);
        }
        Ok(dict)
    }

    pub fn entries(&self) -> &[UserDictEntry] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn contains(&self, reading: &str, surface: &str) -> bool {
        self.entries
            .iter()
            .any(|e| e.reading == reading && e.surface == surface)
    }

    /// 読みは `normalize_key` を通し、`is_indexable` を満たさなければエラー。同じ (読み, 表記) は上書き。
    /// 品詞の名前と、動詞などの語尾が活用型と合うかも確かめる（grammar を引く検査は `build` / `check`）。
    pub fn add(&mut self, entry: UserDictEntry) -> Result<(), UserDataError> {
        let entry = validate_entry(&entry)?;
        match self
            .entries
            .iter_mut()
            .find(|e| e.reading == entry.reading && e.surface == entry.surface)
        {
            Some(existing) => *existing = entry,
            None => self.entries.push(entry),
        }
        Ok(())
    }

    /// 複数の項目をまとめて追加する。1 つでも不正なら何も追加しない。
    pub fn add_all(&mut self, entries: Vec<UserDictEntry>) -> Result<usize, UserDataError> {
        let mut next = self.clone();
        let n = entries.len();
        for e in entries {
            next.add(e)?;
        }
        *self = next;
        Ok(n)
    }

    pub fn remove(&mut self, reading: &str, surface: &str) -> bool {
        let reading = normalize_key(reading);
        let before = self.entries.len();
        self.entries
            .retain(|e| !(e.reading == reading && e.surface == surface));
        self.entries.len() != before
    }

    /// ファイルの中身（ヘッダ付き）。
    pub fn to_text(&self) -> String {
        let mut out = format!("{HEADER_NAME}\tschema={USER_DICT_SCHEMA}\n");
        for e in &self.entries {
            out.push_str(&format!(
                "{}\t{}\t{}\t{}\n",
                e.reading, e.surface, e.pos, e.comment
            ));
        }
        out
    }

    pub fn save(&self, path: &Path) -> Result<(), UserDataError> {
        write_atomic(path, &self.to_text())
    }

    /// 簡略品詞を内部の品詞に対応付け、活用する品詞は `Grammar::conj_rules` で展開して辞書にする。
    /// 代表左右IDと `user_cost` は `Grammar::pos_info`、ペナルティは 0。
    pub fn build(&self, grammar: &dyn Grammar) -> Result<InMemoryDictionary, UserDataError> {
        let mut dict = self.build_with_costs(grammar, &|_, info| info.user_cost)?;
        dict.mark_user_authored();
        Ok(dict)
    }

    /// `build` と同じ作り方で、項目のコストを `cost_of(項目の添字, 品詞の代表ID と user_cost)` で決める
    /// （語彙集が英字を、カタカナの後ろに置くため）。
    pub(crate) fn build_with_costs(
        &self,
        grammar: &dyn Grammar,
        cost_of: &dyn Fn(usize, &PosInfo) -> i16,
    ) -> Result<InMemoryDictionary, UserDataError> {
        let mut dict = InMemoryDictionary::new(grammar.grammar_id());
        for (index, e) in self.entries.iter().enumerate() {
            let pos = resolve_pos(&e.pos)?;
            check_ending(&pos, &e.reading, &e.surface)?;
            let base_id = grammar.pos_id(&pos.name).ok_or_else(|| {
                UserDataError::Invalid(format!(
                    "文法に品詞がありません: {}（{}「{}」）",
                    pos.name, e.pos, e.surface
                ))
            })?;
            let cost = |info: &PosInfo| cost_of(index, info);
            insert_form(
                &mut dict, grammar, &e.reading, &e.surface, &e.surface, base_id, &cost,
            )?;
            if pos.conj == Conj::None {
                continue;
            }
            let rules: Vec<&ConjRule> = grammar
                .conj_rules()
                .iter()
                .filter(|r| r.base_pos == base_id)
                .collect();
            if rules.is_empty() {
                return Err(UserDataError::Invalid(format!(
                    "活用規則がありません: {}（「{}」）。辞書を v2 で作り直してください",
                    pos.name, e.surface
                )));
            }
            for r in rules {
                let surface = conjugate(&e.surface, r, &r.add_surface)?;
                let reading = conjugate(&e.reading, r, &r.add_reading)?;
                insert_form(
                    &mut dict,
                    grammar,
                    &reading,
                    &surface,
                    &e.surface,
                    r.target_pos,
                    &cost,
                )?;
            }
        }
        Ok(dict)
    }

    /// 品詞の対応表と、この辞書の項目を grammar に対して検査する。
    pub fn check(&self, grammar: &dyn Grammar) -> PosCheckReport {
        let mut extra: Vec<String> = Vec::new();
        for e in &self.entries {
            if let Ok(p) = resolve_pos(&e.pos)
                && let Some(t) = p.conj_type
                && !extra.contains(&t)
            {
                extra.push(t);
            }
        }
        let mut report = check_pos_table(grammar, &extra);
        if let Err(e) = self.build(grammar) {
            report
                .errors
                .push(format!("ユーザー辞書を展開できません: {e}"));
        }
        report
    }
}

fn conjugate(base: &str, rule: &ConjRule, add: &str) -> Result<String, UserDataError> {
    let chars = base.chars().count();
    let keep = chars
        .checked_sub(usize::from(rule.strip_chars))
        .ok_or_else(|| {
            UserDataError::Invalid(format!(
                "「{base}」は活用規則が削る{}文字より短いです",
                rule.strip_chars
            ))
        })?;
    let mut s: String = base.chars().take(keep).collect();
    s.push_str(add);
    Ok(s)
}

fn insert_form(
    dict: &mut InMemoryDictionary,
    grammar: &dyn Grammar,
    reading: &str,
    surface: &str,
    lemma: &str,
    pos_id: u16,
    cost: &dyn Fn(&PosInfo) -> i16,
) -> Result<(), UserDataError> {
    let info = grammar.pos_info(pos_id).ok_or_else(|| {
        UserDataError::Invalid(format!("文法に品詞の代表IDがありません (pos_id={pos_id})"))
    })?;
    // 活用形が索引の上限（32文字）を超えるものは入れない
    if !is_indexable(&normalize_key(reading)) {
        return Ok(());
    }
    let id = dict.insert(
        reading,
        surface,
        info.rep_left_id,
        info.rep_right_id,
        cost(&info),
        pos_id,
    );
    // 活用形の lemma は辞書形（システム辞書の正規化表記と同じ扱い。文書内キャッシュが活用形をまたいで効く）
    dict.set_lemma(id, lemma);
    Ok(())
}
