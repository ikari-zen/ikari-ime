use std::collections::HashMap;

use super::mozc::infer_ichidan;
use super::pos::pos_defs;
use super::*;
use crate::dict::{DenseGrammar, Dictionary, PosInfo, UnknownParams};

/// 活用規則をモックした文法（grammar.bin v2 の代わり）
pub(crate) struct MockGrammar {
    inner: DenseGrammar,
    names: Vec<String>,
    infos: HashMap<u16, PosInfo>,
    rules: Vec<ConjRule>,
}

pub(crate) const NOUN: &str = "名詞,普通名詞,一般,*,*,*";
const SHIMO_MA: &str = "動詞,一般,*,*,下一段-マ行,";

impl MockGrammar {
    pub(crate) fn new() -> Self {
        let inner = DenseGrammar::new(
            40,
            40,
            100,
            UnknownParams {
                left_id: 1,
                right_id: 1,
                pos_id: 0,
                cost_per_char: 1,
            },
        );
        let mut g = Self {
            inner,
            names: Vec::new(),
            infos: HashMap::new(),
            rules: Vec::new(),
        };
        let mut next_id = 10u16;
        for def in pos_defs() {
            if def.conj == Conj::Required {
                continue;
            }
            g.add(&def.template, next_id, 1000);
            next_id += 1;
        }
        for form in [
            "終止形-一般",
            "連用形-一般",
            "未然形-一般",
            "連体形-一般",
            "仮定形-一般",
        ] {
            g.add(&format!("{SHIMO_MA}{form}"), next_id, 2000);
            next_id += 1;
        }
        for name in [
            "動詞,一般,*,*,五段-カ行,終止形-一般",
            "動詞,一般,*,*,五段-カ行,連用形-一般",
            "動詞,一般,*,*,五段-カ行,未然形-一般",
            "形容詞,一般,*,*,形容詞,連用形-一般",
            "形容詞,一般,*,*,形容詞,未然形-一般",
        ] {
            g.add(name, next_id, 2500);
            next_id += 1;
        }
        let base = g.pos_id(&format!("{SHIMO_MA}終止形-一般")).unwrap();
        for (form, add) in [
            ("連用形-一般", ""),
            ("未然形-一般", ""),
            ("連体形-一般", "る"),
            ("仮定形-一般", "れ"),
        ] {
            let target = g.pos_id(&format!("{SHIMO_MA}{form}")).unwrap();
            g.rules.push(ConjRule {
                base_pos: base,
                target_pos: target,
                strip_chars: 1,
                add_surface: add.to_string(),
                add_reading: add.to_string(),
            });
        }
        for (base_name, target_name, add) in [
            (
                "動詞,一般,*,*,五段-カ行,終止形-一般",
                "動詞,一般,*,*,五段-カ行,連用形-一般",
                "き",
            ),
            (
                "動詞,一般,*,*,五段-カ行,終止形-一般",
                "動詞,一般,*,*,五段-カ行,未然形-一般",
                "か",
            ),
            (
                "形容詞,一般,*,*,形容詞,終止形-一般",
                "形容詞,一般,*,*,形容詞,連用形-一般",
                "く",
            ),
            (
                "形容詞,一般,*,*,形容詞,終止形-一般",
                "形容詞,一般,*,*,形容詞,未然形-一般",
                "かろ",
            ),
        ] {
            let (base_pos, target_pos) =
                (g.pos_id(base_name).unwrap(), g.pos_id(target_name).unwrap());
            g.rules.push(ConjRule {
                base_pos,
                target_pos,
                strip_chars: 1,
                add_surface: add.to_string(),
                add_reading: add.to_string(),
            });
        }
        g
    }

    fn add(&mut self, name: &str, id: u16, cost: i16) {
        // pos_id は DenseGrammar の連番ではなく、このモックの名前表の添字にする
        while self.names.len() <= usize::from(id) {
            self.names.push(String::new());
        }
        self.names[usize::from(id)] = name.to_string();
        self.infos.insert(
            id,
            PosInfo {
                rep_left_id: id,
                rep_right_id: id,
                user_cost: cost,
                rep_share_permille: 990,
            },
        );
    }

    pub(crate) fn set_share(&mut self, name: &str, permille: u16) {
        let id = self.pos_id(name).unwrap();
        self.infos.get_mut(&id).unwrap().rep_share_permille = permille;
    }

    pub(crate) fn drop_rules(&mut self) {
        self.rules.clear();
    }
}

