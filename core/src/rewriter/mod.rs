//! Rewriter（日付、数字、電卓、記号、顔文字。ADR 0002 §12）。

use crate::context::Context;

mod calc;
mod date;
mod emoticon;
mod era;
mod number;
pub mod postal;
mod symbol;
mod unicode;
mod weekday;

pub use calc::CalculatorRewriter;
pub use date::DateRewriter;
pub use emoticon::EmoticonRewriter;
pub use number::NumberRewriter;
pub use postal::{PostalOptions, PostalRewriter};
pub use symbol::{SymbolRewriter, TableParseError};
pub use unicode::{UnicodeOptions, UnicodeRewriter};
pub use weekday::{DateOptions, WeekdayRewriter};

pub trait Rewriter: Send + Sync {
    fn name(&self) -> &'static str;
    fn rewrite(&self, input: &RewriteInput<'_>, out: &mut Vec<RewriteCandidate>);
}

pub struct RewriteInput<'a> {
    /// normalize_key 済み
    pub reading: &'a str,
    pub context: &'a Context,
    pub now: LocalDateTime,
}

pub struct RewriteCandidate {
    pub surface: String,
    /// Rewriter ごとに固定の書式ID。変えない（学習が書式IDで行われるため）
    pub format: u16,
    /// 候補の置く位置（ADR 0032 §8）。既定は `Anchor`（今までと同じ）
    pub position: RewritePosition,
}

impl RewriteCandidate {
    /// 位置は `Anchor`（`rewriter_anchor` の直後）。既存の Rewriter はこれを使う。
    pub fn new(surface: String, format: u16) -> Self {
        Self {
            surface,
            format,
            position: RewritePosition::Anchor,
        }
    }

    /// 先頭（1 位）に置く候補。
    pub fn at_front(surface: String, format: u16) -> Self {
        Self {
            surface,
            format,
            position: RewritePosition::Front,
        }
    }
}

/// Rewriter の候補を置く位置（ADR 0032 §8）。
///
/// 契約コミットの時点では型だけで、`add_rewriter_candidates`（converter/conversion.rs）は
/// 値を見ずに全部 `Anchor` として扱う。`Front` の実装は #204 のレーン（レーン C の後）が行う。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RewritePosition {
    /// 辞書の候補の `rewriter_anchor` 番目の直後（既定）
    #[default]
    Anchor,
    /// 辞書の候補より前（1 位）
    Front,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalDateTime {
    pub year: i32,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    /// 0=月
    pub weekday: u8,
}

pub trait Clock: Send + Sync {
    fn now(&self) -> LocalDateTime;
}

/// ローカルのタイムゾーンの現在時刻。jiff の型は公開しない。
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> LocalDateTime {
        let z = jiff::Zoned::now();
        // jiff の各値は暦の範囲内（月 1..=12 など）なので、u8 への変換は失われない
        LocalDateTime {
            year: i32::from(z.year()),
            month: z.month() as u8,
            day: z.day() as u8,
            hour: z.hour() as u8,
            minute: z.minute() as u8,
            weekday: z.weekday().to_monday_zero_offset() as u8,
        }
    }
}

/// テスト用の固定時刻。
#[derive(Clone, Copy, Debug)]
pub struct FixedClock(pub LocalDateTime);

impl Clock for FixedClock {
    fn now(&self) -> LocalDateTime {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> LocalDateTime {
        LocalDateTime {
            year: 2026,
            month: 10,
            day: 11,
            hour: 14,
            minute: 5,
            weekday: 6,
        }
    }

    #[test]
    fn fixed_clock_returns_its_value() {
        let clock = FixedClock(sample());
        assert_eq!(clock.now(), sample());
        assert_eq!(clock.now(), sample());
    }

    #[test]
    fn clock_is_usable_as_trait_object() {
        let c: Box<dyn Clock> = Box::new(FixedClock(sample()));
        assert_eq!(c.now().year, 2026);
    }

    #[test]
    fn system_clock_returns_valid_fields() {
        let n = SystemClock.now();
        assert!((1..=12).contains(&n.month));
        assert!((1..=31).contains(&n.day));
        assert!(n.hour < 24 && n.minute < 60 && n.weekday < 7);
    }
}
