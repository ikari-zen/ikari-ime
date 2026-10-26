use super::*;

/// 活用の扱い
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Conj {
    None,
    /// 活用型の指定が必要（動詞）
    Required,
    /// 自動で展開（形容詞）
    Auto,
}

pub(super) struct PosDef {
    pub(super) simple: String,
    /// `{type}` を含みうる内部の品詞名
    pub(super) template: String,
    pub(super) conj: Conj,
}

/// 簡略品詞を解決した結果。
pub(super) struct ResolvedPos {
    /// 内部の品詞名（grammar の品詞表の名前）
    pub(super) name: String,
    pub(super) conj: Conj,
    /// 動詞の活用型（`下一段-マ行` など）
    pub(super) conj_type: Option<String>,
}

pub(super) fn pos_defs() -> &'static [PosDef] {
    static DEFS: OnceLock<Vec<PosDef>> = OnceLock::new();
    DEFS.get_or_init(|| {
        table_rows(POS_TABLE)
            .filter_map(|f| {
                let [simple, template, conj] = f[..] else {
                    return None;
                };
                let conj = match conj {
                    CONJ_NONE => Conj::None,
                    CONJ_REQUIRED => Conj::Required,
                    CONJ_AUTO => Conj::Auto,
                    _ => return None,
                };
                Some(PosDef {
                    simple: simple.to_string(),
                    template: template.to_string(),
                    conj,
                })
            })
            .collect()
    })
}

pub(super) fn mozc_map() -> &'static [(String, String)] {
    static MAP: OnceLock<Vec<(String, String)>> = OnceLock::new();
    MAP.get_or_init(|| {
        table_rows(MOZC_TABLE)
            .filter_map(|f| {
                let [mozc, simple] = f[..] else { return None };
                Some((mozc.to_string(), simple.to_string()))
            })
            .collect()
    })
}

/// 同梱の表の行（注釈と空行を除く）をタブで分けたもの。
fn table_rows(src: &'static str) -> impl Iterator<Item = Vec<&'static str>> {
    src.lines()
        .filter(|l| !l.is_empty() && !l.starts_with(COMMENT_PREFIXES[0]))
        .map(|l| l.split(FIELD_SEPARATOR).collect())
}

pub(super) fn resolve_pos(simple: &str) -> Result<ResolvedPos, UserDataError> {
    let (head, conj_type) = match simple.split_once(VERB_TYPE_SEPARATOR) {
        Some((h, t)) => (h, Some(t)),
        None => (simple, None),
    };
    let def = pos_defs()
        .iter()
        .find(|d| d.simple == head)
        .ok_or_else(|| UserDataError::Invalid(format!("未知の品詞です: {simple}")))?;
    match (def.conj, conj_type) {
        (Conj::Required, None) => Err(UserDataError::Invalid(format!(
            "{head} には活用型が必要です（例: {head}{VERB_TYPE_SEPARATOR}五段-ラ行）"
        ))),
        (Conj::Required, Some(t)) if t.is_empty() || t.contains([',', ' ', '*']) => Err(
            UserDataError::Invalid(format!("活用型が不正です: {simple}")),
        ),
        (Conj::Required, Some(t)) => Ok(ResolvedPos {
            name: def.template.replace(TYPE_PLACEHOLDER, t),
            conj: def.conj,
            conj_type: Some(t.to_string()),
        }),
        (_, Some(_)) => Err(UserDataError::Invalid(format!(
            "{head} に活用型は指定できません: {simple}"
        ))),
        (conj, None) => Ok(ResolvedPos {
            name: def.template.clone(),
            conj,
            conj_type: None,
        }),
    }
}

/// 活用型が要求する語尾（終止形の最後の文字、または最後の 2 文字）。確認できない型は None。
fn expected_endings(pos: &ResolvedPos) -> Option<Vec<&'static str>> {
    const GODAN: [(&str, &str); 9] = [
        ("カ", "く"),
        ("ガ", "ぐ"),
        ("サ", "す"),
        ("タ", "つ"),
        ("ナ", "ぬ"),
        ("バ", "ぶ"),
        ("マ", "む"),
        ("ラ", "る"),
        ("ワア", "う"),
    ];
    if pos.conj == Conj::Auto {
        return Some(vec!["い"]);
    }
    let t = pos.conj_type.as_deref()?;
    if let Some(row) = t.strip_prefix("五段-").and_then(|r| r.strip_suffix("行")) {
        return GODAN
            .iter()
            .find(|(r, _)| *r == row)
            .map(|(_, end)| vec![*end]);
    }
    if t.starts_with("上一段-") || t.starts_with("下一段-") {
        return Some(vec!["る"]);
    }
    match t {
        "サ行変格" => Some(vec!["する", "ずる"]),
        "カ行変格" => Some(vec!["くる"]),
        _ => None,
    }
}