impl Grammar for MockGrammar {
    fn connection_cost(&self, prev: u16, next: u16) -> i16 {
        self.inner.connection_cost(prev, next)
    }
    fn bos_eos_id(&self) -> u16 {
        0
    }
    fn unknown(&self) -> UnknownParams {
        self.inner.unknown()
    }
    fn pos_name(&self, id: u16) -> Option<&str> {
        self.names
            .get(usize::from(id))
            .map(String::as_str)
            .filter(|s| !s.is_empty())
    }
    fn grammar_id(&self) -> u64 {
        42
    }
    fn pos_id(&self, name: &str) -> Option<u16> {
        self.names.iter().position(|n| n == name).map(|i| i as u16)
    }
    fn pos_info(&self, id: u16) -> Option<PosInfo> {
        self.infos.get(&id).copied()
    }
    fn conj_rules(&self) -> &[ConjRule] {
        &self.rules
    }
}

fn entry(reading: &str, surface: &str, pos: &str) -> UserDictEntry {
    UserDictEntry {
        reading: reading.to_string(),
        surface: surface.to_string(),
        pos: pos.to_string(),
        comment: String::new(),
    }
}

fn lookup(d: &InMemoryDictionary, reading: &str) -> Vec<(String, u16)> {
    let mut out = Vec::new();
    d.lookup_exact(reading, &mut |e| {
        out.push((e.surface.to_string(), e.pos_id))
    });
    out
}

#[test]
fn add_normalizes_validates_and_overwrites() {
    let mut d = UserDictionary::default();
    d.add(entry("イカリ", "ikari-ime", "短縮よみ")).unwrap();
    assert_eq!(d.entries()[0].reading, "いかり");
    let mut again = entry("いかり", "ikari-ime", "名詞");
    again.comment = "更新".into();
    d.add(again).unwrap();
    assert_eq!(d.len(), 1);
    assert_eq!(d.entries()[0].pos, "名詞");
    assert!(d.remove("イカリ", "ikari-ime"));
    assert!(!d.remove("いかり", "ikari-ime"));
    assert!(d.is_empty());
}

#[test]
fn add_rejects_bad_entries() {
    let mut d = UserDictionary::default();
    for e in [
        entry("", "あ", "名詞"),
        entry("abc", "あ", "名詞"),
        entry("あ", "", "名詞"),
        entry("あ", "a\tb", "名詞"),
        entry("あ", "亜", "名詞らしきもの"),
        entry("あ", "亜", "動詞"),
        entry("あ", "亜", "動詞:"),
        entry("あ", "亜", "名詞:五段-ラ行"),
        // 語尾が活用型と合わない
        entry("かきすすめん", "書き進めん", "動詞:下一段-マ行"),
        entry("かきすすめる", "書き進めん", "動詞:下一段-マ行"),
        entry("かく", "書く", "動詞:五段-ラ行"),
        entry("あかい", "赤", "形容詞"),
    ] {
        assert!(d.add(e.clone()).is_err(), "{e:?}");
    }
    assert!(d.is_empty());
    let long = "あ".repeat(ikari_dict_format::MAX_READING_CHARS + 1);
    assert!(d.add(entry(&long, "亜", "名詞")).is_err());
}

