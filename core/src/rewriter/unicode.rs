//! Unicode 表記（設定 `unicode.notation`。既定 off）。
//!
//! 読みが 1 文字のひらがなら、`U+3042` の形の候補を足す。逆方向（`u+3042` → あ）は
//! 日本語のモードでは扱わない。入力の補完はしない（読みが 1 文字でなければ何も出さない）。

use super::{RewriteCandidate, RewriteInput, Rewriter};

/// `U+<16進>` の書式ID（この Rewriter の中で一意。変えない）
const FORMAT_CODE_POINT: u16 = 1;
/// ひらがなの範囲（ぁ〜ゖ。小さいかな、濁点つきを含む。長音記号 ー は範囲外）
const HIRAGANA_FIRST: char = '\u{3041}';
const HIRAGANA_LAST: char = '\u{3096}';
/// 表記の接頭辞
const NOTATION_PREFIX: &str = "U+";

/// 設定から決まる動作（`unicode.notation`）
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UnicodeOptions {
    /// `unicode.notation`（既定 off）
    pub notation: bool,
}

pub struct UnicodeRewriter {
    options: UnicodeOptions,
}

impl UnicodeRewriter {
    pub fn new(options: UnicodeOptions) -> Self {
        Self { options }
    }
}

impl Rewriter for UnicodeRewriter {
    fn name(&self) -> &'static str {
        "unicode"
    }

    fn rewrite(&self, input: &RewriteInput<'_>, out: &mut Vec<RewriteCandidate>) {
        if !self.options.notation {
            return;
        }
        let mut chars = input.reading.chars();
        let (Some(c), None) = (chars.next(), chars.next()) else {
            return;
        };
        if !(HIRAGANA_FIRST..=HIRAGANA_LAST).contains(&c) {
            return;
        }
        out.push(RewriteCandidate::new(
            format!("{NOTATION_PREFIX}{:04X}", u32::from(c)),
            FORMAT_CODE_POINT,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::Context;
    use crate::rewriter::LocalDateTime;

    fn run(notation: bool, reading: &str) -> Vec<(u16, String)> {
        let ctx = Context::new();
        let input = RewriteInput {
            reading,
            context: &ctx,
            now: LocalDateTime {
                year: 2026,
                month: 10,
                day: 23,
                hour: 0,
                minute: 0,
                weekday: 4,
            },
        };
        let mut out = Vec::new();
        UnicodeRewriter::new(UnicodeOptions { notation }).rewrite(&input, &mut out);
        out.into_iter().map(|c| (c.format, c.surface)).collect()
    }

    fn surfaces(reading: &str) -> Vec<String> {
        run(true, reading).into_iter().map(|(_, s)| s).collect()
    }

    #[test]
    fn single_hiragana_gives_code_point() {
        assert_eq!(surfaces("あ"), ["U+3042"]);
        assert_eq!(surfaces("ぁ"), ["U+3041"]);
        assert_eq!(surfaces("が"), ["U+304C"]);
        assert_eq!(surfaces("ゖ"), ["U+3096"]);
        assert_eq!(run(true, "あ")[0].0, FORMAT_CODE_POINT);
    }

    #[test]
    fn default_is_off_and_off_gives_nothing() {
        assert!(!UnicodeOptions::default().notation);
        assert!(run(false, "あ").is_empty());
    }

    #[test]
    fn non_single_hiragana_gives_nothing() {
        for r in [
            "", "あい", "ああ", "漢", "a", "A", "ア", "ー", "1", "、", "あー",
        ] {
            assert!(surfaces(r).is_empty(), "{r}");
        }
    }
}
