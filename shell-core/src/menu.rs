//! 入力メニューの中身の決定（今の `InputMenuModel.swift`。ADR 0005 §5、§15.2-3/4。レーン L-A1）。
//!
//! OS にも UI にも依存しない純関数。OS 側はこの結果（`MenuModel`）を `NSMenuItem` などに写すだけにする。
//!
//! 契約:
//! - `build` は純関数。`InputMenuState` が同じなら、毎回同じ `MenuModel` を返す。
//!   `menu()` はほぼ打鍵ごとに呼ばれるので、キャッシュは OS 側が `InputMenuState` を鍵にして持つ。
//! - 項目は平らに並べる（サブメニューを使わない。§5.2）。
//! - 排他の選択（ラジオ）は `MenuItemKind::Radio { group }` で表す。同じ `MenuRadioGroup` の項目は、
//!   ちょうど 1 つだけ `checked == true` になる（§15.2-3）。チェックは毎回 `state` から作り直す。

use crate::types::{
    InputMethodKind, KeymapPreset, MenuCommand, MenuItem, MenuItemKind, MenuModel, MenuRadioGroup,
    SettingsTab,
};

/// エンジンの LM の状態（`engine.proto` の `LmState`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EngineLmState {
    Unspecified,
    Disabled,
    Loading,
    Ready,
    Failed,
}

/// 分野（属性プロファイル）の重み。エンジンの `ProfileWeightKind` に対応する。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProfileWeight {
    Off,
    Sometimes,
    Often,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileDomain {
    pub id: String,
    pub name: String,
    pub weight: ProfileWeight,
}

/// メニューを決める入力のすべて。`Eq` なので、OS 側が前回の値と比べて `build` の結果を使い回せる。
///
/// Swift の `settings: Settings` は、メニューが読む 2 つの欄（`keymap_preset`、`input_method`）に分けた
/// （`SettingsSnapshot` 全体の複製を鍵にしないため）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputMenuState {
    /// `keymap.preset`
    pub keymap_preset: KeymapPreset,
    /// `input.method`
    pub input_method: InputMethodKind,
    /// `engine.lm`
    pub lm_enabled: bool,
    /// エンジンの LM の状態。まだ分からないとき `None`。読み込み中や使えないときは項目名に括弧で添える（§5.2）
    pub lm_state: Option<EngineLmState>,
    /// キャッシュしている分野。まだ取れていないとき `None`（分野の項目を出さない）。`menu()` の中でエンジンは呼ばない
    pub profile: Option<Vec<ProfileDomain>>,
}

impl InputMenuState {
    pub fn new(
        keymap_preset: KeymapPreset,
        input_method: InputMethodKind,
        lm_enabled: bool,
    ) -> Self {
        Self {
            keymap_preset,
            input_method,
            lm_enabled,
            lm_state: None,
            profile: None,
        }
    }
}

/// 項目名（ADR 0005 §5.2）。
mod title {
    pub const KEYMAP_MACOS: &str = "キー割り当て: macOS 風";
    pub const KEYMAP_MS_IME: &str = "キー割り当て: MS-IME 風";
    pub const KEYMAP_ATOK: &str = "キー割り当て: ATOK 風";
    pub const INPUT_ROMAJI: &str = "入力方式: ローマ字";
    pub const INPUT_KANA: &str = "入力方式: かな";
    pub const LM: &str = "LM（zenz）で並べ替える";
    pub const LM_LOADING: &str = "（読み込み中）";
    pub const LM_UNAVAILABLE: &str = "（使えません）";
    pub const DOMAIN_PREFIX: &str = "分野: ";
    pub const DOMAIN_SOMETIMES: &str = "（たまに）";
    pub const USER_DICTIONARY: &str = "ユーザー辞書と学習…";
    pub const PREFERENCES: &str = "環境設定…";
}

fn separator() -> MenuItem {
    MenuItem {
        kind: MenuItemKind::Separator,
        title: String::new(),
        command: None,
        checked: false,
        enabled: false,
    }
}

fn item(kind: MenuItemKind, title: String, command: MenuCommand, checked: bool) -> MenuItem {
    MenuItem {
        kind,
        title,
        command: Some(command),
        checked,
        enabled: true,
    }
}

fn radio(group: MenuRadioGroup, title: &str, command: MenuCommand, checked: bool) -> MenuItem {
    item(
        MenuItemKind::Radio { group },
        title.to_string(),
        command,
        checked,
    )
}