/// 活用する語の語尾が活用型と合っているか（規則が削る文字列と一致するか。ADR 0002 §10.3-2）。
pub(super) fn check_ending(
    pos: &ResolvedPos,
    reading: &str,
    surface: &str,
) -> Result<(), UserDataError> {
    let Some(endings) = expected_endings(pos) else {
        return Ok(());
    };
    if !endings.iter().any(|e| reading.ends_with(e)) {
        return Err(UserDataError::Invalid(format!(
            "読み「{reading}」の語尾が品詞 {} と合いません（{}で終わる必要があります）",
            pos.name,
            endings.join("か")
        )));
    }
    if !endings.iter().any(|e| surface.ends_with(e)) {
        return Err(UserDataError::Invalid(format!(
            "表記「{surface}」の語尾が品詞 {} と合いません（{}で終わる必要があります）",
            pos.name,
            endings.join("か")
        )));
    }
    Ok(())
}

/// 追加前の検査。読みを正規化した形を返す。
pub(super) fn validate_entry(e: &UserDictEntry) -> Result<UserDictEntry, UserDataError> {
    let reading = normalize_key(&e.reading);
    if !is_indexable(&reading) {
        return Err(UserDataError::Invalid(format!(
            "読みはひらがな（とー）の1〜{}文字にしてください: {}",
            ikari_dict_format::MAX_READING_CHARS,
            e.reading
        )));
    }
    let surface_chars = e.surface.chars().count();
    if e.surface.is_empty() || surface_chars > MAX_SURFACE_CHARS {
        return Err(UserDataError::Invalid(format!(
            "表記は1〜{MAX_SURFACE_CHARS}文字にしてください"
        )));
    }
    if has_cell_breaker(&e.surface) || has_cell_breaker(&e.comment) || has_cell_breaker(&e.pos) {
        return Err(UserDataError::Invalid(
            "表記、品詞、コメントにタブと改行は使えません".to_string(),
        ));
    }
    let pos = resolve_pos(&e.pos)?;
    check_ending(&pos, &reading, &e.surface)?;
    Ok(UserDictEntry {
        reading,
        surface: e.surface.clone(),
        pos: e.pos.clone(),
        comment: e.comment.clone(),
    })
}

/// 設定画面に出す品詞の選択肢（同梱の `user_pos.tsv` の簡略品詞）
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PosOption {
    /// 簡略名（`UserDictEntry::pos` に書く名前）
    pub name: String,
    /// 活用型が必要（`動詞:五段-ラ行` のように `名前:活用型` と書く）
    pub needs_conj_type: bool,
}

/// 動詞の活用型の候補（Sudachi の品詞表の名前）。語尾の検査（`expected_endings`）ができる型だけ。
/// 実際に選べるのは、文法に品詞があるもの（[`verb_conj_types`]）。
const VERB_CONJ_TYPE_CANDIDATES: &[&str] = &[
    "五段-カ行",
    "五段-ガ行",
    "五段-サ行",
    "五段-タ行",
    "五段-ナ行",
    "五段-バ行",
    "五段-マ行",
    "五段-ラ行",
    "五段-ワア行",
    "上一段-ア行",
    "上一段-カ行",
    "上一段-ガ行",
    "上一段-ザ行",
    "上一段-タ行",
    "上一段-ナ行",
    "上一段-ハ行",
    "上一段-バ行",
    "上一段-マ行",
    "上一段-ラ行",
    "下一段-ア行",
    "下一段-カ行",
    "下一段-ガ行",
    "下一段-サ行",
    "下一段-ザ行",
    "下一段-タ行",
    "下一段-ダ行",
    "下一段-ナ行",
    "下一段-ハ行",
    "下一段-バ行",
    "下一段-マ行",
    "下一段-ラ行",
    "サ行変格",
    "カ行変格",
];

/// ユーザー辞書に書ける品詞の選択肢（表の順）。
pub fn pos_options() -> Vec<PosOption> {
    pos_defs()
        .iter()
        .map(|d| PosOption {
            name: d.simple.clone(),
            needs_conj_type: d.conj == Conj::Required,
        })
        .collect()
}