#[test]
fn file_roundtrip_with_comment() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("user_dict.tsv");
    let mut d = UserDictionary::default();
    d.add(entry("かきすすめる", "書き進める", "動詞:下一段-マ行"))
        .unwrap();
    let mut c = entry("いかり", "ikari-ime", "短縮よみ");
    c.comment = "自作IME".into();
    d.add(c).unwrap();
    d.save(&p).unwrap();
    let text = std::fs::read_to_string(&p).unwrap();
    assert!(text.starts_with("#ikari-user-dict\tschema=1\n"));
    assert!(text.contains("いかり\tikari-ime\t短縮よみ\t自作IME\n"));
    let back = UserDictionary::open(&p).unwrap();
    assert_eq!(back.entries(), d.entries());
    assert!(
        UserDictionary::open(&dir.path().join("none.tsv"))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn parse_rejects_invalid_files() {
    let h = "#ikari-user-dict\tschema=1\n";
    for text in [
        "",
        "かな\t仮名\t名詞\t\n",
        "#ikari-user-dict\tschema=2\n",
        &format!("{h}あ\t亜\n"),
        &format!("{h}あ\t亜\t名詞\tc\textra\n"),
        &format!("{h}あ\t亜\t謎\t\n"),
        &format!("{h}あ\t亜\t名詞\t\nあ\t亜\t名詞\t\n"),
    ] {
        assert!(UserDictionary::parse(text).is_err(), "{text:?}");
    }
    // コメント列は省略してよい
    let ok = UserDictionary::parse(&format!("{h}あ\t亜\t名詞\n")).unwrap();
    assert_eq!(ok.len(), 1);
}

#[test]
fn build_noun_uses_rep_ids_and_user_cost() {
    let g = MockGrammar::new();
    let mut d = UserDictionary::default();
    d.add(entry("いかり", "ikari-ime", "短縮よみ")).unwrap();
    let dict = d.build(&g).unwrap();
    let noun = g.pos_id(NOUN).unwrap();
    assert_eq!(
        lookup(&dict, "いかり"),
        vec![("ikari-ime".to_string(), noun)]
    );
    let mut got = None;
    dict.lookup_exact("いかり", &mut |e| {
        got = Some((e.left_id, e.right_id, e.cost, e.penalty))
    });
    let info = g.pos_info(noun).unwrap();
    assert_eq!(
        got,
        Some((info.rep_left_id, info.rep_right_id, info.user_cost, 0))
    );
    assert_eq!(dict.grammar_id(), g.grammar_id());
}

#[test]
fn build_expands_verb_conjugations_with_the_rules() {
    let g = MockGrammar::new();
    let mut d = UserDictionary::default();
    d.add(entry("かきすすめる", "書き進める", "動詞:下一段-マ行"))
        .unwrap();
    let dict = d.build(&g).unwrap();
    let id = |form: &str| g.pos_id(&format!("{SHIMO_MA}{form}")).unwrap();
    assert_eq!(
        lookup(&dict, "かきすすめる"),
        vec![
            ("書き進める".to_string(), id("終止形-一般")),
            ("書き進める".to_string(), id("連体形-一般")),
        ]
    );
    let ren = lookup(&dict, "かきすすめ");
    assert!(ren.contains(&("書き進め".to_string(), id("連用形-一般"))));
    assert!(ren.contains(&("書き進め".to_string(), id("未然形-一般"))));
    assert_eq!(
        lookup(&dict, "かきすすめれ"),
        vec![("書き進めれ".to_string(), id("仮定形-一般"))]
    );
}

#[test]
fn build_errors_are_specific() {
    let mut g = MockGrammar::new();
    let mut d = UserDictionary::default();
    // 文法に品詞がない（モックにない五段-ラ行）
    d.add(entry("つくる", "作る", "動詞:五段-ラ行")).unwrap();
    let err = d.build(&g).err().unwrap().to_string();
    assert!(err.contains("文法に品詞がありません"), "{err}");

    // 規則がない
    let mut d = UserDictionary::default();
    d.add(entry("かきすすめる", "書き進める", "動詞:下一段-マ行"))
        .unwrap();
    g.drop_rules();
    let err = d.build(&g).err().unwrap().to_string();
    assert!(err.contains("活用規則がありません"), "{err}");
}

#[test]
fn mozc_nouns_and_verbs() {
    let tsv = "!Microsoft IME Dictionary Tool\n\
               いかり\tikari-ime\t名詞\tコメント\n\
               かく\t書く\t動詞カ行五段\n\
               たべる\t食べる\t動詞一段\t\n\
               みる\t見る\t動詞一段\t\n\
               かきすすめる\t書き進める\t動詞一段\t\n\
               たなか\t田中\t姓\t\n";
    let e = UserDictionary::parse_mozc(tsv).unwrap();
    let pos: Vec<&str> = e.iter().map(|x| x.pos.as_str()).collect();
    assert_eq!(
        pos,
        [
            "名詞",
            "動詞:五段-カ行",
            "動詞:下一段-バ行",
            "動詞:上一段-マ行",
            "動詞:下一段-マ行",
            "姓",
        ]
    );
    assert_eq!(e[0].comment, "コメント");
}

#[test]
fn mozc_report_skips_unsupported_but_strict_parse_errors() {
    let tsv = "あ\t亜\t名詞\t\n\
               にっこり\t:-)\t顔文字\t\n\
               １\t一\t数\t\n\
               ABC\tabc\t名詞\t\n";
    let r = UserDictionary::parse_mozc_report(tsv).unwrap();
    assert_eq!(r.entries.len(), 1);
    assert_eq!(r.skipped.len(), 3);
    assert_eq!(r.skipped[0].line, 2);
    assert!(UserDictionary::parse_mozc(tsv).is_err());
    assert!(UserDictionary::parse_mozc("だけ\t二列\n").is_err());
}

#[test]
fn ichidan_inference_table() {
    assert_eq!(infer_ichidan("みる").as_deref(), Some("動詞:上一段-マ行"));
    assert_eq!(infer_ichidan("おきる").as_deref(), Some("動詞:上一段-カ行"));
    assert_eq!(infer_ichidan("ける").as_deref(), Some("動詞:下一段-カ行"));
    assert_eq!(infer_ichidan("いる").as_deref(), Some("動詞:上一段-ア行"));
    assert_eq!(infer_ichidan("える").as_deref(), Some("動詞:下一段-ア行"));
    assert_eq!(infer_ichidan("つくる"), None);
    assert_eq!(infer_ichidan("る"), None);
    assert_eq!(infer_ichidan("たべ"), None);
}

#[test]
fn add_all_is_atomic() {
    let mut d = UserDictionary::default();
    d.add(entry("あ", "亜", "名詞")).unwrap();
    let r = d.add_all(vec![entry("い", "伊", "名詞"), entry("う", "宇", "謎")]);
    assert!(r.is_err());
    assert_eq!(d.len(), 1);
    assert_eq!(
        d.add_all(vec![entry("い", "伊", "名詞"), entry("う", "宇", "名詞")])
            .unwrap(),
        2
    );
    assert_eq!(d.len(), 3);
}

#[test]
fn check_passes_for_a_consistent_grammar() {
    let g = MockGrammar::new();
    let mut d = UserDictionary::default();
    d.add(entry("かきすすめる", "書き進める", "動詞:下一段-マ行"))
        .unwrap();
    let r = d.check(&g);
    assert!(r.is_ok(), "{:?}", r.errors);
    assert!(r.checked >= 10);
}

#[test]
fn check_warns_on_low_rep_share_and_errors_on_missing_forms() {
    let mut g = MockGrammar::new();
    g.set_share(NOUN, 400);
    let r = check_pos_table(&g, &[]);
    assert!(r.is_ok(), "{:?}", r.errors);
    assert!(r.warnings.iter().any(|w| w.contains("400‰")));

    // 下一段-マ行 を使う利用者の辞書があるのに、規則がない
    let mut d = UserDictionary::default();
    d.add(entry("かきすすめる", "書き進める", "動詞:下一段-マ行"))
        .unwrap();
    g.drop_rules();
    let r = d.check(&g);
    assert!(!r.is_ok());
    assert!(r.errors.iter().any(|e| e.contains("活用規則")));
}

#[test]
fn pos_options_follow_the_bundled_table() {
    let options = pos_options();
    let find = |n: &str| options.iter().find(|o| o.name == n);
    assert_eq!(find("名詞").map(|o| o.needs_conj_type), Some(false));
    assert_eq!(find("形容詞").map(|o| o.needs_conj_type), Some(false));
    assert_eq!(find("動詞").map(|o| o.needs_conj_type), Some(true));
    assert!(options.iter().any(|o| o.name == "人名"));
    // 動詞は `名前:活用型` の形で項目になる
    let mut d = UserDictionary::default();
    d.add(entry("かきすすめる", "書き進める", "動詞:下一段-マ行"))
        .unwrap();
    assert!(d.add(entry("かきすすめる", "書き進める", "動詞")).is_err());
}

#[test]
fn verb_conj_types_are_limited_to_the_grammar() {
    let g = MockGrammar::new();
    // モックの文法にある型だけ（下一段-マ行、五段-カ行）
    assert_eq!(
        verb_conj_types(&g),
        vec!["五段-カ行".to_string(), "下一段-マ行".to_string()]
    );
    let empty = DenseGrammar::new(
        2,
        2,
        0,
        UnknownParams {
            left_id: 1,
            right_id: 1,
            pos_id: 0,
            cost_per_char: 1,
        },
    );
    assert!(verb_conj_types(&empty).is_empty());
}

#[test]
fn check_reports_missing_pos() {
    // 品詞を何も持たない文法
    let g = DenseGrammar::new(
        2,
        2,
        0,
        UnknownParams {
            left_id: 1,
            right_id: 1,
            pos_id: 0,
            cost_per_char: 1,
        },
    );
    let r = check_pos_table(&g, &[]);
    assert!(!r.is_ok());
    assert!(
        r.errors
            .iter()
            .any(|e| e.contains("文法に品詞がありません"))
    );
}
