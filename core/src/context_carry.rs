//! 左文脈の組み立て（ADR 0046 §3、§4、§8.1）。
//!
//! 入力欄の左（周辺テキスト）が足りないぶんを、同じセッションの確定履歴の末尾で補う純関数。
//! engine と eval が同じ関数を呼ぶ。長さはすべて文字（`char`）で数える。

/// 入力欄の左（周辺テキスト）の取れ方
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldLeft<'a> {
    /// 周辺テキストが取れない
    Unavailable,
    /// 取れた。入力欄の左（呼び出し側が末尾 `max_chars` 文字に切ったもの）
    Available(&'a str),
}

/// 足した履歴と入力欄の左のあいだの区切り（境目だと分かっているときだけ入れる）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CarryJoint {
    None,
    /// 「。」
    Period,
    /// 改行
    Newline,
}

impl CarryJoint {
    fn as_str(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Period => "。",
            Self::Newline => "\n",
        }
    }
}

/// 組み立ての値。`fill_chars == 0` なら履歴を足さない（今の動き）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CarryParams {
    pub fill_chars: usize,
    pub joint: CarryJoint,
}

impl CarryParams {
    /// 既定（ADR 0049 K1）。engine と eval の両方がこれを使う。
    /// 契約のコミットでは今と同じ動き（`fill_chars: 0`）。統合で 40 にする（ADR 0049 §7）
    pub const DEFAULT: CarryParams = CarryParams {
        fill_chars: 0,
        joint: CarryJoint::None,
    };
}

/// 組み立ての材料
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LeftSources<'a> {
    pub field: FieldLeft<'a>,
    /// 同じセッションの確定の表記の連結
    pub history: &'a str,
}

/// 左文脈の出どころ
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeftOrigin {
    Empty,
    Field,
    FieldWithHistory,
    History,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComposedLeft {
    pub text: String,
    pub origin: LeftOrigin,
    /// `text` の先頭のうち、履歴から足した字数（区切りを含む）。
    /// History なら `text` 全体の字数、Field・Empty なら 0（ADR 0049 K2）
    pub carried_chars: usize,
}

/// 区切りを入れない末尾の文字（句読点、閉じ括弧、改行、空白）
const NO_JOINT_AFTER: &[char] = &[
    '。', '．', '.', '！', '？', '!', '?', '、', '，', '」', '』', '）', ')', '】', '\n', '\r',
    ' ', '　', '\t',
];

fn tail_chars(s: &str, n: usize) -> &str {
    let total = s.chars().count();
    if total <= n {
        return s;
    }
    let idx = s.char_indices().nth(total - n).map_or(s.len(), |(i, _)| i);
    &s[idx..]
}

/// `x` の後ろに入れる区切り。末尾が句読点などなら空
fn joint_after(x: &str, joint: CarryJoint) -> &'static str {
    match x.chars().last() {
        Some(c) if NO_JOINT_AFTER.contains(&c) => "",
        _ => joint.as_str(),
    }
}

fn field_only(l: &str, max_chars: usize) -> ComposedLeft {
    let origin = if l.is_empty() {
        LeftOrigin::Empty
    } else {
        LeftOrigin::Field
    };
    ComposedLeft {
        text: tail_chars(l, max_chars).to_string(),
        origin,
        carried_chars: 0,
    }
}