/// 動詞に指定できる活用型のうち、この文法の品詞表にあるもの（`動詞:<型>` の `<型>`）。
pub fn verb_conj_types(grammar: &dyn Grammar) -> Vec<String> {
    let Some(def) = pos_defs().iter().find(|d| d.simple == VERB_POS) else {
        return Vec::new();
    };
    VERB_CONJ_TYPE_CANDIDATES
        .iter()
        .filter(|t| {
            grammar
                .pos_id(&def.template.replace(TYPE_PLACEHOLDER, t))
                .is_some()
        })
        .map(|t| (*t).to_string())
        .collect()
}

/// 簡略品詞の対応表の検査結果（`ikari user-dict check`）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PosCheckReport {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    /// 確認した品詞の数
    pub checked: usize,
}

impl PosCheckReport {
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }
}

/// 品詞の対応表そのものの検査（ADR 0002 §10.4）。`extra_conj_types` は、追加で確かめる動詞の活用型。
pub fn check_pos_table(grammar: &dyn Grammar, extra_conj_types: &[String]) -> PosCheckReport {
    let mut report = PosCheckReport::default();
    let bos = grammar.bos_eos_id();
    let check_one = |label: &str, name: &str, report: &mut PosCheckReport| {
        report.checked += 1;
        let Some(id) = grammar.pos_id(name) else {
            report
                .errors
                .push(format!("{label}: 文法に品詞がありません: {name}"));
            return None;
        };
        let Some(info) = grammar.pos_info(id) else {
            report
                .errors
                .push(format!("{label}: 品詞の代表IDがありません: {name}"));
            return None;
        };
        let out_of_range = [
            (info.rep_right_id, info.rep_left_id),
            (bos, info.rep_left_id),
            (info.rep_right_id, bos),
        ]
        .iter()
        .any(|&(prev, next)| grammar.connection_cost(prev, next) == i16::MAX);
        if out_of_range {
            report.errors.push(format!(
                "{label}: 代表左右ID ({}, {}) が連接表の範囲外です",
                info.rep_left_id, info.rep_right_id
            ));
        }
        if info.rep_share_permille < MIN_REP_SHARE_PERMILLE {
            report.warnings.push(format!(
                "{label}: 代表IDと一致するエントリが{}‰しかありません（{MIN_REP_SHARE_PERMILLE}‰未満。代表IDでは足りない品詞）",
                info.rep_share_permille
            ));
        }
        Some(id)
    };

    let mut seen_names: Vec<String> = Vec::new();
    for def in pos_defs() {
        match def.conj {
            Conj::None | Conj::Auto => {
                if seen_names.contains(&def.template) {
                    continue;
                }
                seen_names.push(def.template.clone());
                let id = check_one(&def.simple, &def.template, &mut report);
                if def.conj == Conj::Auto
                    && let Some(id) = id
                {
                    check_conj_forms(grammar, &def.simple, &def.template, id, &mut report);
                }
            }
            Conj::Required => {}
        }
    }

    let mut types: Vec<String> = mozc_map()
        .iter()
        .filter_map(|(_, s)| s.strip_prefix("動詞:"))
        .filter(|t| !t.starts_with('@'))
        .map(str::to_string)
        .collect();
    types.extend(extra_conj_types.iter().cloned());
    types.sort();
    types.dedup();
    if let Some(def) = pos_defs().iter().find(|d| d.simple == VERB_POS) {
        for t in &types {
            let name = def.template.replace(TYPE_PLACEHOLDER, t);
            let label = format!("{VERB_POS}:{t}");
            let is_extra = extra_conj_types.contains(t);
            let before = report.errors.len();
            let id = check_one(&label, &name, &mut report);
            // Mozc の対応表だけにある型は、Sudachi の品詞表になくても警告に留める
            if !is_extra && report.errors.len() > before && id.is_none() {
                let msg = report.errors.pop().unwrap_or_default();
                report.warnings.push(msg);
                continue;
            }
            if let Some(id) = id {
                check_conj_forms(grammar, &label, &name, id, &mut report);
            }
        }
    }
    report
}

/// 終止形から 連用形 と 未然形 を導く規則があるか。
fn check_conj_forms(
    grammar: &dyn Grammar,
    label: &str,
    base_name: &str,
    base_id: u16,
    report: &mut PosCheckReport,
) {
    let Some(prefix) = base_name.strip_suffix("終止形-一般") else {
        return;
    };
    for form in REQUIRED_CONJ_FORMS {
        let target_name = format!("{prefix}{form}");
        let derived = grammar.pos_id(&target_name).is_some_and(|target| {
            grammar
                .conj_rules()
                .iter()
                .any(|r| r.base_pos == base_id && r.target_pos == target)
        });
        if !derived {
            report.errors.push(format!(
                "{label}: 終止形から {form} を導く活用規則がありません"
            ));
        }
    }
}
