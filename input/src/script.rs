//! 字種の変換（F6〜F10。ADR 0004 §3.3）。

use std::collections::HashMap;
use std::sync::OnceLock;

/// 変換先の字種。`repr(u32)` の値は `ikari_shell.h` の `IKARI_SCRIPT_*` と一致させる（契約）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Script {
    /// F6: ひらがな
    Hiragana = 0,
    /// F7: 全角カタカナ
    FullwidthKatakana = 1,
    /// F8: 半角カタカナ
    HalfwidthKatakana = 2,
    /// F9: 全角英数
    FullwidthAlnum = 3,
    /// F10: 半角英数
    HalfwidthAlnum = 4,
}

impl Script {
    /// FFI の番号から引く。範囲外は None。
    pub fn from_u32(value: u32) -> Option<Script> {
        match value {
            0 => Some(Script::Hiragana),
            1 => Some(Script::FullwidthKatakana),
            2 => Some(Script::HalfwidthKatakana),
            3 => Some(Script::FullwidthAlnum),
            4 => Some(Script::HalfwidthAlnum),
            _ => None,
        }
    }
}

const ROMAJI_TSV: &str = include_str!("../assets/romaji.tsv");
const TSV_COMMENT_PREFIX: char = '#';
const TSV_SEPARATOR: char = '\t';
/// 「入力<TAB>出力」の2列の行だけを逆引きに使う（3列目は促音の重ね打ち）
const TSV_PLAIN_COLUMNS: usize = 2;

const HIRAGANA_FIRST: u32 = 0x3041;
const HIRAGANA_LAST: u32 = 0x3096;
const KATAKANA_FIRST: u32 = 0x30A1;
const KATAKANA_LAST: u32 = 0x30F6;
const HIRAGANA_ITERATION: char = '\u{309D}';
const HIRAGANA_VOICED_ITERATION: char = '\u{309E}';
const KATAKANA_ITERATION: char = '\u{30FD}';
const KATAKANA_VOICED_ITERATION: char = '\u{30FE}';

const ASCII_FIRST: u32 = 0x21;
const ASCII_LAST: u32 = 0x7E;
const FULLWIDTH_OFFSET: u32 = 0xFEE0;
const ASCII_SPACE: char = ' ';
const IDEOGRAPHIC_SPACE: char = '\u{3000}';

const HALFWIDTH_VOICED_MARK: char = '\u{FF9E}';
const HALFWIDTH_SEMI_VOICED_MARK: char = '\u{FF9F}';

const SOKUON: char = 'っ';
/// 「っ」の次がローマ字にならないときの入力
const SOKUON_FALLBACK: &str = "xtu";
const SOKUON_BEFORE_CH: &str = "t";
const HATSUON: char = 'ん';
const HATSUON_ROMAJI: &str = "n";
/// 「ん」の次が母音・y・n のとき（「かんい」が「kani」にならないように）
const HATSUON_BEFORE_VOWEL: &str = "nn";
const VOWELS_AND_YN: [char; 7] = ['a', 'i', 'u', 'e', 'o', 'y', 'n'];
const VOWELS: [char; 5] = ['a', 'i', 'u', 'e', 'o'];
const CH_PREFIX: &str = "ch";
/// 小書き用の入力と、c 始まり（ch 以外）は逆引きで避ける
const AVOIDED_PREFIXES: [char; 3] = ['x', 'l', 'q'];
/// 複数の入力がある仮名のうち、ヘボン式に寄せるもの
const HEPBURN_OVERRIDES: [(&str, &str); 6] = [
    ("し", "shi"),
    ("ち", "chi"),
    ("つ", "tsu"),
    ("ふ", "fu"),
    ("じ", "ji"),
    ("を", "wo"),
];
/// 仮名の組（「きょ」など）の最大の長さ
const MAX_KANA_GROUP: usize = 2;

