//! 数字を含む町名（`北1条西`、`新野9線`）の読みを作る（ADR 0030 §3.1）。
//!
//! 数詞の読みは、辞書（Sudachi）の行にはあるが、コードの規則表はなかったので、ここで持つ（町名の数字は 1〜数百。
//! 最大は 4 桁まで）。ユーザー決定（2026-11-10）: 読みのゆれ（4 = よん/し、7 = なな/しち、9 = きゅう/く、
//! 促音 = いっちょうめ/いちちょうめ）は全部入れる。
//!
//! すべて純粋な関数。読みの文字は `number_readings` がひらがな、`expand_digit_town` がカタカナ（ABR の読みに合わせる）。

use ikari_dict_format::reading::normalize_key;

use crate::town::match_key;

/// 数詞の読みを作れる数の上限（この数まで。町名の数字は 1〜数百）
const MAX_NUMBER: u32 = 9999;
/// 促音にならない数詞の読み（0 の位や、単位を持たない桁の読み）。ones の 0 番目は使わない
const ONES: [&[&str]; 10] = [
    &[],
    &["いち"],
    &["に"],
    &["さん"],
    &["よん", "し"],
    &["ご"],
    &["ろく"],
    &["なな", "しち"],
    &["はち"],
    &["きゅう", "く"],
];
/// 百の位（`ひゃく` の連濁・促音。ゆれは両方入れる）。添字は桁の値（0 は使わない）
const HUNDREDS: [&[&str]; 10] = [
    &[],
    &["ひゃく"],
    &["にひゃく"],
    &["さんびゃく"],
    &["よんひゃく"],
    &["ごひゃく"],
    &["ろっぴゃく", "ろくひゃく"],
    &["ななひゃく", "しちひゃく"],
    &["はっぴゃく", "はちひゃく"],
    &["きゅうひゃく"],
];
/// 千の位。1 は `せん` と `いっせん`
const THOUSANDS: [&[&str]; 10] = [
    &[],
    &["せん", "いっせん"],
    &["にせん"],
    &["さんぜん"],
    &["よんせん"],
    &["ごせん"],
    &["ろくせん"],
    &["ななせん", "しちせん"],
    &["はっせん", "はちせん"],
    &["きゅうせん"],
];
const TEN: &str = "じゅう";
/// 次の読みが か・さ・た・ぱ行のとき、最後の 1 文字を促音（っ）にできる数詞の終わり（いち、ろく、はち、じゅう、ひゃく）。
/// `しち`（7）、`く`（9）は促音にしない
const SOKUON_ENDINGS: [&str; 7] = [
    "いち",
    "ろく",
    "はち",
    "じゅう",
    "ひゃく",
    "びゃく",
    "ぴゃく",
];
const SMALL_TSU: char = 'っ';
/// 数詞のあとの読みの頭が、促音を呼ぶ音か（か行・さ行・た行・ぱ行。`ジョウ`（じ）、`バン`（ば）、`ゴウ`（ご）は呼ばない）
const SOKUON_TRIGGERS: &str = "かきくけこさしすせそたちつてとぱぴぷぺぽ";
const KATAKANA_OFFSET: u32 = 0x60;
const HIRAGANA_FIRST: char = '\u{3041}';
const HIRAGANA_LAST: char = '\u{3096}';

/// 数詞に続く助数詞（表記, 読み）。読みに数字がない町名（ABR が `ニセンヨンゴウ` と書いたもの）から、数詞の位置を見つけるために使う
const COUNTERS: [(&str, &str); 10] = [
    ("丁目", "ちょうめ"),
    ("条", "じょう"),
    ("線", "せん"),
    ("番", "ばん"),
    ("号", "ごう"),
    ("区", "く"),
    ("街", "がい"),
    ("地", "ち"),
    ("字", "あざ"),
    ("基", "き"),
];

