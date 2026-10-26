//! テスト用のメモリ辞書と文法（フェーズ1のユーザー辞書の土台にもなる）。

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};

use ikari_dict_format::reading::normalize_key;
use ikari_dict_format::{Fnv1a64, MAX_READING_CHARS};

use super::{ConjRule, Dictionary, Grammar, Lemma, PosInfo, UnknownParams, WordEntry, WordId};

const BOS_EOS_ID: u16 = 0;

struct StoredEntry {
    /// 正規化した読み
    reading: String,
    surface: String,
    left_id: u16,
    right_id: u16,
    cost: i16,
    pos_id: u16,
    penalty: i16,
    /// `InMemoryDictionary::lemmas` の添字
    lemma: u32,
}

/// メモリ上の辞書。語ID の index は `insert` した順の添字（`dict` は常に 0）。
pub struct InMemoryDictionary {
    grammar_id: u64,
    entries: Vec<StoredEntry>,
    /// 正規化した読み -> エントリ添字（コスト昇順、同コストは挿入順）
    index: BTreeMap<String, Vec<u32>>,
    /// 表記 -> エントリ添字（コスト昇順、同コストは挿入順）。`common_prefix_search_surface` 用
    surface_index: BTreeMap<String, Vec<u32>>,
    /// lemma の文字列（出現順に番号を振る。lemma を指定しなければ表記と同じ）
    lemmas: Vec<String>,
    lemma_ids: HashMap<String, u32>,
    /// ユーザーが登録した辞書（ユーザー辞書）か。語彙集は false
    user_authored: bool,
}

impl InMemoryDictionary {
    pub fn new(grammar_id: u64) -> Self {
        Self {
            grammar_id,
            entries: Vec::new(),
            index: BTreeMap::new(),
            surface_index: BTreeMap::new(),
            lemmas: Vec::new(),
            lemma_ids: HashMap::new(),
            user_authored: false,
        }
    }

    /// この辞書をユーザー自身が登録したもの（ユーザー辞書）として印を付ける。
    pub fn mark_user_authored(&mut self) {
        self.user_authored = true;
    }

    fn intern_lemma(&mut self, lemma: &str) -> u32 {
        if let Some(&id) = self.lemma_ids.get(lemma) {
            return id;
        }
        let id = u32::try_from(self.lemmas.len()).expect("InMemoryDictionary: too many lemmas");
        self.lemmas.push(lemma.to_owned());
        self.lemma_ids.insert(lemma.to_owned(), id);
        id
    }

    /// `reading` は `normalize_key` を通して格納する。
    /// 空の読みや `MAX_READING_CHARS` を超える読みは、`lookup_exact` でしか引けない。
    /// 表記は `MAX_READING_CHARS` 文字までが `common_prefix_search_surface` で引ける。
    ///
    /// penalty は 0、lemma は表記と同じになる。変えるには `set_penalty` / `set_lemma` を使う。
    /// 返す語ID は、この辞書の中の添字（`dict` は 0）。
    pub fn insert(
        &mut self,
        reading: &str,
        surface: &str,
        left_id: u16,
        right_id: u16,
        cost: i16,
        pos_id: u16,
    ) -> WordId {
        let id = u32::try_from(self.entries.len()).expect("InMemoryDictionary: too many entries");
        let lemma = self.intern_lemma(surface);
        let reading = normalize_key(reading);
        self.entries.push(StoredEntry {
            reading: reading.clone(),
            surface: surface.to_owned(),
            left_id,
            right_id,
            cost,
            pos_id,
            penalty: 0,
            lemma,
        });
        let entries = &self.entries;
        let ids = self.index.entry(reading).or_default();
        let at = ids.partition_point(|&i| entries[i as usize].cost <= cost);
        ids.insert(at, id);
        let ids = self.surface_index.entry(surface.to_owned()).or_default();
        let at = ids.partition_point(|&i| entries[i as usize].cost <= cost);
        ids.insert(at, id);
        WordId { dict: 0, index: id }
    }

    /// エントリの `ime_penalty` を設定する（`IME_EXCLUDED` なら変換では使わない）。
    /// 存在しない語IDなら何もせず false。
    pub fn set_penalty(&mut self, id: WordId, penalty: i16) -> bool {
        match self.entry_mut(id) {
            Some(e) => {
                e.penalty = penalty;
                true
            }
            None => false,
        }
    }