/// 全角カタカナ -> 半角カタカナ。濁点と半濁点は別の文字に分ける。
const HALFWIDTH_TABLE: &[(char, &str)] = &[
    ('ァ', "ｧ"),
    ('ア', "ｱ"),
    ('ィ', "ｨ"),
    ('イ', "ｲ"),
    ('ゥ', "ｩ"),
    ('ウ', "ｳ"),
    ('ェ', "ｪ"),
    ('エ', "ｴ"),
    ('ォ', "ｫ"),
    ('オ', "ｵ"),
    ('カ', "ｶ"),
    ('ガ', "ｶﾞ"),
    ('キ', "ｷ"),
    ('ギ', "ｷﾞ"),
    ('ク', "ｸ"),
    ('グ', "ｸﾞ"),
    ('ケ', "ｹ"),
    ('ゲ', "ｹﾞ"),
    ('コ', "ｺ"),
    ('ゴ', "ｺﾞ"),
    ('サ', "ｻ"),
    ('ザ', "ｻﾞ"),
    ('シ', "ｼ"),
    ('ジ', "ｼﾞ"),
    ('ス', "ｽ"),
    ('ズ', "ｽﾞ"),
    ('セ', "ｾ"),
    ('ゼ', "ｾﾞ"),
    ('ソ', "ｿ"),
    ('ゾ', "ｿﾞ"),
    ('タ', "ﾀ"),
    ('ダ', "ﾀﾞ"),
    ('チ', "ﾁ"),
    ('ヂ', "ﾁﾞ"),
    ('ッ', "ｯ"),
    ('ツ', "ﾂ"),
    ('ヅ', "ﾂﾞ"),
    ('テ', "ﾃ"),
    ('デ', "ﾃﾞ"),
    ('ト', "ﾄ"),
    ('ド', "ﾄﾞ"),
    ('ナ', "ﾅ"),
    ('ニ', "ﾆ"),
    ('ヌ', "ﾇ"),
    ('ネ', "ﾈ"),
    ('ノ', "ﾉ"),
    ('ハ', "ﾊ"),
    ('バ', "ﾊﾞ"),
    ('パ', "ﾊﾟ"),
    ('ヒ', "ﾋ"),
    ('ビ', "ﾋﾞ"),
    ('ピ', "ﾋﾟ"),
    ('フ', "ﾌ"),
    ('ブ', "ﾌﾞ"),
    ('プ', "ﾌﾟ"),
    ('ヘ', "ﾍ"),
    ('ベ', "ﾍﾞ"),
    ('ペ', "ﾍﾟ"),
    ('ホ', "ﾎ"),
    ('ボ', "ﾎﾞ"),
    ('ポ', "ﾎﾟ"),
    ('マ', "ﾏ"),
    ('ミ', "ﾐ"),
    ('ム', "ﾑ"),
    ('メ', "ﾒ"),
    ('モ', "ﾓ"),
    ('ャ', "ｬ"),
    ('ヤ', "ﾔ"),
    ('ュ', "ｭ"),
    ('ユ', "ﾕ"),
    ('ョ', "ｮ"),
    ('ヨ', "ﾖ"),
    ('ラ', "ﾗ"),
    ('リ', "ﾘ"),
    ('ル', "ﾙ"),
    ('レ', "ﾚ"),
    ('ロ', "ﾛ"),
    ('ワ', "ﾜ"),
    ('ヲ', "ｦ"),
    ('ン', "ﾝ"),
    ('ヴ', "ｳﾞ"),
    ('ー', "ｰ"),
    ('。', "｡"),
    ('、', "､"),
    ('「', "｢"),
    ('」', "｣"),
    ('・', "･"),
    ('゛', "ﾞ"),
    ('゜', "ﾟ"),
];

fn shift_code(c: char, from: u32, to: u32) -> char {
    char::from_u32(c as u32 - from + to).unwrap_or(c)
}

fn hiragana_to_katakana_char(c: char) -> char {
    match c {
        HIRAGANA_ITERATION => KATAKANA_ITERATION,
        HIRAGANA_VOICED_ITERATION => KATAKANA_VOICED_ITERATION,
        c if (HIRAGANA_FIRST..=HIRAGANA_LAST).contains(&(c as u32)) => {
            shift_code(c, HIRAGANA_FIRST, KATAKANA_FIRST)
        }
        c => c,
    }
}

fn katakana_to_hiragana_char(c: char) -> char {
    match c {
        KATAKANA_ITERATION => HIRAGANA_ITERATION,
        KATAKANA_VOICED_ITERATION => HIRAGANA_VOICED_ITERATION,
        c if (KATAKANA_FIRST..=KATAKANA_LAST).contains(&(c as u32)) => {
            shift_code(c, KATAKANA_FIRST, HIRAGANA_FIRST)
        }
        c => c,
    }
}

fn halfwidth_to_full_map() -> &'static HashMap<&'static str, char> {
    static MAP: OnceLock<HashMap<&'static str, char>> = OnceLock::new();
    MAP.get_or_init(|| {
        HALFWIDTH_TABLE
            .iter()
            .map(|&(full, half)| (half, full))
            .collect()
    })
}

