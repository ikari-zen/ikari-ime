//! テスト用の小さな辞書と文法（表記の索引と lemma を持つ。実辞書の代わり）。

use ikari_core::{
    DenseGrammar, Dictionary, Lemma, UnknownParams, WordEntry, WordId, pos::is_content_word,
};
use ikari_dict_format::MAX_READING_CHARS;
use ikari_dict_format::reading::normalize_key;

pub const NOUN: &str = "名詞,普通名詞,一般,*,*,*";
pub const PARTICLE: &str = "助詞,格助詞,*,*,*,*";
pub const VERB: &str = "動詞,一般,*,*,五段-ラ行,終止形-一般";
pub const ADV: &str = "副詞,*,*,*,*,*";

struct Entry {
    reading: String,
    surface: String,
    lemma: String,
    id: u16,
    cost: i16,
    pos: u16,
}

pub struct TestDict {
    entries: Vec<Entry>,
}

impl TestDict {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub fn add(&mut self, reading: &str, surface: &str, id: u16, cost: i16, pos: u16) {
        self.add_with_lemma(reading, surface, surface, id, cost, pos);
    }

    pub fn add_with_lemma(
        &mut self,
        reading: &str,
        surface: &str,
        lemma: &str,
        id: u16,
        cost: i16,
        pos: u16,
    ) {
        self.entries.push(Entry {
            reading: normalize_key(reading),
            surface: surface.to_string(),
            lemma: lemma.to_string(),
            id,
            cost,
            pos,
        });
    }

    fn entry_at(&self, index: usize) -> WordEntry<'_> {
        let e = &self.entries[index];
        WordEntry {
            id: WordId {
                dict: 0,
                index: index as u32,
            },
            surface: &e.surface,
            left_id: e.id,
            right_id: e.id,
            cost: e.cost,
            pos_id: e.pos,
            penalty: 0,
        }
    }

    fn sorted(&self, mut hits: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
        hits.sort_by_key(|&(len, i)| (len, self.entries[i].cost, i));
        hits
    }
}

impl Dictionary for TestDict {
    fn common_prefix_search(&self, key: &str, f: &mut dyn FnMut(usize, WordEntry<'_>)) {
        let hits = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                key.starts_with(&e.reading) && e.reading.chars().count() <= MAX_READING_CHARS
            })
            .map(|(i, e)| (e.reading.len(), i))
            .collect();
        for (len, i) in self.sorted(hits) {
            f(len, self.entry_at(i));
        }
    }

    fn lookup_exact(&self, reading: &str, f: &mut dyn FnMut(WordEntry<'_>)) {
        let hits = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.reading == reading)
            .map(|(i, _)| (0, i))
            .collect();
        for (_, i) in self.sorted(hits) {
            f(self.entry_at(i));
        }
    }

    fn entry(&self, id: WordId) -> Option<WordEntry<'_>> {
        (id.dict == 0 && (id.index as usize) < self.entries.len())
            .then(|| self.entry_at(id.index as usize))
    }

    fn grammar_id(&self) -> u64 {
        0
    }

    fn common_prefix_search_surface(&self, text: &str, f: &mut dyn FnMut(usize, WordEntry<'_>)) {
        let hits = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| text.starts_with(&e.surface))
            .map(|(i, e)| (e.surface.len(), i))
            .collect();
        for (len, i) in self.sorted(hits) {
            f(len, self.entry_at(i));
        }
    }

    fn lemma(&self, id: WordId) -> Option<Lemma<'_>> {
        if id.dict != 0 {
            return None;
        }
        let e = self.entries.get(id.index as usize)?;
        Some(Lemma {
            text: &e.lemma,
            id: Some(id.index),
        })
    }
}

pub struct Mini {
    pub grammar: DenseGrammar,
    pub dict: TestDict,
}

/// 取り込みファイルの先頭に付けるヘッダ行
pub const HEADER_LINE: &str =
    r#"{"type":"header","format":"ikari-ingest","version":1,"prompt_version":"ingest-prompt-v1"}"#;

pub fn sentence_line(surface: &str, reading: &str) -> String {
    serde_json::json!({"type": "sentence", "surface": surface, "reading": reading}).to_string()
}

pub fn jsonl(sentences: &[(String, String)]) -> String {
    let mut out = format!("{HEADER_LINE}\n");
    for (s, r) in sentences {
        out.push_str(&sentence_line(s, r));
        out.push('\n');
    }
    out
}

/// 評価用に取り分けられない（学習に使われる）文にする。末尾に「、」を足して表記のハッシュを変える。
pub fn training(surface: &str, reading: &str) -> (String, String) {
    pad_until(surface, reading, false)
}

/// 評価用に取り分けられる文にする。
pub fn holdout(surface: &str, reading: &str) -> (String, String) {
    pad_until(surface, reading, true)
}

fn pad_until(surface: &str, reading: &str, want_holdout: bool) -> (String, String) {
    for k in 0..200 {
        let pad = "、".repeat(k);
        let s = format!("{surface}{pad}");
        if crate::holdout::is_holdout(&s) == want_holdout {
            return (s, format!("{reading}{pad}"));
        }
    }
    panic!("no padding found for {surface}");
}

/// ID: 1=名詞 2=助詞 3=動詞 4=副詞（左右同じ）。連接は全部 0、未知語は ID 1
pub fn mini() -> Mini {
    let mut grammar = DenseGrammar::new(
        5,
        5,
        0,
        UnknownParams {
            left_id: 1,
            right_id: 1,
            pos_id: 0,
            cost_per_char: 1,
        },
    );
    let noun = grammar.add_pos(NOUN);
    let particle = grammar.add_pos(PARTICLE);
    let verb = grammar.add_pos(VERB);
    let adv = grammar.add_pos(ADV);
    debug_assert!(is_content_word(NOUN) && !is_content_word(PARTICLE));
    let mut dict = TestDict::new();
    dict.add("ゆーざー", "ユーザー", 1, 3000, noun);
    dict.add_with_lemma("ゆーざ", "ユーザ", "ユーザー", 1, 3500, noun);
    dict.add("じしょ", "辞書", 1, 4000, noun);
    dict.add("たんご", "単語", 1, 4000, noun);
    dict.add("ひんし", "品詞", 1, 5000, noun);
    dict.add("ひんし", "瀕死", 1, 5500, noun);
    dict.add("に", "に", 2, 3000, particle);
    dict.add("を", "を", 2, 3000, particle);
    dict.add("は", "は", 2, 3000, particle);
    dict.add("の", "の", 2, 3000, particle);
    dict.add("とうろく", "登録", 1, 4000, noun);
    dict.add("する", "する", 3, 4000, verb);
    dict.add("はじめて", "初めて", 4, 4000, adv);
    dict.add("はじめて", "始めて", 3, 4500, verb);
    dict.add("しった", "知った", 3, 4000, verb);
    dict.add("はなし", "話", 1, 4000, noun);
    dict.add("その", "その", 4, 3000, adv);
    Mini { grammar, dict }
}