    /// エントリの lemma（正規化表記）を設定する。存在しない語IDなら何もせず false。
    pub fn set_lemma(&mut self, id: WordId, lemma: &str) -> bool {
        if self.entry_mut(id).is_none() {
            return false;
        }
        let lemma_id = self.intern_lemma(lemma);
        match self.entry_mut(id) {
            Some(e) => {
                e.lemma = lemma_id;
                true
            }
            None => false,
        }
    }

    fn entry_mut(&mut self, id: WordId) -> Option<&mut StoredEntry> {
        if id.dict != 0 {
            return None;
        }
        self.entries.get_mut(id.index as usize)
    }

    fn word_entry(&self, index: u32) -> Option<WordEntry<'_>> {
        let e = self.entries.get(index as usize)?;
        Some(WordEntry {
            id: WordId { dict: 0, index },
            surface: &e.surface,
            left_id: e.left_id,
            right_id: e.right_id,
            cost: e.cost,
            pos_id: e.pos_id,
            penalty: e.penalty,
        })
    }
}

impl Dictionary for InMemoryDictionary {
    fn common_prefix_search(&self, key: &str, f: &mut dyn FnMut(usize, WordEntry<'_>)) {
        for (i, c) in key.char_indices().take(MAX_READING_CHARS) {
            let end = i + c.len_utf8();
            if let Some(ids) = self.index.get(&key[..end]) {
                for &id in ids {
                    if let Some(e) = self.word_entry(id) {
                        f(end, e);
                    }
                }
            }
        }
    }

    fn lookup_exact(&self, reading: &str, f: &mut dyn FnMut(WordEntry<'_>)) {
        if let Some(ids) = self.index.get(reading) {
            for &id in ids {
                if let Some(e) = self.word_entry(id) {
                    f(e);
                }
            }
        }
    }

    fn entry(&self, id: WordId) -> Option<WordEntry<'_>> {
        if id.dict != 0 {
            return None;
        }
        self.word_entry(id.index)
    }

    fn grammar_id(&self) -> u64 {
        self.grammar_id
    }

    fn common_prefix_search_surface(&self, text: &str, f: &mut dyn FnMut(usize, WordEntry<'_>)) {
        for (i, c) in text.char_indices().take(MAX_READING_CHARS) {
            let end = i + c.len_utf8();
            if let Some(ids) = self.surface_index.get(&text[..end]) {
                for &id in ids {
                    if let Some(e) = self.word_entry(id) {
                        f(end, e);
                    }
                }
            }
        }
    }

    fn is_user_authored(&self, id: WordId) -> bool {
        self.user_authored && id.dict == 0
    }

    /// 挿入のときの読み（`normalize_key` を通したもの）。
    fn reading(&self, id: WordId) -> Option<Cow<'_, str>> {
        if id.dict != 0 {
            return None;
        }
        let e = self.entries.get(id.index as usize)?;
        Some(Cow::Borrowed(e.reading.as_str()))
    }

    /// lemma 番号は挿入順に振った、この辞書の中の番号。
    fn lemma(&self, id: WordId) -> Option<Lemma<'_>> {
        if id.dict != 0 {
            return None;
        }
        let e = self.entries.get(id.index as usize)?;
        Some(Lemma {
            text: self.lemmas.get(e.lemma as usize)?,
            id: Some(e.lemma),
        })
    }
}

/// 密行列の連接表を持つ文法。`bos_eos_id` は 0。
pub struct DenseGrammar {
    prev_right_size: u16,
    next_left_size: u16,
    /// `matrix[next_left * prev_right_size + prev_right]`（本番の grammar.bin と同じ並び）
    matrix: Vec<i16>,
    unknown: UnknownParams,
    pos_names: Vec<String>,
    pos_infos: BTreeMap<u16, PosInfo>,
    conj_rules: Vec<ConjRule>,
}

impl DenseGrammar {
    /// bos_eos_id = 0
    pub fn new(
        prev_right_size: u16,
        next_left_size: u16,
        default_cost: i16,
        unknown: UnknownParams,
    ) -> Self {
        let len = usize::from(prev_right_size) * usize::from(next_left_size);
        Self {
            prev_right_size,
            next_left_size,
            matrix: vec![default_cost; len],
            unknown,
            pos_names: Vec::new(),
            pos_infos: BTreeMap::new(),
            conj_rules: Vec::new(),
        }
    }

    /// 品詞の代表左右IDとユーザー辞書のコストを設定する（`Grammar::pos_info` が返す）。
    /// `grammar_id` は変えない。
    pub fn set_pos_info(&mut self, pos_id: u16, info: PosInfo) {
        self.pos_infos.insert(pos_id, info);
    }