/// 半角カタカナを全角カタカナにする（濁点と半濁点は直前の文字と合成する）。それ以外はそのまま。
fn halfwidth_to_fullwidth_katakana(s: &str) -> String {
    let map = halfwidth_to_full_map();
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if let Some(&mark) = chars.get(i + 1)
            && (mark == HALFWIDTH_VOICED_MARK || mark == HALFWIDTH_SEMI_VOICED_MARK)
        {
            let pair: String = [c, mark].iter().collect();
            if let Some(&full) = map.get(pair.as_str()) {
                out.push(full);
                i += 2;
                continue;
            }
        }
        let single = c.to_string();
        out.push(map.get(single.as_str()).copied().unwrap_or(c));
        i += 1;
    }
    out
}

/// ひらがなに揃える（全角・半角のカタカナもひらがなにする）。
fn normalize_to_hiragana(s: &str) -> String {
    halfwidth_to_fullwidth_katakana(s)
        .chars()
        .map(katakana_to_hiragana_char)
        .collect()
}

/// ひらがな（ぁ..=ゖ、ゝ、ゞ）を全角カタカナにする。それ以外の文字（長音記号、ASCII、半角カナを含む）はそのまま。
/// 半角カタカナを全角にはしない（辞書の索引のキーのように、字種をそろえ済みの読みの上で使う）。
pub fn hiragana_to_katakana(s: &str) -> String {
    s.chars().map(hiragana_to_katakana_char).collect()
}

fn to_fullwidth_katakana(s: &str) -> String {
    hiragana_to_katakana(&halfwidth_to_fullwidth_katakana(s))
}

fn to_halfwidth_katakana(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in to_fullwidth_katakana(s).chars() {
        match HALFWIDTH_TABLE.iter().find(|&&(full, _)| full == c) {
            Some(&(_, half)) => out.push_str(half),
            None => out.push(c),
        }
    }
    out
}

fn ascii_to_fullwidth(c: char) -> char {
    match c {
        ASCII_SPACE => IDEOGRAPHIC_SPACE,
        c if (ASCII_FIRST..=ASCII_LAST).contains(&(c as u32)) => {
            char::from_u32(c as u32 + FULLWIDTH_OFFSET).unwrap_or(c)
        }
        c => c,
    }
}

fn fullwidth_to_ascii(c: char) -> char {
    match c {
        IDEOGRAPHIC_SPACE => ASCII_SPACE,
        c if (ASCII_FIRST + FULLWIDTH_OFFSET..=ASCII_LAST + FULLWIDTH_OFFSET)
            .contains(&(c as u32)) =>
        {
            char::from_u32(c as u32 - FULLWIDTH_OFFSET).unwrap_or(c)
        }
        c => c,
    }
}

/// 小さいほど逆引きで選ばれる。避けたい入力、短さ、辞書順の順で決める。
fn input_rank(input: &str) -> (bool, usize, String) {
    let avoid = input.starts_with(AVOIDED_PREFIXES)
        || (input.starts_with('c') && !input.starts_with(CH_PREFIX));
    (avoid, input.len(), input.to_owned())
}

/// 仮名 -> ローマ字（ヘボン式に寄せる）。同梱のローマ字表から作る。
fn reverse_romaji_map() -> &'static HashMap<String, String> {
    static MAP: OnceLock<HashMap<String, String>> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut best: HashMap<String, String> = HashMap::new();
        for line in ROMAJI_TSV.lines() {
            if line.is_empty() || line.starts_with(TSV_COMMENT_PREFIX) {
                continue;
            }
            let cols: Vec<&str> = line.split(TSV_SEPARATOR).collect();
            if cols.len() != TSV_PLAIN_COLUMNS || cols[0].is_empty() || cols[1].is_empty() {
                continue;
            }
            let (input, output) = (cols[0], cols[1]);
            let better = best
                .get(output)
                .is_none_or(|old| input_rank(input) < input_rank(old));
            if better {
                best.insert(output.to_owned(), input.to_owned());
            }
        }
        for (kana, romaji) in HEPBURN_OVERRIDES {
            best.insert((*kana).to_owned(), (*romaji).to_owned());
        }
        best
    })
}

/// `kana[at..]` の先頭の仮名の組（最長一致）のローマ字と、使った文字数。
fn romaji_at<'m>(
    map: &'m HashMap<String, String>,
    kana: &[char],
    at: usize,
) -> (Option<&'m str>, usize) {
    for len in (1..=MAX_KANA_GROUP).rev() {
        let Some(group) = kana.get(at..at + len) else {
            continue;
        };
        let key: String = group.iter().collect();
        if let Some(r) = map.get(&key) {
            return (Some(r.as_str()), len);
        }
    }
    (None, 0)
}

