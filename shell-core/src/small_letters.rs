//! 上付き・下付きの小さい文字の候補（`:m2:` → m²、`:m_2:` → m₂。Mozc の small_letter_rewriter 相当。#264）。
//! 今の `SmallLetters.swift`。純粋。ショートコードの名前（`a-z 0-9 _ + -`）から、閉じの `:` のときに足す候補を作る。
//! 絵文字の表に同じ名前があれば、そちらが先（呼び出し側 `ShortcodeMachine` が決める）。

/// 名前に対する候補（「そのままの入力」は含まない）。上付き・下付きの文字列、単位の 1 文字の記号の順。なければ空。
pub fn candidates(name: &str) -> Vec<String> {
    let mut result: Vec<String> = Vec::new();
    if let Some(scripted) = scripted(name) {
        result.push(scripted);
    }
    if let Some(unit) = unit(name)
        && !result.iter().any(|existing| existing == unit)
    {
        result.push(unit.to_string());
    }
    result
}

/// `<英字>?<数字>+` は英字 + 上付きの数字（m2 → m²、x10 → x¹⁰）。`<英字>?_<文字>+` は英字 + 下付き（m_2 → m₂、x_n → xₙ）。
/// 英字だけ・数字だけの名前の先頭の英字は、そのまま残す。小さい文字にできない文字を含むときは `None`。
fn scripted(name: &str) -> Option<String> {
    let characters: Vec<char> = name.chars().collect();
    let base_end = characters
        .iter()
        .position(|c| !c.is_alphabetic())
        .unwrap_or(characters.len());
    let base: String = characters[..base_end].iter().collect();
    let mut tail = &characters[base_end..];
    if tail.is_empty() {
        return None;
    }
    let table: fn(char) -> Option<char> = if tail.first() == Some(&'_') {
        tail = &tail[1..];
        if tail.is_empty() {
            return None;
        }
        subscript
    } else {
        if !tail.iter().all(|c| c.is_numeric()) {
            return None;
        }
        superscript
    };
    let mut converted = base;
    for &character in tail {
        converted.push(table(character)?);
    }
    Some(converted)
}

fn superscript(c: char) -> Option<char> {
    Some(match c {
        '0' => '⁰',
        '1' => '¹',
        '2' => '²',
        '3' => '³',
        '4' => '⁴',
        '5' => '⁵',
        '6' => '⁶',
        '7' => '⁷',
        '8' => '⁸',
        '9' => '⁹',
        _ => return None,
    })
}

fn subscript(c: char) -> Option<char> {
    Some(match c {
        '0' => '₀',
        '1' => '₁',
        '2' => '₂',
        '3' => '₃',
        '4' => '₄',
        '5' => '₅',
        '6' => '₆',
        '7' => '₇',
        '8' => '₈',
        '9' => '₉',
        '+' => '₊',
        '-' => '₋',
        'a' => 'ₐ',
        'e' => 'ₑ',
        'h' => 'ₕ',
        'i' => 'ᵢ',
        'j' => 'ⱼ',
        'k' => 'ₖ',
        'l' => 'ₗ',
        'm' => 'ₘ',
        'n' => 'ₙ',
        'o' => 'ₒ',
        'p' => 'ₚ',
        'r' => 'ᵣ',
        's' => 'ₛ',
        't' => 'ₜ',
        'u' => 'ᵤ',
        'v' => 'ᵥ',
        'x' => 'ₓ',
        _ => return None,
    })
}

/// 単位の 1 文字の記号（Unicode の互換用。CJK Compatibility の U+33xx）。
fn unit(name: &str) -> Option<&'static str> {
    Some(match name {
        "mm" => "㎜",
        "cm" => "㎝",
        "km" => "㎞",
        "mm2" => "㎟",
        "cm2" => "㎠",
        "m2" => "㎡",
        "km2" => "㎢",
        "mm3" => "㎣",
        "cm3" => "㎤",
        "m3" => "㎥",
        "km3" => "㎦",
        "mg" => "㎎",
        "kg" => "㎏",
        "ml" => "㎖",
        "dl" => "㎗",
        "kl" => "㎘",
        "hz" => "㎐",
        "khz" => "㎑",
        "mhz" => "㎒",
        "ghz" => "㎓",
        "thz" => "㎔",
        "pa" => "㎩",
        "kpa" => "㎪",
        "mpa" => "㎫",
        "gpa" => "㎬",
        "ps" => "㎰",
        "ns" => "㎱",
        "ms" => "㎳",
        "pf" => "㎊",
        "nf" => "㎋",
        "kb" => "㎅",
        "mb" => "㎆",
        "gb" => "㎇",
        "cal" => "㎈",
        "kcal" => "㎉",
        "db" => "㏈",
        "ha" => "㏊",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_follow_the_rule() {
        let cases: [(&str, &[&str]); 12] = [
            ("m2", &["m²", "㎡"]),
            ("m3", &["m³", "㎥"]),
            ("x10", &["x¹⁰"]),
            ("2", &["²"]),
            ("m_2", &["m₂"]),
            ("x_n", &["xₙ"]),
            ("a_i", &["aᵢ"]),
            ("_2", &["₂"]),
            ("x_n+1", &["xₙ₊₁"]),
            ("cm", &["㎝"]),
            ("cm2", &["cm²", "㎠"]),
            ("kg", &["㎏"]),
        ];
        for (name, expected) in cases {
            assert_eq!(candidates(name), expected, "{name}");
        }
    }

    #[test]
    fn names_without_candidates() {
        for name in ["", "m", "eyes", "x_", "x_q", "m2x", "x_n_1", "2m"] {
            assert!(candidates(name).is_empty(), "{name:?}");
        }
    }
}