    /// 活用規則を足す（`Grammar::conj_rules` が追加順に返す）。`grammar_id` は変えない。
    pub fn add_conj_rule(&mut self, rule: ConjRule) {
        self.conj_rules.push(rule);
    }

    /// 連接コストを設定する。範囲外の ID はテストコードの誤りなのでパニックする。
    pub fn set(&mut self, prev_right: u16, next_left: u16, cost: i16) {
        assert!(
            prev_right < self.prev_right_size && next_left < self.next_left_size,
            "DenseGrammar::set out of range: ({prev_right}, {next_left}) for {}x{}",
            self.prev_right_size,
            self.next_left_size
        );
        let at =
            usize::from(next_left) * usize::from(self.prev_right_size) + usize::from(prev_right);
        self.matrix[at] = cost;
    }

    /// 品詞名を登録して pos_id を返す。同じ名前なら既存の ID を返す。
    pub fn add_pos(&mut self, name: &str) -> u16 {
        if let Some(i) = self.pos_names.iter().position(|n| n == name) {
            return i as u16;
        }
        let id =
            u16::try_from(self.pos_names.len()).expect("DenseGrammar: too many parts of speech");
        self.pos_names.push(name.to_owned());
        id
    }
}

impl Grammar for DenseGrammar {
    fn connection_cost(&self, prev_right_id: u16, next_left_id: u16) -> i16 {
        if prev_right_id >= self.prev_right_size || next_left_id >= self.next_left_size {
            return i16::MAX;
        }
        let at = usize::from(next_left_id) * usize::from(self.prev_right_size)
            + usize::from(prev_right_id);
        self.matrix.get(at).copied().unwrap_or(i16::MAX)
    }

    fn bos_eos_id(&self) -> u16 {
        BOS_EOS_ID
    }

    fn unknown(&self) -> UnknownParams {
        self.unknown
    }

    fn pos_name(&self, pos_id: u16) -> Option<&str> {
        self.pos_names.get(usize::from(pos_id)).map(String::as_str)
    }

    fn pos_id(&self, name: &str) -> Option<u16> {
        self.pos_names
            .iter()
            .position(|n| n == name)
            .and_then(|i| u16::try_from(i).ok())
    }

    fn pos_info(&self, pos_id: u16) -> Option<PosInfo> {
        self.pos_infos.get(&pos_id).copied()
    }

    fn conj_rules(&self) -> &[ConjRule] {
        &self.conj_rules
    }

