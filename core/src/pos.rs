//! 品詞名（`名詞,普通名詞,一般,*,*,*` の形）の判定（ADR 0002 §5.2）。

const POS_FIELD_SEPARATOR: char = ',';
const CONTENT_POS1: [&str; 5] = ["名詞", "動詞", "形容詞", "形状詞", "副詞"];
const NON_CONTENT_POS2: [&str; 2] = ["非自立可能", "数詞"];

/// 品詞1（最初の列）。区切りがなければ全体を返す。
pub fn pos1(pos_name: &str) -> &str {
    pos_name.split(POS_FIELD_SEPARATOR).next().unwrap_or("")
}

/// 品詞1 が 名詞/動詞/形容詞/形状詞/副詞 のどれかで、品詞2 が 非自立可能 でも 数詞 でもない。
pub fn is_content_word(pos_name: &str) -> bool {
    let mut fields = pos_name.split(POS_FIELD_SEPARATOR);
    let pos1 = fields.next().unwrap_or("");
    let pos2 = fields.next().unwrap_or("");
    CONTENT_POS1.contains(&pos1) && !NON_CONTENT_POS2.contains(&pos2)
}

/// 活用形（品詞名の 6 列目）の見出し語の部分を取る位置
const CONJUGATION_FORM_FIELD: usize = 5;
/// 活用形の `-` の前の頭と、詳細の区切り（`連用形-一般` の `-`）
const CONJUGATION_DETAIL_SEPARATOR: char = '-';
const TERMINAL_FORM: &str = "終止形";
const ATTRIBUTIVE_FORM: &str = "連体形";
/// 終止形と連体形を 1 つにまとめた値
const TERMINAL_OR_ATTRIBUTIVE: &str = "終止・連体形";

/// 活用形の頭（`連用形-一般` -> `連用形`）。終止形と連体形は同じ値にする。活用しない語は `*`
fn conjugation_form_head(pos_name: &str) -> &str {
    let form = pos_name
        .split(POS_FIELD_SEPARATOR)
        .nth(CONJUGATION_FORM_FIELD)
        .unwrap_or("*");
    let head = form
        .split(CONJUGATION_DETAIL_SEPARATOR)
        .next()
        .unwrap_or("*");
    if head == TERMINAL_FORM || head == ATTRIBUTIVE_FORM {
        TERMINAL_OR_ATTRIBUTIVE
    } else {
        head
    }
}

/// 学習の word の加点で「同じ役割の語」とみなすか（ADR 0044 §4）。品詞1 と活用形の頭が両方同じなら真。
/// 品詞2 以降と活用型は見ない（Sudachi は普通の「行く」「合う」を 非自立可能 にしているため）。
pub fn same_word_role(a: &str, b: &str) -> bool {
    pos1(a) == pos1(b) && conjugation_form_head(a) == conjugation_form_head(b)
}

/// 語が文節の中でどう振る舞うか（ADR 0006 §2.2 の表）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SegmentRole {
    /// 助詞、助動詞、接尾辞、補助記号。左の文節に付ける
    Attach,
    /// 品詞2 が 非自立可能 の名詞・動詞・形容詞。Sudachi は「行く」「切る」「ある」「いい」のような普通の用言も
    /// この品詞にしている（補助動詞の用法と同じ項目）ので、品詞だけでは補助かどうか決められない。
    /// 左の語が `ClauseWord::closes_phrase`（格助詞などで言い切った形）なら新しい文節を始め、そうでなければ左の文節に付ける
    Dependent,
    /// 名詞（非自立可能を除く）と未知語。左の文節の最後の語が名詞・接頭辞・接尾辞（名詞的）なら付け、そうでなければ新しい文節を始める
    Compound,
    /// 接頭辞。新しい文節を始め、次の語はその文節に付ける
    Prefix,
    /// 動詞、形容詞、形状詞、副詞、連体詞、接続詞、感動詞、代名詞、記号（補助記号以外）。新しい文節を始める。直前が接頭辞なら付ける
    Start,
}

/// 品詞2 が 非自立可能 の語（「いる」「ほしい」「こと」の一部）。左の文節に付ける
const DEPENDENT_POS2: &str = "非自立可能";
/// 品詞1 のうち、品詞2 が 非自立可能 なら付属語になるもの
const DEPENDENT_CAPABLE_POS1: [&str; 3] = ["名詞", "動詞", "形容詞"];
/// 常に左の文節に付ける品詞1。空白もここに入れる（文節の先頭にはならない）
const ATTACH_POS1: [&str; 5] = ["助詞", "助動詞", "接尾辞", "補助記号", "空白"];
const NOUN_POS1: &str = "名詞";
const PREFIX_POS1: &str = "接頭辞";
const SUFFIX_POS1: &str = "接尾辞";
/// 名詞に付く接尾辞の品詞2
const NOMINAL_SUFFIX_POS2: &str = "名詞的";

