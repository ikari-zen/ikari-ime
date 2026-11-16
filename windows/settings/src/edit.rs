//! 画面の操作（スイッチ、数、選択肢）を、設定の口に送る書き換え（`UpdateSettings`）にする。**OS に依存しない**。
//!
//! 真偽と数は `shell-core` の `SettingsIntent` → `SettingsValues::plan` を通す（`on`/`true` の書き方の違い、範囲への
//! 丸め、変わらないときは送らない、は本体が持つ。macOS の画面と同じ道）。選択肢は、一覧の選択肢にあり、
//! `is_acceptable` を通るものだけを、その名前のまま書く。部品が `text` の設定は、この段では書き換えない。

use ikari_shell_core::bridge::proto::{SettingEntry, UpdateSettings};
use ikari_shell_core::settings::*;
use ikari_shell_core::{SettingsChanges, SettingsEntry};

/// 画面が送ってきた値
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiValue {
    Bool(bool),
    Int(i64),
    Choice(String),
}

/// 書き換えを作れない理由（文は `text`）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditError {
    /// 一覧に無いキー
    UnknownKey,
    /// 部品と値の種類が合わない
    KindMismatch,
    /// 選択肢に無い、検査を通らない
    NotAccepted,
    /// この段では書き換えない設定（Bundle ID の並び、キー割り当て）
    NotEditable,
}

/// 真偽の設定を意図にする（一覧の `bool` のキーを、すべてここに書く。テストで一覧と照らす）
fn bool_intent(key: &str, on: bool) -> Option<SettingsIntent> {
    use SettingsIntent as I;
    Some(match key {
        KEY_PAGING_WRAP => I::PagingWrap(on),
        KEY_CANDIDATE_HIDE_WHEN_CONFIDENT => I::CandidateHideWhenConfident(on),
        KEY_ENGINE_LM => I::LmEnabled(on),
        KEY_EMOJI_SHORTCODE => I::EmojiShortcode(on),
        KEY_EMOJI_SMALL_LETTERS => I::EmojiSmallLetters(on),
        KEY_POSTAL_JAPANESE => I::PostalJapanese(on),
        KEY_POSTAL_TE_PREFIX => I::PostalTePrefix(on),
        KEY_POSTAL_ENGLISH => I::PostalEnglish(on),
        KEY_UNICODE_NOTATION => I::UnicodeNotation(on),
        KEY_DATE_WEEKDAY_DATES => I::DateWeekdayDates(on),
        KEY_DEVELOPER_LM_STATUS => I::DeveloperLmStatus(on),
        _ => return None,
    })
}

/// 数の設定を意図にする（範囲への丸めは `plan`）
fn int_intent(key: &str, n: i64) -> Option<SettingsIntent> {
    use SettingsIntent as I;
    Some(match key {
        KEY_PAGING_SIZE => I::PageSize(n),
        KEY_CLAUSE_MIN_CHARS => I::ClauseMinChars(n),
        KEY_ENGINE_IDLE_UNLOAD_SECS => I::IdleUnloadSecs(n),
        KEY_ENGINE_LM_DEADLINE_MS => I::LmDeadlineMs(n),
        KEY_ENGINE_CONVERT_WAIT_MS => I::ConvertWaitMs(n),
        _ => return None,
    })
}