/// 読み（ひらがな）をローマ字にする。仮名でない文字はそのまま残す。
fn to_romaji(s: &str) -> String {
    let map = reverse_romaji_map();
    let kana: Vec<char> = normalize_to_hiragana(s)
        .chars()
        .map(fullwidth_to_ascii)
        .collect();
    let mut out = String::with_capacity(kana.len() * 2);
    let mut i = 0;
    while i < kana.len() {
        let c = kana[i];
        if c == SOKUON {
            let (next, used) = romaji_at(map, &kana, i + 1);
            match next.filter(|r| r.starts_with(|ch: char| ch.is_ascii_alphabetic())) {
                Some(rom) if rom.starts_with(CH_PREFIX) => {
                    out.push_str(SOKUON_BEFORE_CH);
                    out.push_str(rom);
                    i += 1 + used;
                }
                Some(rom) if !rom.starts_with(VOWELS) => {
                    out.push_str(&rom[..1]);
                    out.push_str(rom);
                    i += 1 + used;
                }
                _ => {
                    out.push_str(SOKUON_FALLBACK);
                    i += 1;
                }
            }
            continue;
        }
        if c == HATSUON {
            let (next, _) = romaji_at(map, &kana, i + 1);
            let before_vowel = next.is_some_and(|rom| rom.starts_with(VOWELS_AND_YN));
            out.push_str(if before_vowel {
                HATSUON_BEFORE_VOWEL
            } else {
                HATSUON_ROMAJI
            });
            i += 1;
            continue;
        }
        match romaji_at(map, &kana, i) {
            (Some(rom), used) => {
                out.push_str(rom);
                i += used;
            }
            (None, _) => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

/// `s`（ひらがなの読み。ASCII を含みうる）を字種 `to` に変換する。
/// 変換できない文字はそのまま残す。半角カタカナは濁点を分ける（が → ｶﾞ）。
/// 英数への変換は、仮名をローマ字に戻してから全角・半角にする（きょう → kyou、ｋｙｏｕ）。
pub fn convert_script(s: &str, to: Script) -> String {
    match to {
        Script::Hiragana => normalize_to_hiragana(s),
        Script::FullwidthKatakana => to_fullwidth_katakana(s),
        Script::HalfwidthKatakana => to_halfwidth_katakana(s),
        Script::FullwidthAlnum => to_romaji(s).chars().map(ascii_to_fullwidth).collect(),
        Script::HalfwidthAlnum => to_romaji(s),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_SCRIPTS: [Script; 5] = [
        Script::Hiragana,
        Script::FullwidthKatakana,
        Script::HalfwidthKatakana,
        Script::FullwidthAlnum,
        Script::HalfwidthAlnum,
    ];

    /// `ikari-core` の `Converter` が持っていた `to_katakana` と同じ式。置き換えの前後で結果が変わらないことの固定。
    fn reference_to_katakana(s: &str) -> String {
        s.chars()
            .map(|c| match c {
                '\u{3041}'..='\u{3096}' => char::from_u32(c as u32 + 0x60).unwrap_or(c),
                '\u{309D}' => '\u{30FD}',
                '\u{309E}' => '\u{30FE}',
                _ => c,
            })
            .collect()
    }

    #[test]
    fn hiragana_to_katakana_matches_the_converter_reference_over_every_char() {
        for code in 0..=0x10FFFFu32 {
            let Some(c) = char::from_u32(code) else {
                continue;
            };
            let s = c.to_string();
            assert_eq!(
                hiragana_to_katakana(&s),
                reference_to_katakana(&s),
                "U+{code:04X}"
            );
        }
        let sample = "ぁあぃいゔゕゖゝゞーカタカナabc012ｱｲﾞ、。ひらがな　";
        assert_eq!(hiragana_to_katakana(sample), reference_to_katakana(sample));
        assert_eq!(hiragana_to_katakana("ゝゞ"), "ヽヾ");
        assert_eq!(hiragana_to_katakana("ゔぁー"), "ヴァー");
    }

    #[test]
    fn from_u32_matches_the_repr() {
        for script in ALL_SCRIPTS {
            assert_eq!(Script::from_u32(script as u32), Some(script));
        }
        assert_eq!(Script::from_u32(5), None);
    }

    #[test]
    fn hiragana_accepts_katakana_in_either_width() {
        assert_eq!(convert_script("カタカナ", Script::Hiragana), "かたかな");
        assert_eq!(convert_script("ｶﾞｷﾞｸﾞ", Script::Hiragana), "がぎぐ");
        assert_eq!(convert_script("ひらがな", Script::Hiragana), "ひらがな");
        assert_eq!(convert_script("ー", Script::Hiragana), "ー");
    }

    #[test]
    fn fullwidth_katakana() {
        let to = Script::FullwidthKatakana;
        assert_eq!(convert_script("きょう", to), "キョウ");
        assert_eq!(convert_script("ゔぁいおりん", to), "ヴァイオリン");
        assert_eq!(convert_script("ｶﾞｯｺｳ", to), "ガッコウ");
        assert_eq!(convert_script("らーめん abc", to), "ラーメン abc");
        assert_eq!(convert_script("ゝゞ", to), "ヽヾ");
    }

    #[test]
    fn halfwidth_katakana_splits_voiced_marks() {
        let to = Script::HalfwidthKatakana;
        assert_eq!(convert_script("がっこう", to), "ｶﾞｯｺｳ");
        assert_eq!(convert_script("ぱぴぷぺぽ", to), "ﾊﾟﾋﾟﾌﾟﾍﾟﾎﾟ");
        assert_eq!(convert_script("ゔぁ", to), "ｳﾞｧ");
        assert_eq!(convert_script("らーめん、「あ」。・", to), "ﾗｰﾒﾝ､｢ｱ｣｡･");
        // 半角にできない文字（漢字、ヵ）はそのまま
        assert_eq!(convert_script("今日ヵ", to), "今日ヵ");
    }

    #[test]
    fn halfwidth_alnum_goes_back_to_romaji() {
        for (kana, romaji) in [
            ("きょう", "kyou"),
            ("しんぶん", "shinbun"),
            ("ちょっと", "chotto"),
            ("まっちゃ", "matcha"),
            ("かんい", "kanni"),
            ("ふじさん", "fujisan"),
            ("つくえ", "tsukue"),
            ("がっこう", "gakkou"),
            ("らーめん", "ra-men"),
            ("こんにちは、", "konnnichiha,"),
            ("ふぁいる", "fairu"),
            ("かくにん", "kakunin"),
        ] {
            assert_eq!(
                convert_script(kana, Script::HalfwidthAlnum),
                romaji,
                "{kana}"
            );
        }
    }

    /// 戻したローマ字を同梱の表で打ち直すと、元の仮名になる
    #[test]
    fn romaji_round_trips_through_the_builtin_table() {
        let table = crate::RomajiTable::builtin();
        for kana in [
            "きょうはいいてんきですね",
            "ちょっとまってください",
            "こんにちは、せかい。",
            "しゃしんをとる",
            "ぎゅうにゅう",
            "あんない",
            "ふぁいる",
            "てぃーぱーてぃー",
            "ゔぁいおりん",
            "がっこうへいく",
            "ぜんぶ",
        ] {
            let romaji = convert_script(kana, Script::HalfwidthAlnum);
            assert_eq!(
                crate::romaji_to_kana(&table, &romaji).kana,
                kana,
                "{romaji}"
            );
        }
    }

    #[test]
    fn alnum_keeps_ascii_and_converts_widths() {
        assert_eq!(convert_script("API", Script::HalfwidthAlnum), "API");
        assert_eq!(convert_script("ＡＰＩ１", Script::HalfwidthAlnum), "API1");
        assert_eq!(
            convert_script("API 1", Script::FullwidthAlnum),
            "ＡＰＩ\u{3000}１"
        );
        assert_eq!(convert_script("きょう", Script::FullwidthAlnum), "ｋｙｏｕ");
        // 仮名でも ASCII でもない文字は残す
        assert_eq!(convert_script("今日", Script::HalfwidthAlnum), "今日");
    }

    #[test]
    fn sokuon_and_hatsuon_edges() {
        let to = Script::HalfwidthAlnum;
        assert_eq!(convert_script("っ", to), "xtu");
        assert_eq!(convert_script("あっ", to), "axtu");
        assert_eq!(convert_script("ん", to), "n");
        assert_eq!(convert_script("んあ", to), "nna");
        assert_eq!(convert_script("", Script::FullwidthAlnum), "");
    }

    #[test]
    fn odd_input_never_panics() {
        for to in ALL_SCRIPTS {
            assert_eq!(convert_script("", to), "");
            let _ = convert_script("\u{0}あ\u{FF9E}ﾞ𠮷😀っんー", to);
        }
    }
}