/// 数 `n`（1〜9999）の読み（ひらがな）。ゆれはすべて入れる（最初が標準の読み）。
/// `next` は数詞のあとに続く読みの最初の文字（ひらがなかカタカナ。なければ `None`）。か・さ・た・ぱ行のときは
/// 促音の形（`いっちょうめ`、`はっせん`）を先に、促音にしない形（`いちちょうめ`）をあとに入れる。
/// 0、10000 以上は空。
pub fn number_readings(n: u32, next: Option<char>) -> Vec<String> {
    if n == 0 || n > MAX_NUMBER {
        return Vec::new();
    }
    let digit = |unit: u32| ((n / unit) % 10) as usize;
    // 桁ごとの読みの候補（上の桁から）
    let mut parts: Vec<Vec<String>> = Vec::new();
    let to_owned = |list: &[&str]| list.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
    if digit(1000) > 0 {
        parts.push(to_owned(THOUSANDS[digit(1000)]));
    }
    if digit(100) > 0 {
        parts.push(to_owned(HUNDREDS[digit(100)]));
    }
    match digit(10) {
        0 => {}
        1 => parts.push(vec![TEN.to_string()]),
        d => parts.push(
            ONES[d]
                .iter()
                .map(|s| format!("{s}{TEN}"))
                .collect::<Vec<_>>(),
        ),
    }
    if digit(1) > 0 {
        parts.push(to_owned(ONES[digit(1)]));
    }
    // 桁ごとの候補の直積
    let mut readings = vec![String::new()];
    for part in &parts {
        readings = readings
            .iter()
            .flat_map(|head| part.iter().map(move |p| format!("{head}{p}")))
            .collect();
    }
    let sokuon_follows = next
        .map(|c| normalize_key(&c.to_string()))
        .and_then(|s| s.chars().next())
        .is_some_and(|c| SOKUON_TRIGGERS.contains(c));
    if sokuon_follows {
        // 促音の形を先に、元の形をあとに
        let mut out = Vec::with_capacity(readings.len() * 2);
        for r in &readings {
            if SOKUON_ENDINGS.iter().any(|e| r.ends_with(e)) {
                let mut chars: Vec<char> = r.chars().collect();
                if let Some(last) = chars.last_mut() {
                    *last = SMALL_TSU;
                }
                out.push(chars.into_iter().collect());
            }
        }
        out.extend(readings);
        dedup_keep_order(&mut out);
        return out;
    }
    readings
}

fn dedup_keep_order(v: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    v.retain(|s| seen.insert(s.clone()));
}

/// ひらがなをカタカナにする（それ以外はそのまま）
fn to_katakana(s: &str) -> String {
    s.chars()
        .map(|c| {
            if (HIRAGANA_FIRST..=HIRAGANA_LAST).contains(&c) {
                char::from_u32(c as u32 + KATAKANA_OFFSET).unwrap_or(c)
            } else {
                c
            }
        })
        .collect()
}

/// 文字列の中の数字の連なり（半角）の (開始の文字位置, 終わりの文字位置, 数)。0 で始まる 2 桁以上、5 桁以上の数は `None`（全体が使えない）
fn digit_runs(s: &str) -> Option<Vec<(usize, usize, u32)>> {
    let chars: Vec<char> = s.chars().collect();
    let mut runs = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            let text: String = chars[start..i].iter().collect();
            if (text.len() > 1 && text.starts_with('0')) || text.len() > 4 {
                return None;
            }
            runs.push((start, i, text.parse().ok()?));
        } else {
            i += 1;
        }
    }
    Some(runs)
}