    /// サイズ、連接表、品詞表の内容から計算する（内容が変われば値も変わる）。
    fn grammar_id(&self) -> u64 {
        let mut h = Fnv1a64::new();
        h.update(&self.prev_right_size.to_le_bytes());
        h.update(&self.next_left_size.to_le_bytes());
        h.update(&BOS_EOS_ID.to_le_bytes());
        for c in &self.matrix {
            h.update(&c.to_le_bytes());
        }
        for name in &self.pos_names {
            h.update(name.as_bytes());
            h.update(&[0]);
        }
        h.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unk() -> UnknownParams {
        UnknownParams {
            left_id: 1,
            right_id: 1,
            pos_id: 0,
            cost_per_char: 100,
        }
    }

    fn collect_prefix(d: &InMemoryDictionary, key: &str) -> Vec<(usize, String, i16)> {
        let mut out = Vec::new();
        d.common_prefix_search(key, &mut |len, e| {
            out.push((len, e.surface.to_owned(), e.cost))
        });
        out
    }

    #[test]
    fn prefix_search_matches_all_prefixes_in_cost_order() {
        let mut d = InMemoryDictionary::new(7);
        d.insert("へん", "変", 1, 1, 500, 0);
        d.insert("へんか", "変化", 1, 1, 300, 0);
        d.insert("へんか", "返歌", 1, 1, 100, 0);
        d.insert("へんかん", "変換", 1, 1, 200, 0);
        d.insert("ほか", "他", 1, 1, 1, 0);
        let r = collect_prefix(&d, "へんかんする");
        let want = vec![
            (6, "変".to_owned(), 500),
            (9, "返歌".to_owned(), 100),
            (9, "変化".to_owned(), 300),
            (12, "変換".to_owned(), 200),
        ];
        assert_eq!(r, want);
    }

    #[test]
    fn insert_normalizes_reading_to_hiragana() {
        let mut d = InMemoryDictionary::new(0);
        d.insert("ヒンシ", "品詞", 1, 1, 10, 0);
        let mut n = 0;
        d.lookup_exact("ひんし", &mut |e| {
            assert_eq!(e.surface, "品詞");
            n += 1;
        });
        assert_eq!(n, 1);
        assert_eq!(collect_prefix(&d, "ひんし").len(), 1);
    }

    #[test]
    fn same_reading_is_sorted_by_cost_and_stable() {
        let mut d = InMemoryDictionary::new(0);
        d.insert("あ", "c", 1, 1, 30, 0);
        d.insert("あ", "a1", 1, 1, 10, 0);
        d.insert("あ", "b", 1, 1, 20, 0);
        d.insert("あ", "a2", 1, 1, 10, 0);
        let got: Vec<_> = collect_prefix(&d, "あ").into_iter().map(|x| x.1).collect();
        assert_eq!(got, ["a1", "a2", "b", "c"]);
    }

    #[test]
    fn prefix_search_is_limited_to_max_reading_chars() {
        let mut d = InMemoryDictionary::new(0);
        let max = "あ".repeat(MAX_READING_CHARS);
        let over = "あ".repeat(MAX_READING_CHARS + 1);
        d.insert(&max, "max", 1, 1, 1, 0);
        d.insert(&over, "over", 1, 1, 1, 0);
        let r = collect_prefix(&d, &over);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].1, "max");
    }

    #[test]
    fn entry_roundtrip_and_out_of_range() {
        let mut d = InMemoryDictionary::new(0);
        d.insert("あ", "亜", 3, 4, -5, 6);
        let mut id = None;
        d.lookup_exact("あ", &mut |e| id = Some(e.id));
        let e = d.entry(id.unwrap()).unwrap();
        assert_eq!(
            (e.surface, e.left_id, e.right_id, e.cost, e.pos_id),
            ("亜", 3, 4, -5, 6)
        );
        assert!(d.entry(WordId { dict: 0, index: 1 }).is_none());
        assert!(d.entry(WordId { dict: 1, index: 0 }).is_none());
    }

    #[test]
    fn dense_grammar_costs_and_bounds() {
        let mut g = DenseGrammar::new(3, 4, 50, unk());
        g.set(2, 3, -7);
        g.set(0, 1, 9);
        assert_eq!(g.connection_cost(2, 3), -7);
        assert_eq!(g.connection_cost(0, 1), 9);
        assert_eq!(g.connection_cost(1, 1), 50);
        // 転置していないこと（prev=3 は範囲外、next=3 は範囲内）
        assert_eq!(g.connection_cost(3, 0), i16::MAX);
        assert_eq!(g.connection_cost(0, 4), i16::MAX);
        assert_eq!(g.connection_cost(u16::MAX, u16::MAX), i16::MAX);
        assert_eq!(g.bos_eos_id(), 0);
    }

    #[test]
    fn dense_grammar_pos_names_and_id() {
        let mut g = DenseGrammar::new(2, 2, 0, unk());
        let before = g.grammar_id();
        let a = g.add_pos("名詞,普通名詞,一般,*,*,*");
        let b = g.add_pos("助詞,格助詞,*,*,*,*");
        assert_eq!(g.add_pos("名詞,普通名詞,一般,*,*,*"), a);
        assert_ne!(a, b);
        assert_eq!(g.pos_name(b), Some("助詞,格助詞,*,*,*,*"));
        assert_eq!(g.pos_name(9), None);
        assert_ne!(g.grammar_id(), before);
        let id = g.grammar_id();
        g.set(1, 1, 5);
        assert_ne!(g.grammar_id(), id);
    }

    /// `grammar_id` は辞書側の値と照合されうるので、計算の式が変わってはいけない（現行の値の固定）。
    #[test]
    fn dense_grammar_id_known_value() {
        let mut g = DenseGrammar::new(2, 2, 0, unk());
        g.add_pos("名詞,普通名詞,一般,*,*,*");
        g.set(1, 1, 5);
        assert_eq!(g.grammar_id(), 0x327e52198a02e85e);
    }

    #[test]
    fn reading_is_the_normalized_reading_given_at_insert() {
        let mut d = InMemoryDictionary::new(0);
        let a = d.insert("ヒンシ", "品詞", 1, 1, 100, 0);
        let b = d.insert("ひんし", "瀕死", 1, 1, 200, 0);
        assert_eq!(d.reading(a).as_deref(), Some("ひんし"));
        assert_eq!(d.reading(b).as_deref(), Some("ひんし"));
        // lookup_exact の逆引きになっている
        let mut found = false;
        d.lookup_exact(&d.reading(a).unwrap(), &mut |e| found |= e.id == a);
        assert!(found);
        assert!(d.reading(WordId { dict: 0, index: 2 }).is_none());
        assert!(d.reading(WordId { dict: 1, index: 0 }).is_none());
    }
}
