//! 左文脈の持ち越し（ADR 0046 §7.1）。評価セットと scenario の `field`・`history` 列から、
//! core の `compose_left`（engine と同じ関数）で左文脈を作る。

use ikari_core::{CarryJoint, CarryParams, FieldLeft, LeftSources, compose_left};

/// 左文脈の最大の文字数（engine の `MAX_CONTEXT_CHARS`。eval は engine に依存しないので値を揃える）
pub const COMPOSE_MAX_CHARS: usize = 2000;
const FIELD_NONE: &str = "none";
const FIELD_AVAILABLE: &str = "available";

/// `field` 列の値。列がない、または空なら `FieldMode` を持たず、`context` をそのまま左文脈にする
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldMode {
    /// 周辺テキストが取れない（`context` は使わない）
    None,
    /// 取れた。`context` の列を入力欄の左とする
    Available,
}

/// `field` 列のセルを読む。空は `Ok(None)`（今までどおり）
pub fn parse_field(cell: &str) -> Result<Option<FieldMode>, String> {
    match cell.trim() {
        "" => Ok(None),
        FIELD_NONE => Ok(Some(FieldMode::None)),
        FIELD_AVAILABLE => Ok(Some(FieldMode::Available)),
        other => Err(format!(
            "field must be `{FIELD_NONE}` or `{FIELD_AVAILABLE}`, got `{other}`"
        )),
    }
}

/// 足さない設定（`fill = 0`。契約の既定）
pub const NO_CARRY: CarryParams = CarryParams {
    fill_chars: 0,
    joint: CarryJoint::None,
};

/// 左文脈を作る。`field` がなければ `context` をそのまま返す（`--carry-fill` に関わらず今と同じ）。
/// `carry` が `None` のときは足さない（`NO_CARRY`）
pub fn left_text(
    context: &str,
    field: Option<FieldMode>,
    history: &str,
    carry: Option<CarryParams>,
) -> String {
    let Some(mode) = field else {
        return context.to_owned();
    };
    let field_left = match mode {
        FieldMode::None => FieldLeft::Unavailable,
        FieldMode::Available => FieldLeft::Available(context),
    };
    compose_left(
        &LeftSources {
            field: field_left,
            history,
        },
        &carry.unwrap_or(NO_CARRY),
        COMPOSE_MAX_CHARS,
    )
    .text
}

#[cfg(test)]
mod tests {
    use super::*;

    const HISTORY: &str = "名詞や動詞など、品詞の話をした";

    fn carry(fill: usize, joint: CarryJoint) -> Option<CarryParams> {
        Some(CarryParams {
            fill_chars: fill,
            joint,
        })
    }

    #[test]
    fn no_field_keeps_context_as_is() {
        for c in [None, carry(40, CarryJoint::Period)] {
            assert_eq!(left_text("前の文", None, HISTORY, c), "前の文");
        }
    }

    #[test]
    fn parse_field_accepts_none_available_and_empty() {
        assert_eq!(parse_field(""), Ok(None));
        assert_eq!(parse_field(" none "), Ok(Some(FieldMode::None)));
        assert_eq!(parse_field("available"), Ok(Some(FieldMode::Available)));
        assert!(parse_field("yes").is_err());
    }

    #[test]
    fn fill_zero_does_not_add_history_to_an_available_field() {
        let off = left_text("", Some(FieldMode::Available), HISTORY, None);
        assert_eq!(off, "");
        let zero = left_text(
            "",
            Some(FieldMode::Available),
            HISTORY,
            carry(0, CarryJoint::Period),
        );
        assert_eq!(zero, "");
    }

    #[test]
    fn empty_available_field_takes_history_with_a_joint() {
        let p = left_text(
            "",
            Some(FieldMode::Available),
            HISTORY,
            carry(40, CarryJoint::Period),
        );
        assert_eq!(p, "名詞や動詞など、品詞の話をした。");
        let n = left_text(
            "",
            Some(FieldMode::Available),
            HISTORY,
            carry(40, CarryJoint::None),
        );
        assert_eq!(n, HISTORY);
    }

    #[test]
    fn overlap_adds_the_part_before_the_field() {
        let h = "品詞の話をした次は";
        let t = left_text(
            "次は",
            Some(FieldMode::Available),
            h,
            carry(40, CarryJoint::Period),
        );
        assert_eq!(t, "品詞の話をした。次は");
    }

    #[test]
    fn unavailable_field_uses_history_without_a_joint() {
        let t = left_text(
            "",
            Some(FieldMode::None),
            HISTORY,
            carry(40, CarryJoint::Period),
        );
        assert_eq!(t, HISTORY);
        let e = left_text("", Some(FieldMode::None), "", carry(40, CarryJoint::Period));
        assert_eq!(e, "");
    }
}