/// 数字を含む町名の読みの組を作る。`surface` は半角の数字の表記、`reading` は半角の数字、カタカナの ABR の読み（どちらも
/// 数字以外は `convert` を通したもの）。戻り値の先頭から、**数字を読みにした形**（`キタイチジョウニシ`。ゆれの全部。カタカナ）、
/// **数字を含む読み**（`キタ1ジョウニシ`）の順。作れなければ `None`（数字の連なりが合わない、助数詞が分からない、数が大きいなど）。
///
/// - 読みにも同じ数字がある（`キタ1ジョウニシ`。約 75%）: 数字を読みに置き換える。後ろの文字が促音を呼ぶかは読みの続きで決める。
/// - 読みに数字がない（`ニシカグラニセンヨンゴウ`。ABR が数詞を読みにしたもの）: 表記の `2線4号` の数と助数詞から、読みの中の
///   `ニセン` `ヨンゴウ` の位置を探し（小書き・大書きの違いは無視。`ジユウイツセン` も見つかる）、数詞の部分を読みに替える・数字に替える。
pub fn expand_digit_town(surface: &str, reading: &str) -> Option<Vec<String>> {
    let surface_runs = digit_runs(surface)?;
    if surface_runs.is_empty() {
        return None;
    }
    let reading_runs = digit_runs(reading)?;
    let spans: Vec<Span> = if reading_runs.is_empty() {
        align_kana_numerals(surface, &surface_runs, reading)?
    } else {
        let a: Vec<u32> = surface_runs.iter().map(|r| r.2).collect();
        let b: Vec<u32> = reading_runs.iter().map(|r| r.2).collect();
        if a != b {
            return None;
        }
        let chars: Vec<char> = reading.chars().collect();
        reading_runs
            .iter()
            .map(|&(start, end, n)| Span {
                start,
                end,
                n,
                next: chars.get(end).copied(),
            })
            .collect()
    };
    // 数字を読みにした形の直積
    let chars: Vec<char> = reading.chars().collect();
    let mut kana_forms = vec![String::new()];
    let mut digit_form = String::new();
    let mut cursor = 0;
    for span in &spans {
        let between: String = chars[cursor..span.start].iter().collect();
        let variants = number_readings(span.n, span.next);
        if variants.is_empty() {
            return None;
        }
        let between_ref = between.as_str();
        kana_forms = kana_forms
            .iter()
            .flat_map(|head| {
                variants
                    .iter()
                    .map(move |v| format!("{head}{between_ref}{}", to_katakana(v)))
            })
            .collect();
        digit_form.push_str(&between);
        digit_form.push_str(&span.n.to_string());
        cursor = span.end;
    }
    let tail: String = chars[cursor..].iter().collect();
    for form in &mut kana_forms {
        form.push_str(&tail);
    }
    digit_form.push_str(&tail);
    dedup_keep_order(&mut kana_forms);
    kana_forms.push(digit_form);
    Some(kana_forms)
}

/// 読みの中の数詞の位置（文字単位の [start, end)）、数、そのあとの読みの最初の文字
struct Span {
    start: usize,
    end: usize,
    n: u32,
    next: Option<char>,
}