/// 分野が無いときの項目数（キー割り当て 3 + 入力方式 2 + LM 1 + 設定 2 + 区切り 3）。分野があるときは伸びる
const MENU_ITEM_CAPACITY: usize = 11;

/// 入力メニューの項目（ADR 0005 §5.2 の順）。純関数: `state` が同じなら結果も同じ。
pub fn build(state: &InputMenuState) -> MenuModel {
    let mut items: Vec<MenuItem> = Vec::with_capacity(MENU_ITEM_CAPACITY);

    let preset = state.keymap_preset;
    for (label, candidate) in [
        (title::KEYMAP_MACOS, KeymapPreset::Macos),
        (title::KEYMAP_MS_IME, KeymapPreset::MsIme),
        (title::KEYMAP_ATOK, KeymapPreset::Atok),
    ] {
        items.push(radio(
            MenuRadioGroup::KeymapPreset,
            label,
            MenuCommand::SelectKeymapPreset { preset: candidate },
            preset == candidate,
        ));
    }
    items.push(separator());

    let method = state.input_method;
    for (label, candidate) in [
        (title::INPUT_ROMAJI, InputMethodKind::Romaji),
        (title::INPUT_KANA, InputMethodKind::Kana),
    ] {
        items.push(radio(
            MenuRadioGroup::InputMethod,
            label,
            MenuCommand::SelectInputMethod { method: candidate },
            method == candidate,
        ));
    }
    items.push(separator());

    items.push(item(
        MenuItemKind::Toggle,
        format!(
            "{}{}",
            title::LM,
            lm_suffix(state.lm_state, state.lm_enabled)
        ),
        MenuCommand::SetLm {
            enabled: !state.lm_enabled,
        },
        state.lm_enabled,
    ));
    items.push(separator());

    if let Some(domains) = state.profile.as_ref().filter(|domains| !domains.is_empty()) {
        for domain in domains {
            let included = domain.weight != ProfileWeight::Off;
            let suffix = if domain.weight == ProfileWeight::Sometimes {
                title::DOMAIN_SOMETIMES
            } else {
                ""
            };
            items.push(item(
                MenuItemKind::Toggle,
                format!("{}{}{}", title::DOMAIN_PREFIX, domain.name, suffix),
                MenuCommand::SetProfileDomain {
                    id: domain.id.clone(),
                    included: !included,
                },
                included,
            ));
        }
        items.push(separator());
    }

    items.push(item(
        MenuItemKind::Action,
        title::USER_DICTIONARY.to_string(),
        MenuCommand::OpenSettings {
            tab: Some(SettingsTab::UserDictionary),
        },
        false,
    ));
    items.push(item(
        MenuItemKind::Action,
        title::PREFERENCES.to_string(),
        MenuCommand::OpenSettings { tab: None },
        false,
    ));
    MenuModel { items }
}