/// `effective`（今効いている値の一式）のもとで、`key` を `value` にする書き換え。変わらなければ `Ok(None)`（送らない）。
pub fn plan_edit(
    effective: &[(String, String)],
    key: &str,
    value: &UiValue,
) -> Result<Option<UpdateSettings>, EditError> {
    let spec = all_specs()
        .into_iter()
        .find(|s| s.key == key)
        .ok_or(EditError::UnknownKey)?;
    let intent = match (&spec.kind, value) {
        (SettingKind::Bool, UiValue::Bool(on)) => bool_intent(key, *on),
        (SettingKind::Int { .. }, UiValue::Int(n)) => int_intent(key, *n),
        (SettingKind::Enum { choices }, UiValue::Choice(choice)) => {
            if !choices.contains(&choice.as_str()) || !is_acceptable(key, choice) {
                return Err(EditError::NotAccepted);
            }
            let current = effective.iter().find(|(k, _)| k == key).map(|(_, v)| v);
            if current == Some(choice) {
                return Ok(None);
            }
            return Ok(Some(to_update(SettingsChanges {
                set: vec![SettingsEntry {
                    key: key.to_owned(),
                    value: choice.clone(),
                }],
                reset: Vec::new(),
            })));
        }
        (SettingKind::BundleIdList | SettingKind::KeyChord, _) => {
            return Err(EditError::NotEditable);
        }
        _ => return Err(EditError::KindMismatch),
    };
    // 一覧にあって、ここに書いていない真偽・数のキー（テストで止める）
    let intent = intent.ok_or(EditError::NotEditable)?;
    let values =
        SettingsValues::from_entries(effective.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    // キー割り当ての意図は作らないので、プリセットの表は空でよい
    let presets = KeymapPresetTable::new(|_| None);
    match values.plan(&intent, &presets) {
        Ok(changes) => Ok(changes.map(to_update)),
        Err(_) => Err(EditError::NotAccepted),
    }
}

/// `key` の行を消して既定に戻す書き換え
pub fn plan_reset(key: &str) -> Result<UpdateSettings, EditError> {
    if !is_known_key(key) {
        return Err(EditError::UnknownKey);
    }
    Ok(UpdateSettings {
        set: Vec::new(),
        reset: vec![key.to_owned()],
    })
}

fn to_update(changes: SettingsChanges) -> UpdateSettings {
    UpdateSettings {
        set: changes
            .set
            .into_iter()
            .map(|e| SettingEntry {
                key: e.key,
                value: e.value,
            })
            .collect(),
        reset: changes.reset,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults() -> Vec<(String, String)> {
        effective_entries(&default_snapshot())
    }

    fn single_set(update: &UpdateSettings) -> (&str, &str) {
        assert_eq!(update.set.len(), 1, "{update:?}");
        (update.set[0].key.as_str(), update.set[0].value.as_str())
    }

    /// 一覧の真偽と数の設定は、どれも書き換えを作れる（キーを足したのに、ここに書き忘れると落ちる）
    #[test]
    fn every_bool_and_int_spec_has_an_intent() {
        for spec in all_specs() {
            match spec.kind {
                SettingKind::Bool => {
                    assert!(bool_intent(&spec.key, true).is_some(), "{}", spec.key)
                }
                SettingKind::Int { .. } => {
                    assert!(int_intent(&spec.key, 1).is_some(), "{}", spec.key)
                }
                _ => {}
            }
        }
    }

    #[test]
    fn a_switch_is_written_the_way_the_store_expects_and_unchanged_values_are_not_sent() {
        let effective = defaults();
        // 既定は true。false にすると、本体の書き方（true/false）で 1 行
        let update = plan_edit(&effective, KEY_PAGING_WRAP, &UiValue::Bool(false))
            .unwrap()
            .unwrap();
        assert_eq!(single_set(&update), (KEY_PAGING_WRAP, VALUE_FALSE));
        // 既定は on。off は on/off で書く
        let update = plan_edit(&effective, KEY_ENGINE_LM, &UiValue::Bool(false))
            .unwrap()
            .unwrap();
        assert_eq!(single_set(&update), (KEY_ENGINE_LM, VALUE_OFF));
        assert_eq!(
            plan_edit(&effective, KEY_PAGING_WRAP, &UiValue::Bool(true)),
            Ok(None)
        );
    }

    #[test]
    fn numbers_are_clamped_by_the_store_rules() {
        let update = plan_edit(&defaults(), KEY_PAGING_SIZE, &UiValue::Int(99))
            .unwrap()
            .unwrap();
        assert_eq!(single_set(&update), (KEY_PAGING_SIZE, "9"));
    }

    #[test]
    fn choices_must_be_in_the_list() {
        let effective = defaults();
        let update = plan_edit(
            &effective,
            KEY_INPUT_METHOD,
            &UiValue::Choice("kana".to_owned()),
        )
        .unwrap()
        .unwrap();
        assert_eq!(single_set(&update), (KEY_INPUT_METHOD, "kana"));
        assert_eq!(
            plan_edit(
                &effective,
                KEY_INPUT_METHOD,
                &UiValue::Choice("romaji".to_owned())
            ),
            Ok(None)
        );
        assert_eq!(
            plan_edit(
                &effective,
                KEY_INPUT_METHOD,
                &UiValue::Choice("qwerty".to_owned())
            ),
            Err(EditError::NotAccepted)
        );
    }

    #[test]
    fn mismatches_unknown_keys_and_read_only_settings_are_refused() {
        let effective = defaults();
        assert_eq!(
            plan_edit(&effective, KEY_PAGING_WRAP, &UiValue::Int(1)),
            Err(EditError::KindMismatch)
        );
        assert_eq!(
            plan_edit(&effective, "no.such.key", &UiValue::Bool(true)),
            Err(EditError::UnknownKey)
        );
        assert_eq!(
            plan_edit(
                &effective,
                KEY_EMOJI_SHORTCODE_EXCLUDE,
                &UiValue::Choice(String::new())
            ),
            Err(EditError::NotEditable)
        );
    }

    #[test]
    fn reset_names_a_known_key() {
        assert_eq!(
            plan_reset(KEY_PAGING_SIZE).unwrap().reset,
            vec![KEY_PAGING_SIZE.to_owned()]
        );
        assert_eq!(plan_reset("no.such.key"), Err(EditError::UnknownKey));
    }
}