/// 品詞名から文節の中での役割を決める純関数（ADR 0006 §2.2）。品詞名は `grammar.pos_name(pos_id)` の形。
/// 未知語（`Segment.word == None`）は呼ぶ側が `Compound` として扱う。
/// 表にない品詞は `Start`（新しい文節を始める）にする。
pub fn segment_role(pos_name: &str) -> SegmentRole {
    let mut fields = pos_name.split(POS_FIELD_SEPARATOR);
    let pos1 = fields.next().unwrap_or("");
    let pos2 = fields.next().unwrap_or("");
    if ATTACH_POS1.contains(&pos1) {
        return SegmentRole::Attach;
    }
    if DEPENDENT_CAPABLE_POS1.contains(&pos1) && pos2 == DEPENDENT_POS2 {
        return SegmentRole::Dependent;
    }
    match pos1 {
        NOUN_POS1 => SegmentRole::Compound,
        PREFIX_POS1 => SegmentRole::Prefix,
        _ => SegmentRole::Start,
    }
}

/// 右に名詞が来たとき、同じ文節の複合名詞として付けられる語か（名詞、接頭辞、接尾辞（名詞的））。
/// 未知語は呼ぶ側が true として扱う。
pub fn joins_following_noun(pos_name: &str) -> bool {
    let mut fields = pos_name.split(POS_FIELD_SEPARATOR);
    let pos1 = fields.next().unwrap_or("");
    let pos2 = fields.next().unwrap_or("");
    pos1 == NOUN_POS1 || pos1 == PREFIX_POS1 || (pos1 == SUFFIX_POS1 && pos2 == NOMINAL_SUFFIX_POS2)
}

/// 接続助詞（て、ば、ながら、…）は用言に続くので、右の補助的な用言を同じ文節に付ける
const CONNECTIVE_PARTICLE_POS2: &str = "接続助詞";
const PARTICLE_POS1: &str = "助詞";
/// 後ろに用言が来たら、その用言は別の文節になる品詞1（副詞、連体詞、接続詞）
const PHRASE_ENDING_POS1: [&str; 3] = ["副詞", "連体詞", "接続詞"];

/// 右に `SegmentRole::Dependent` の語が来たとき、その語を新しい文節にする語か。
/// 接続助詞以外の助詞（を、が、に、は、で、…）と、副詞・連体詞・接続詞。未知語は呼ぶ側が false として扱う。
pub fn closes_phrase(pos_name: &str) -> bool {
    let mut fields = pos_name.split(POS_FIELD_SEPARATOR);
    let pos1 = fields.next().unwrap_or("");
    let pos2 = fields.next().unwrap_or("");
    (pos1 == PARTICLE_POS1 && pos2 != CONNECTIVE_PARTICLE_POS2)
        || PHRASE_ENDING_POS1.contains(&pos1)
}

/// 文節の区切りの規則に必要な、語ごとの情報
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClauseWord {
    pub role: SegmentRole,
    /// `joins_following_noun`
    pub joins_following_noun: bool,
    /// `closes_phrase`
    pub closes_phrase: bool,
}