/// 読みに数字がないとき、表記の数字と助数詞から、読みの中の数詞の位置を探す。見つからなければ `None`。
fn align_kana_numerals(
    surface: &str,
    runs: &[(usize, usize, u32)],
    reading: &str,
) -> Option<Vec<Span>> {
    let surface_chars: Vec<char> = surface.chars().collect();
    let reading_chars: Vec<char> = reading.chars().collect();
    let folded: Vec<char> = match_key(reading).chars().collect();
    if folded.len() != reading_chars.len() {
        return None;
    }
    let mut spans = Vec::new();
    let mut cursor = 0;
    for &(_, end, n) in runs {
        let after: String = surface_chars[end..].iter().collect();
        let (_, counter) = COUNTERS.iter().find(|(s, _)| after.starts_with(s))?;
        let counter_folded: Vec<char> = match_key(counter).chars().collect();
        let next = counter.chars().next();
        // 最も早く見つかる (数詞 + 助数詞)
        let mut best: Option<(usize, usize)> = None;
        for variant in number_readings(n, next) {
            let pattern: Vec<char> = match_key(&variant)
                .chars()
                .chain(counter_folded.iter().copied())
                .collect();
            let num_len = variant.chars().count();
            let found = (cursor..=folded.len().saturating_sub(pattern.len()))
                .find(|&i| folded[i..].starts_with(&pattern));
            if let Some(i) = found
                && best.is_none_or(|(s, l)| i < s || (i == s && num_len > l))
            {
                best = Some((i, num_len));
            }
        }
        let (start, num_len) = best?;
        spans.push(Span {
            start,
            end: start + num_len,
            n,
            next,
        });
        cursor = start + num_len;
    }
    Some(spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(n: u32, next: Option<char>) -> Vec<String> {
        number_readings(n, next)
    }

    #[test]
    fn small_numbers_have_all_variants() {
        assert_eq!(r(1, None), ["いち"]);
        assert_eq!(r(2, None), ["に"]);
        assert_eq!(r(3, None), ["さん"]);
        assert_eq!(r(4, None), ["よん", "し"]);
        assert_eq!(r(5, None), ["ご"]);
        assert_eq!(r(6, None), ["ろく"]);
        assert_eq!(r(7, None), ["なな", "しち"]);
        assert_eq!(r(8, None), ["はち"]);
        assert_eq!(r(9, None), ["きゅう", "く"]);
        assert_eq!(r(10, None), ["じゅう"]);
    }

    #[test]
    fn tens_and_hundreds() {
        assert_eq!(r(11, None), ["じゅういち"]);
        assert_eq!(r(14, None), ["じゅうよん", "じゅうし"]);
        assert_eq!(r(20, None), ["にじゅう"]);
        assert_eq!(r(40, None), ["よんじゅう", "しじゅう"]);
        assert_eq!(r(99, None).len(), 4);
        assert_eq!(r(100, None), ["ひゃく"]);
        assert_eq!(r(240, None), ["にひゃくよんじゅう", "にひゃくしじゅう"]);
        assert_eq!(r(300, None), ["さんびゃく"]);
        assert_eq!(r(600, None), ["ろっぴゃく", "ろくひゃく"]);
        assert_eq!(r(1000, None), ["せん", "いっせん"]);
        assert_eq!(r(3000, None), ["さんぜん"]);
        assert_eq!(r(1234, None)[0], "せんにひゃくさんじゅうよん");
    }

    #[test]
    fn out_of_range_is_empty() {
        assert!(r(0, None).is_empty());
        assert!(r(10000, None).is_empty());
    }

    #[test]
    fn sokuon_before_k_s_t_p_rows() {
        // 促音の形が先、そのあとに促音にしない形
        assert_eq!(r(1, Some('チ')), ["いっ", "いち"]);
        assert_eq!(r(1, Some('せ')), ["いっ", "いち"]);
        assert_eq!(r(8, Some('チ')), ["はっ", "はち"]);
        assert_eq!(r(6, Some('セ')), ["ろっ", "ろく"]);
        assert_eq!(r(10, Some('セ')), ["じゅっ", "じゅう"]);
        assert_eq!(r(11, Some('チ')), ["じゅういっ", "じゅういち"]);
        assert_eq!(r(100, Some('か')), ["ひゃっ", "ひゃく"]);
        assert_eq!(r(300, Some('か')), ["さんびゃっ", "さんびゃく"]);
        assert_eq!(
            r(600, Some('か')),
            ["ろっぴゃっ", "ろくひゃっ", "ろっぴゃく", "ろくひゃく"]
        );
        // 7、9 は促音にしないが、ゆれは残る
        assert_eq!(r(7, Some('セ')), ["なな", "しち"]);
        assert_eq!(r(9, Some('チ')), ["きゅう", "く"]);
        // 促音を呼ばない音（じ、ば、ご、あ）と、続きがないとき
        assert_eq!(r(1, Some('ジ')), ["いち"]);
        assert_eq!(r(1, Some('バ')), ["いち"]);
        assert_eq!(r(1, Some('ゴ')), ["いち"]);
        assert_eq!(r(1, Some('ア')), ["いち"]);
        assert_eq!(r(1, None), ["いち"]);
        // 促音の対象でない終わり（に、さん）はそのまま
        assert_eq!(r(2, Some('チ')), ["に"]);
        assert_eq!(r(3, Some('チ')), ["さん"]);
    }

    #[test]
    fn expands_a_town_whose_reading_has_digits() {
        // 北1条西 → きたいちじょうにし（じ は促音を呼ばない）と キタ1ジョウニシ
        assert_eq!(
            expand_digit_town("北1条西", "キタ1ジョウニシ"),
            Some(vec![
                "キタイチジョウニシ".to_string(),
                "キタ1ジョウニシ".to_string()
            ])
        );
        // 10 条（じゅう）
        assert_eq!(
            expand_digit_town("北10条西", "キタ10ジョウニシ"),
            Some(vec![
                "キタジュウジョウニシ".to_string(),
                "キタ10ジョウニシ".to_string()
            ])
        );
        // 4 条（よん/し）
        assert_eq!(
            expand_digit_town("南4条", "ミナミ4ジョウ"),
            Some(vec![
                "ミナミヨンジョウ".to_string(),
                "ミナミシジョウ".to_string(),
                "ミナミ4ジョウ".to_string()
            ])
        );
        // 1線（せん は促音を呼ぶ）、8丁目（ちょう）
        assert_eq!(
            expand_digit_town("東1線", "ヒガシ1セン"),
            Some(vec![
                "ヒガシイッセン".to_string(),
                "ヒガシイチセン".to_string(),
                "ヒガシ1セン".to_string()
            ])
        );
        let chome = expand_digit_town("大樹寺8丁目", "ダイジュウジ8チョウメ").unwrap();
        assert_eq!(chome[0], "ダイジュウジハッチョウメ");
        assert_eq!(chome[1], "ダイジュウジハチチョウメ");
        assert_eq!(chome.last().unwrap(), "ダイジュウジ8チョウメ");
        // 240 線
        let big = expand_digit_town("新240線", "シン240セン").unwrap();
        assert_eq!(big[0], "シンニヒャクヨンジュッセン");
        assert!(big.contains(&"シンニヒャクヨンジュウセン".to_string()));
        assert_eq!(big.last().unwrap(), "シン240セン");
    }

    #[test]
    fn expands_a_town_with_two_numbers() {
        let forms =
            expand_digit_town("桔梗が丘1番町1街区", "キキョウガオカ1バンチョウ1ガイク").unwrap();
        assert_eq!(forms[0], "キキョウガオカイチバンチョウイチガイク");
        assert_eq!(forms.last().unwrap(), "キキョウガオカ1バンチョウ1ガイク");
    }

    #[test]
    fn aligns_a_reading_without_digits() {
        // ABR が数詞を読みにしたもの。数詞の位置は表記の助数詞から探す
        let forms = expand_digit_town("西神楽2線4号", "ニシカグラニセンヨンゴウ").unwrap();
        assert_eq!(forms[0], "ニシカグラニセンヨンゴウ");
        assert!(forms.contains(&"ニシカグラニセンシゴウ".to_string()));
        assert_eq!(forms.last().unwrap(), "ニシカグラ2セン4ゴウ");
        // 小書きにしていない読み（ジユウイツセン）も見つかり、読みの形は直る
        let forms = expand_digit_town("駒牧11線", "コママキジユウイツセン").unwrap();
        assert_eq!(forms[0], "コママキジュウイッセン");
        assert_eq!(forms.last().unwrap(), "コママキ11セン");
    }

    #[test]
    fn rejects_what_cannot_be_aligned() {
        // 読みに数詞がない、数字の連なりが合わない、助数詞が分からない、数が大きい
        assert_eq!(expand_digit_town("北1条西", "キタジョウニシ"), None);
        assert_eq!(expand_digit_town("北1条西", "キタ2ジョウニシ"), None);
        assert_eq!(expand_digit_town("布部1", "ヌノベイチ"), None);
        assert_eq!(expand_digit_town("新野9線", "ニイノセン"), None);
        assert_eq!(expand_digit_town("北10000条", "キタ10000ジョウ"), None);
        assert_eq!(expand_digit_town("北0条", "キタ0ジョウ"), None);
        assert_eq!(expand_digit_town("北01条", "キタ01ジョウ"), None);
        assert_eq!(expand_digit_town("北条", "キタジョウ"), None);
    }
}