/// 左文脈を組み立てる（擬似コードは ADR 0046 §3.2）。`max_chars` は左文脈の最大の文字数
pub fn compose_left(src: &LeftSources<'_>, p: &CarryParams, max_chars: usize) -> ComposedLeft {
    let h = src.history;
    let l = match src.field {
        FieldLeft::Unavailable => {
            // 取れない入力欄は今のまま。区切りなし（境目か分からない）
            return if h.is_empty() {
                ComposedLeft {
                    text: String::new(),
                    origin: LeftOrigin::Empty,
                    carried_chars: 0,
                }
            } else {
                let text = tail_chars(h, max_chars).to_string();
                let carried_chars = text.chars().count();
                ComposedLeft {
                    text,
                    origin: LeftOrigin::History,
                    carried_chars,
                }
            };
        }
        FieldLeft::Available(l) => l,
    };
    // 足さない設定（既定）では、今と同じ。取れた左がすべて
    if p.fill_chars == 0 {
        return field_only(l, max_chars);
    }
    if l.trim().is_empty() {
        // 送信で空になった入力欄。履歴のあとに区切りを入れる
        if h.is_empty() {
            return ComposedLeft {
                text: tail_chars(l, max_chars).to_string(),
                origin: LeftOrigin::Empty,
                carried_chars: 0,
            };
        }
        let j = joint_after(h, p.joint);
        let keep = max_chars.saturating_sub(j.chars().count());
        let text = format!("{}{}", tail_chars(h, keep), j);
        let carried_chars = text.chars().count();
        return ComposedLeft {
            text,
            origin: LeftOrigin::History,
            carried_chars,
        };
    }
    let deficit = p.fill_chars.saturating_sub(l.chars().count());
    if deficit == 0 {
        return field_only(l, max_chars);
    }
    // 重なりの除去: 左が履歴の接尾辞のときだけ足す
    let Some(prefix) = h.strip_suffix(l) else {
        return field_only(l, max_chars);
    };
    if prefix.is_empty() {
        return field_only(l, max_chars);
    }
    let j = joint_after(prefix, p.joint);
    let j_len = j.chars().count();
    if deficit <= j_len {
        return field_only(l, max_chars);
    }
    let take = (deficit - j_len).min(max_chars.saturating_sub(l.chars().count() + j_len));
    let carried = format!("{}{}", tail_chars(prefix, take), j);
    ComposedLeft {
        carried_chars: carried.chars().count(),
        text: format!("{carried}{l}"),
        origin: LeftOrigin::FieldWithHistory,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAX: usize = 2000;
    const P40: CarryParams = CarryParams {
        fill_chars: 40,
        joint: CarryJoint::Period,
    };

    fn run(field: FieldLeft<'_>, h: &str, p: &CarryParams) -> (String, LeftOrigin) {
        let r = compose_left(&LeftSources { field, history: h }, p, MAX);
        (r.text, r.origin)
    }

    fn avail(l: &str, h: &str) -> (String, LeftOrigin) {
        run(FieldLeft::Available(l), h, &P40)
    }

    #[test]
    fn ex1_emptied_field_adds_joint() {
        let (t, o) = avail("", "名詞や動詞など、品詞の話をした");
        assert_eq!(t, "名詞や動詞など、品詞の話をした。");
        assert_eq!(o, LeftOrigin::History);
    }

    #[test]
    fn ex2_no_joint_after_period() {
        let (t, _) = avail("", "名詞や動詞など、品詞の話をした。");
        assert_eq!(t, "名詞や動詞など、品詞の話をした。");
    }

    #[test]
    fn ex3_overlap_fills_with_joint() {
        let (t, o) = avail("次は", "品詞の話をした次は");
        assert_eq!(t, "品詞の話をした。次は");
        assert_eq!(o, LeftOrigin::FieldWithHistory);
    }

    #[test]
    fn ex4_mismatch_adds_nothing() {
        let (t, o) = avail("次は", "品詞の話をした次の");
        assert_eq!(t, "次は");
        assert_eq!(o, LeftOrigin::Field);
    }

    #[test]
    fn ex5_not_suffix_adds_nothing() {
        assert_eq!(avail("次は", "品詞の話をした").0, "次は");
    }

    #[test]
    fn ex6_left_longer_than_history() {
        let l = "メモ：品詞の話をした次は";
        assert_eq!(avail(l, "品詞の話をした次は").0, l);
    }

    #[test]
    fn ex7_long_left_unchanged() {
        let l = "あ".repeat(41);
        let (t, o) = avail(&l, "何でも");
        assert_eq!(t, l);
        assert_eq!(o, LeftOrigin::Field);
    }

    #[test]
    fn ex8_prefix_empty() {
        assert_eq!(
            avail("品詞の話をした次は", "品詞の話をした次は").0,
            "品詞の話をした次は"
        );
    }

    #[test]
    fn ex9_deficit_not_above_joint() {
        // 39 文字の左: 足りないのは 1 文字で、区切り 1 文字に等しいので足さない
        let l = format!("{}次は", "あ".repeat(37));
        assert_eq!(l.chars().count(), 39);
        let h = format!("前の文{l}");
        assert_eq!(avail(&l, &h).0, l);
        // 38 文字なら、足りない 2 文字のうち履歴は 1 文字
        let l = format!("{}次は", "あ".repeat(36));
        let h = format!("前の文{l}");
        assert_eq!(avail(&l, &h).0, format!("文。{l}"));
    }

    #[test]
    fn ex10_unavailable_uses_history_without_joint() {
        let (t, o) = run(FieldLeft::Unavailable, "品詞の話をした", &P40);
        assert_eq!(t, "品詞の話をした");
        assert_eq!(o, LeftOrigin::History);
    }

    #[test]
    fn ex11_whitespace_without_history() {
        let (t, o) = avail(" ", "");
        assert_eq!(t, " ");
        assert_eq!(o, LeftOrigin::Empty);
    }

    #[test]
    fn ex12_same_field_continuation_gets_joint() {
        assert_eq!(avail("を", "品詞を").0, "品詞。を");
        let none = CarryParams {
            fill_chars: 40,
            joint: CarryJoint::None,
        };
        assert_eq!(run(FieldLeft::Available("を"), "品詞を", &none).0, "品詞を");
    }

    #[test]
    fn ex13_unavailable_without_history() {
        let (t, o) = run(FieldLeft::Unavailable, "", &P40);
        assert_eq!(t, "");
        assert_eq!(o, LeftOrigin::Empty);
    }

    #[test]
    fn fill_zero_keeps_current_behaviour() {
        let p = CarryParams {
            fill_chars: 0,
            joint: CarryJoint::Period,
        };
        assert_eq!(
            run(FieldLeft::Available(""), "品詞の話をした", &p),
            (String::new(), LeftOrigin::Empty)
        );
        assert_eq!(
            run(FieldLeft::Available("次は"), "品詞の話をした次は", &p).0,
            "次は"
        );
        assert_eq!(run(FieldLeft::Unavailable, "品詞", &p).0, "品詞");
    }

    #[test]
    fn newline_joint_and_max_chars() {
        let p = CarryParams {
            fill_chars: 40,
            joint: CarryJoint::Newline,
        };
        assert_eq!(
            run(FieldLeft::Available("次は"), "品詞の話をした次は", &p).0,
            "品詞の話をした\n次は"
        );
        let r = compose_left(
            &LeftSources {
                field: FieldLeft::Unavailable,
                history: "あいうえお",
            },
            &P40,
            3,
        );
        assert_eq!(r.text, "うえお");
    }

    fn carried(field: FieldLeft<'_>, h: &str, p: &CarryParams) -> usize {
        compose_left(&LeftSources { field, history: h }, p, MAX).carried_chars
    }

    #[test]
    fn carried_chars_counts_the_prefix_with_joint() {
        // 履歴だけ（区切り込み）。Unavailable は全体
        assert_eq!(carried(FieldLeft::Available(""), "品詞の話をした", &P40), 8);
        assert_eq!(carried(FieldLeft::Unavailable, "品詞の話", &P40), 4);
        // 足した前文 + 区切り + L
        let r = compose_left(
            &LeftSources {
                field: FieldLeft::Available("次は"),
                history: "品詞の話をした次は",
            },
            &P40,
            MAX,
        );
        assert_eq!(r.text, "品詞の話をした。次は");
        assert_eq!(r.carried_chars, 8);
    }

    #[test]
    fn carried_chars_is_zero_without_carry() {
        assert_eq!(
            carried(FieldLeft::Available("次は"), "品詞の話をした次は", &P40),
            8
        );
        let none = CarryParams::DEFAULT;
        assert_eq!(none.fill_chars, 0);
        assert_eq!(
            carried(FieldLeft::Available("次は"), "品詞の話をした次は", &none),
            0
        );
        assert_eq!(carried(FieldLeft::Available(""), "", &P40), 0);
        assert_eq!(carried(FieldLeft::Unavailable, "", &P40), 0);
        let long = "あ".repeat(50);
        assert_eq!(carried(FieldLeft::Available(&long), "x", &P40), 0);
    }
}