/// 語の並びを文節にまとめる（ADR 0006 §2.2）。`true` の位置が新しい文節の先頭。最初の語は必ず先頭。
pub fn clause_starts(words: &[ClauseWord]) -> Vec<bool> {
    let mut starts = Vec::with_capacity(words.len());
    let mut prev: Option<ClauseWord> = None;
    // 先頭の語が付属語（読みが助詞から始まる「をしよう」）なら、それだけの文節にはしない
    let mut leading_attached = false;
    for word in words {
        let starts_here = match (prev, word.role) {
            (None, _) => true,
            (Some(_), SegmentRole::Dependent) if leading_attached => false,
            (Some(_), SegmentRole::Attach) => false,
            (Some(p), SegmentRole::Dependent) => p.closes_phrase,
            (Some(_), SegmentRole::Prefix) => true,
            (Some(p), SegmentRole::Start) => p.role != SegmentRole::Prefix,
            (Some(p), SegmentRole::Compound) => !p.joins_following_noun,
        };
        leading_attached =
            prev.is_none() && matches!(word.role, SegmentRole::Attach | SegmentRole::Dependent);
        starts.push(starts_here);
        prev = Some(*word);
    }
    starts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_words() {
        assert!(is_content_word("名詞,普通名詞,一般,*,*,*"));
        assert!(is_content_word("名詞,固有名詞,人名,一般,*,*"));
        assert!(is_content_word("動詞,一般,*,*,五段-ラ行,終止形-一般"));
        assert!(is_content_word("形容詞,一般,*,*,形容詞,終止形-一般"));
        assert!(is_content_word("形状詞,一般,*,*,*,*"));
        assert!(is_content_word("副詞,*,*,*,*,*"));
    }

    #[test]
    fn non_content_words() {
        assert!(!is_content_word("名詞,非自立可能,*,*,*,*"));
        assert!(!is_content_word(
            "動詞,非自立可能,*,*,五段-ラ行,連用形-促音便"
        ));
        assert!(!is_content_word("名詞,数詞,*,*,*,*"));
        assert!(!is_content_word("助詞,格助詞,*,*,*,*"));
        assert!(!is_content_word("助動詞,*,*,*,助動詞-タ,終止形-一般"));
        assert!(!is_content_word("接尾辞,名詞的,一般,*,*,*"));
        assert!(!is_content_word("連体詞,*,*,*,*,*"));
        assert!(!is_content_word("記号,一般,*,*,*,*"));
    }

    #[test]
    fn edge_cases() {
        assert!(!is_content_word(""));
        assert!(!is_content_word("*"));
        // 品詞2 がなければ非自立可能でも数詞でもない
        assert!(is_content_word("名詞"));
        assert!(!is_content_word("名詞のようなもの,一般"));
    }

    #[test]
    fn word_role_compares_pos1_and_conjugation_head() {
        let noun = "名詞,普通名詞,サ変可能,*,*,*";
        let volitional = "動詞,非自立可能,*,*,サ行変格,意志推量形";
        assert!(!same_word_role(volitional, noun));
        // 品詞2 が違っても、品詞1 と活用形が同じなら同じ役割
        assert!(same_word_role(
            "名詞,普通名詞,一般,*,*,*",
            "名詞,固有名詞,地名,一般,*,*"
        ));
        assert!(same_word_role(
            "動詞,非自立可能,*,*,五段-ワ行,終止形-一般",
            "動詞,一般,*,*,五段-ワ行,終止形-一般"
        ));
        // 連用形の詳細は無視する
        assert!(same_word_role(
            "動詞,一般,*,*,五段-ラ行,連用形-促音便",
            "動詞,一般,*,*,五段-ラ行,連用形-一般"
        ));
        // 書き（連用形）と下記（名詞）、書き（連用形）と書く（終止形）は違う
        assert!(!same_word_role(
            "動詞,一般,*,*,五段-カ行,連用形-一般",
            "名詞,普通名詞,一般,*,*,*"
        ));
        assert!(!same_word_role(
            "動詞,一般,*,*,五段-カ行,連用形-一般",
            "動詞,一般,*,*,五段-カ行,終止形-一般"
        ));
    }

    #[test]
    fn word_role_treats_terminal_and_attributive_alike() {
        assert!(same_word_role(
            "動詞,一般,*,*,五段-ラ行,終止形-一般",
            "動詞,一般,*,*,下一段-ア行,連体形-一般"
        ));
        assert!(!same_word_role(
            "動詞,一般,*,*,五段-ラ行,終止形-一般",
            "動詞,一般,*,*,五段-ラ行,未然形-一般"
        ));
    }

    #[test]
    fn word_role_with_missing_fields() {
        assert!(same_word_role("名詞", "名詞,普通名詞,一般,*,*,*"));
        assert!(!same_word_role(
            "副詞,*,*,*,*,*",
            "名詞,普通名詞,一般,*,*,*"
        ));
    }

    #[test]
    fn pos1_splits_first_field() {
        assert_eq!(pos1("名詞,普通名詞,一般,*,*,*"), "名詞");
        assert_eq!(pos1("副詞"), "副詞");
        assert_eq!(pos1(""), "");
    }

    fn word(pos: &str) -> ClauseWord {
        ClauseWord {
            role: segment_role(pos),
            joins_following_noun: joins_following_noun(pos),
            closes_phrase: closes_phrase(pos),
        }
    }

    fn clauses(poses: &[&str]) -> Vec<usize> {
        let words: Vec<ClauseWord> = poses.iter().map(|p| word(p)).collect();
        let mut sizes: Vec<usize> = Vec::new();
        for starts in clause_starts(&words) {
            if starts {
                sizes.push(1);
            } else {
                *sizes.last_mut().unwrap() += 1;
            }
        }
        sizes
    }

    const NOUN: &str = "名詞,普通名詞,一般,*,*,*";
    const NOUN_DEPENDENT: &str = "名詞,非自立可能,*,*,*,*";
    const VERB: &str = "動詞,一般,*,*,五段-ラ行,終止形-一般";
    const VERB_DEPENDENT: &str = "動詞,非自立可能,*,*,上一段-ア行,終止形-一般";
    const ADJ_DEPENDENT: &str = "形容詞,非自立可能,*,*,形容詞,終止形-一般";
    const PARTICLE: &str = "助詞,格助詞,*,*,*,*";
    const AUX: &str = "助動詞,*,*,*,助動詞-タ,終止形-一般";
    const SUFFIX_NOUN: &str = "接尾辞,名詞的,一般,*,*,*";
    const SUFFIX_VERB: &str = "接尾辞,動詞的,*,*,*,*";
    const PREFIX: &str = "接頭辞,*,*,*,*,*";
    const PUNCT: &str = "補助記号,句点,*,*,*,*";

    #[test]
    fn roles_follow_the_table() {
        assert_eq!(segment_role(PARTICLE), SegmentRole::Attach);
        assert_eq!(segment_role(AUX), SegmentRole::Attach);
        assert_eq!(segment_role(SUFFIX_NOUN), SegmentRole::Attach);
        assert_eq!(segment_role(SUFFIX_VERB), SegmentRole::Attach);
        assert_eq!(segment_role(PUNCT), SegmentRole::Attach);
        assert_eq!(segment_role(NOUN_DEPENDENT), SegmentRole::Dependent);
        assert_eq!(segment_role(VERB_DEPENDENT), SegmentRole::Dependent);
        assert_eq!(segment_role(ADJ_DEPENDENT), SegmentRole::Dependent);
        assert_eq!(segment_role(NOUN), SegmentRole::Compound);
        assert_eq!(
            segment_role("名詞,固有名詞,人名,一般,*,*"),
            SegmentRole::Compound
        );
        assert_eq!(segment_role("名詞,数詞,*,*,*,*"), SegmentRole::Compound);
        assert_eq!(segment_role(PREFIX), SegmentRole::Prefix);
        for pos in [
            VERB,
            "形容詞,一般,*,*,形容詞,終止形-一般",
            "形状詞,一般,*,*,*,*",
            "副詞,*,*,*,*,*",
            "連体詞,*,*,*,*,*",
            "接続詞,*,*,*,*,*",
            "感動詞,一般,*,*,*,*",
            "代名詞,*,*,*,*,*",
            "記号,一般,*,*,*,*",
        ] {
            assert_eq!(segment_role(pos), SegmentRole::Start, "{pos}");
        }
    }

    #[test]
    fn unknown_pos_starts_a_clause() {
        assert_eq!(segment_role(""), SegmentRole::Start);
        assert_eq!(segment_role("*"), SegmentRole::Start);
        assert_eq!(segment_role("謎の品詞,一般"), SegmentRole::Start);
        // 品詞2 がなくても名詞は名詞
        assert_eq!(segment_role("名詞"), SegmentRole::Compound);
        // 非自立可能は名詞・動詞・形容詞のときだけ
        assert_eq!(segment_role("副詞,非自立可能"), SegmentRole::Start);
    }

    #[test]
    fn whitespace_never_starts_a_clause_by_itself() {
        assert_eq!(segment_role("空白,*,*,*,*,*"), SegmentRole::Attach);
    }

    #[test]
    fn compound_nouns_are_one_clause() {
        // リポジトリ名
        assert_eq!(clauses(&[NOUN, NOUN]), [2]);
        // 三つ以上
        assert_eq!(clauses(&[NOUN, NOUN, NOUN]), [3]);
        // 田中さん（接尾辞）+ 名詞
        assert_eq!(clauses(&[NOUN, SUFFIX_NOUN, NOUN]), [3]);
    }

    #[test]
    fn particles_end_a_noun_run() {
        // ユーザー辞書に | 単語を | 登録する
        assert_eq!(
            clauses(&[NOUN, NOUN, PARTICLE, NOUN, PARTICLE, NOUN, VERB_DEPENDENT]),
            [3, 2, 2]
        );
    }

    #[test]
    fn verb_with_dependents_is_one_clause() {
        // 初めて | 知った
        assert_eq!(clauses(&["副詞,*,*,*,*,*", VERB, AUX]), [1, 2]);
        // 書いている
        assert_eq!(
            clauses(&[VERB, "助詞,接続助詞,*,*,*,*", VERB_DEPENDENT]),
            [3]
        );
        // 変換する
        assert_eq!(clauses(&[NOUN, VERB_DEPENDENT]), [2]);
        // 食べたいこと
        assert_eq!(clauses(&[VERB, AUX, NOUN_DEPENDENT]), [3]);
    }

    const PARTICLE_CONNECTIVE: &str = "助詞,接続助詞,*,*,*,*";
    const ADVERB: &str = "副詞,*,*,*,*,*";

    #[test]
    fn closes_phrase_by_particle_kind() {
        assert!(closes_phrase(PARTICLE));
        assert!(closes_phrase("助詞,係助詞,*,*,*,*"));
        assert!(closes_phrase(ADVERB));
        assert!(closes_phrase("連体詞,*,*,*,*,*"));
        assert!(!closes_phrase(PARTICLE_CONNECTIVE));
        assert!(!closes_phrase(VERB));
        assert!(!closes_phrase(NOUN));
        assert!(!closes_phrase(AUX));
        assert!(!closes_phrase(""));
    }

    #[test]
    fn dependent_verb_after_a_case_particle_is_a_main_verb() {
        // 髪を | 切る（Sudachi は「切る」「行く」「ある」を 動詞,非自立可能 にしている）
        assert_eq!(clauses(&[NOUN, PARTICLE, VERB_DEPENDENT]), [2, 1]);
        // 公園に | 行く / 感心が | ある
        assert_eq!(clauses(&[NOUN, PARTICLE, VERB_DEPENDENT, AUX]), [2, 2]);
        // 天気が | いい
        assert_eq!(clauses(&[NOUN, PARTICLE, ADJ_DEPENDENT]), [2, 1]);
        // すぐ | 行く
        assert_eq!(clauses(&[ADVERB, VERB_DEPENDENT]), [1, 1]);
        // 人の | こと
        assert_eq!(clauses(&[NOUN, PARTICLE, NOUN_DEPENDENT]), [2, 1]);
    }

    #[test]
    fn dependent_after_a_predicate_or_connective_particle_stays_attached() {
        // 食べて いる（接続助詞）
        assert_eq!(clauses(&[VERB, PARTICLE_CONNECTIVE, VERB_DEPENDENT]), [3]);
        // 食べ続ける（動詞のあと）、食べたい こと（助動詞のあと）
        assert_eq!(clauses(&[VERB, VERB_DEPENDENT]), [2]);
        assert_eq!(clauses(&[VERB, AUX, VERB_DEPENDENT]), [3]);
        // 変換 する（名詞のあと。サ変）
        assert_eq!(clauses(&[NOUN, VERB_DEPENDENT]), [2]);
        // 助詞のあとに付いた補助的な用言は、その文節の中で続く（書いて いる いる）
        assert_eq!(
            clauses(&[VERB, PARTICLE_CONNECTIVE, VERB_DEPENDENT, VERB_DEPENDENT]),
            [4]
        );
    }

    #[test]
    fn verb_then_noun_starts_a_new_clause() {
        // 走る | 人
        assert_eq!(clauses(&[VERB, NOUN]), [1, 1]);
    }

    #[test]
    fn prefix_attaches_the_next_word() {
        // 御 + 名詞 / 御 + 形状詞
        assert_eq!(clauses(&[PREFIX, NOUN]), [2]);
        assert_eq!(clauses(&[PREFIX, "形状詞,一般,*,*,*,*"]), [2]);
        // 前の名詞とは別の文節（接頭辞は新しい文節を始める）
        assert_eq!(clauses(&[NOUN, PREFIX, NOUN]), [1, 2]);
        assert_eq!(clauses(&[PARTICLE, PREFIX, NOUN]), [1, 2]);
    }

    #[test]
    fn first_word_starts_even_when_it_is_an_attached_word() {
        // をしよう
        assert_eq!(clauses(&[PARTICLE, VERB_DEPENDENT]), [2]);
        assert_eq!(clauses(&[AUX]), [1]);
        assert_eq!(clauses(&[]), Vec::<usize>::new());
    }

    #[test]
    fn punctuation_attaches_to_the_left() {
        assert_eq!(clauses(&[NOUN, PARTICLE, PUNCT]), [3]);
    }
}