/// 読み込み中と使えないときだけ括弧で添える。切っているとき（古い状態が残りうる）と、使えるときは何も添えない。
fn lm_suffix(state: Option<EngineLmState>, enabled: bool) -> &'static str {
    if !enabled {
        return "";
    }
    match state {
        Some(EngineLmState::Loading) => title::LM_LOADING,
        Some(EngineLmState::Failed) => title::LM_UNAVAILABLE,
        Some(EngineLmState::Unspecified | EngineLmState::Disabled | EngineLmState::Ready)
        | None => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRESETS: [KeymapPreset; 3] =
        [KeymapPreset::Macos, KeymapPreset::MsIme, KeymapPreset::Atok];
    const METHODS: [InputMethodKind; 2] = [InputMethodKind::Romaji, InputMethodKind::Kana];
    const GROUPS: [MenuRadioGroup; 2] = [MenuRadioGroup::KeymapPreset, MenuRadioGroup::InputMethod];

    fn state(preset: KeymapPreset, method: InputMethodKind, lm: bool) -> InputMenuState {
        InputMenuState::new(preset, method, lm)
    }

    fn default_state() -> InputMenuState {
        state(KeymapPreset::Macos, InputMethodKind::Romaji, true)
    }

    fn domains() -> Vec<ProfileDomain> {
        let domain = |id: &str, name: &str, weight| ProfileDomain {
            id: id.into(),
            name: name.into(),
            weight,
        };
        vec![
            domain("it", "IT・ソフトウェア", ProfileWeight::Often),
            domain("infra", "インフラ・クラウド", ProfileWeight::Sometimes),
            domain("novel", "小説", ProfileWeight::Off),
        ]
    }

    fn radios(model: &MenuModel, group: MenuRadioGroup) -> Vec<&MenuItem> {
        model
            .items
            .iter()
            .filter(|i| i.kind == MenuItemKind::Radio { group })
            .collect()
    }

    fn lm_item(model: &MenuModel) -> &MenuItem {
        model
            .items
            .iter()
            .find(|i| i.title.starts_with("LM（zenz）"))
            .unwrap()
    }

    #[test]
    fn lists_the_items_in_the_order_of_the_adr() {
        let mut s = default_state();
        s.profile = Some(domains());
        let model = build(&s);
        let titles: Vec<&str> = model.items.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "キー割り当て: macOS 風",
                "キー割り当て: MS-IME 風",
                "キー割り当て: ATOK 風",
                "",
                "入力方式: ローマ字",
                "入力方式: かな",
                "",
                "LM（zenz）で並べ替える",
                "",
                "分野: IT・ソフトウェア",
                "分野: インフラ・クラウド（たまに）",
                "分野: 小説",
                "",
                "ユーザー辞書と学習…",
                "環境設定…",
            ]
        );
    }

    #[test]
    fn exactly_one_keymap_preset_is_checked_and_it_is_the_current_one() {
        for preset in PRESETS {
            let model = build(&state(preset, InputMethodKind::Romaji, true));
            let items = radios(&model, MenuRadioGroup::KeymapPreset);
            assert_eq!(items.len(), PRESETS.len());
            let checked: Vec<_> = items.iter().filter(|i| i.checked).collect();
            assert_eq!(checked.len(), 1);
            assert_eq!(
                checked[0].command,
                Some(MenuCommand::SelectKeymapPreset { preset })
            );
        }
    }

    #[test]
    fn exactly_one_input_method_is_checked_and_it_is_the_current_one() {
        for method in METHODS {
            let model = build(&state(KeymapPreset::Macos, method, true));
            let items = radios(&model, MenuRadioGroup::InputMethod);
            assert_eq!(items.len(), 2);
            let checked: Vec<_> = items.iter().filter(|i| i.checked).collect();
            assert_eq!(checked.len(), 1);
            assert_eq!(
                checked[0].command,
                Some(MenuCommand::SelectInputMethod { method })
            );
        }
    }

    #[test]
    fn every_radio_group_has_exactly_one_check_for_every_combination() {
        for preset in PRESETS {
            for method in METHODS {
                let model = build(&state(preset, method, true));
                for group in GROUPS {
                    assert_eq!(
                        radios(&model, group).iter().filter(|i| i.checked).count(),
                        1
                    );
                }
            }
        }
    }

    #[test]
    fn lm_item_toggles_and_shows_the_state() {
        let with = |lm: bool, lm_state: Option<EngineLmState>| {
            let mut s = default_state();
            s.lm_enabled = lm;
            s.lm_state = lm_state;
            build(&s)
        };
        let on = with(true, None);
        let on = lm_item(&on);
        assert_eq!(on.kind, MenuItemKind::Toggle);
        assert!(on.checked);
        assert_eq!(on.command, Some(MenuCommand::SetLm { enabled: false }));
        let off = with(false, None);
        let off = lm_item(&off);
        assert!(!off.checked);
        assert_eq!(off.command, Some(MenuCommand::SetLm { enabled: true }));

        let title = |lm: bool, st: Option<EngineLmState>| lm_item(&with(lm, st)).title.clone();
        assert_eq!(
            title(true, Some(EngineLmState::Loading)),
            "LM（zenz）で並べ替える（読み込み中）"
        );
        assert_eq!(
            title(true, Some(EngineLmState::Failed)),
            "LM（zenz）で並べ替える（使えません）"
        );
        assert_eq!(
            title(true, Some(EngineLmState::Ready)),
            "LM（zenz）で並べ替える"
        );
        assert_eq!(
            title(true, Some(EngineLmState::Unspecified)),
            "LM（zenz）で並べ替える"
        );
        // 切っているときは、古い状態が残っていても添えない
        assert_eq!(
            title(false, Some(EngineLmState::Loading)),
            "LM（zenz）で並べ替える"
        );
    }

    #[test]
    fn domain_items_toggle_between_included_and_excluded() {
        let mut s = default_state();
        s.profile = Some(domains());
        let model = build(&s);
        let find = |prefix: &str| {
            model
                .items
                .iter()
                .find(|i| i.title.starts_with(prefix))
                .unwrap()
        };
        let it = find("分野: IT・ソフトウェア");
        assert!(it.kind == MenuItemKind::Toggle && it.checked);
        assert_eq!(
            it.command,
            Some(MenuCommand::SetProfileDomain {
                id: "it".into(),
                included: false
            })
        );
        // sometimes はチェックを付けて名前に添える。選ぶと外す（3 状態を回さない）
        let infra = find("分野: インフラ");
        assert!(infra.checked);
        assert_eq!(
            infra.command,
            Some(MenuCommand::SetProfileDomain {
                id: "infra".into(),
                included: false
            })
        );
        let novel = find("分野: 小説");
        assert!(!novel.checked);
        assert_eq!(
            novel.command,
            Some(MenuCommand::SetProfileDomain {
                id: "novel".into(),
                included: true
            })
        );
    }

    #[test]
    fn domain_section_is_omitted_until_the_profile_is_known() {
        for profile in [None, Some(Vec::new())] {
            let mut s = default_state();
            s.profile = profile;
            let model = build(&s);
            assert!(!model.items.iter().any(|i| i.title.starts_with("分野")));
            // 区切りが連続しない
            for pair in model.items.windows(2) {
                assert!(
                    !(pair[0].kind == MenuItemKind::Separator
                        && pair[1].kind == MenuItemKind::Separator)
                );
            }
        }
    }

    #[test]
    fn ends_with_the_settings_items() {
        let model = build(&default_state());
        let n = model.items.len();
        assert_eq!(
            model.items[n - 2].command,
            Some(MenuCommand::OpenSettings {
                tab: Some(SettingsTab::UserDictionary)
            })
        );
        assert_eq!(
            model.items[n - 1].command,
            Some(MenuCommand::OpenSettings { tab: None })
        );
        assert_eq!(model.items[n - 1].kind, MenuItemKind::Action);
    }

    #[test]
    fn no_separator_at_the_edges_and_every_other_item_has_a_command() {
        let mut s = default_state();
        s.profile = Some(domains());
        let model = build(&s);
        assert_ne!(model.items.first().unwrap().kind, MenuItemKind::Separator);
        assert_ne!(model.items.last().unwrap().kind, MenuItemKind::Separator);
        for item in &model.items {
            if item.kind == MenuItemKind::Separator {
                assert!(item.command.is_none());
            } else {
                assert!(item.command.is_some());
                assert!(item.enabled);
            }
        }
    }

    #[test]
    fn same_state_gives_the_same_result() {
        let mut s = state(KeymapPreset::MsIme, InputMethodKind::Romaji, true);
        s.lm_state = Some(EngineLmState::Ready);
        s.profile = Some(domains());
        assert_eq!(build(&s), build(&s));
    }

    /// Swift の `menuStateIsAnEquatableKeyForCachingTheMenu`
    #[test]
    fn state_is_an_equatable_key_for_caching_the_menu() {
        let base = default_state();
        assert_eq!(base, default_state());
        assert_ne!(
            base,
            state(KeymapPreset::MsIme, InputMethodKind::Romaji, true)
        );
        assert_ne!(
            base,
            state(KeymapPreset::Macos, InputMethodKind::Romaji, false)
        );
        let mut loading = default_state();
        loading.lm_state = Some(EngineLmState::Loading);
        assert_ne!(base, loading);
        let mut with_domain = default_state();
        with_domain.profile = Some(vec![ProfileDomain {
            id: "it".into(),
            name: "IT".into(),
            weight: ProfileWeight::Often,
        }]);
        assert_ne!(base, with_domain);
    }

    #[test]
    fn settings_tab_names_are_the_names_carried_by_the_bridge() {
        let tabs = [
            SettingsTab::Input,
            SettingsTab::Conversion,
            SettingsTab::UserDictionary,
            SettingsTab::Learning,
            SettingsTab::About,
        ];
        let names: Vec<String> = tabs
            .iter()
            .map(|t| {
                serde_json::to_string(t)
                    .unwrap()
                    .trim_matches('"')
                    .to_string()
            })
            .collect();
        assert_eq!(
            names,
            ["input", "conversion", "dictionary", "learning", "about"]
        );
    }
}
