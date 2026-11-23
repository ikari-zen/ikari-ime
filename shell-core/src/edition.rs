//! 版（通常版・オフライン版）と、画面に出してよい機能（ADR 0080 §7.2）。
//!
//! 判断の置き場所: Rust（macOS と Windows で 1 つ）。Swift と C# は [`edition_capabilities`] の値（殻の FFI を通した値）で、
//! ボタンを出す・出さないを決めるだけ。
//!
//! 値は [`capabilities_for`]（入力を受ける純粋な関数）が決める。入力は 3 つ:
//! - `cfg!(feature = "online")`（取得の核をビルドに持つか。ADR 0080 §7.1）
//! - `ikari-data` が同梱の場所にあるか（自分の exe の場所から決める。調べるのは OS の入口を持つ `shell-ffi` と
//!   `windows/settings`。ここでは調べない）
//! - 自由指定のモデルを読める構成か（檻の段。自由指定は段 2 のある OS だけ。ADR 0072 Q1。決めるのはエンジン）
//!
//! [`edition_capabilities`]（引数なし）は、このビルドで分かる値だけで [`capabilities_for`] を呼ぶ。分からない 2 つは、
//! 「通常版の束には `ikari-data` がある」として出す側に置く。自由指定は、エンジンと同じ値（`ikari-resource` の `platform`）を読み、
//! 不明なときは出さない側に倒す。`ikari-data` の実際の有無で絞るのは FFI の入口（`ikari_settings_edition_capabilities`）。

use ikari_resource::platform::USER_MODELS_OFFERED;

/// 取得の核をビルドに持つか（`online` の feature。ADR 0080 §7.1）
const ONLINE: bool = cfg!(feature = "online");

/// オフライン版で、取得・版の確認の入口が呼ばれたときの文（画面はボタンを出さないので、ふつうは出ない）
pub const FETCH_UNAVAILABLE_IN_OFFLINE: &str =
    "この版（オフライン版）には、通信する機能がありません。ファイルから取り込んでください";

/// 版
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Edition {
    /// 通常版（ボタンを押したときだけ、自由指定のモデルの取得と版の確認で通信する）
    Normal,
    /// オフライン版（通信のロジックを持たない。`ikari-data` も入れない）
    Offline,
}

/// 画面に出してよい機能（ADR 0080 §7.2）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Capabilities {
    /// 版（「このソフトについて」に出す名前）
    pub edition: Edition,
    /// 「URL から取る」（自由指定のモデルの取得）
    pub url_model_fetch: bool,
    /// 「新しい版を確認」
    pub version_check: bool,
    /// 「ファイルから取り込む」（#246。両方の版で出す）
    pub file_import: bool,
}

/// [`capabilities_for`] の入力
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EditionInputs {
    /// 取得の核をビルドに持つか（`cfg!(feature = "online")`）
    pub online: bool,
    /// `ikari-data` が同梱の場所にあるか
    pub data_tool_present: bool,
    /// 自由指定のモデルを読める構成か（檻の段 2 がある。ADR 0072 Q1）
    pub user_models_offered: bool,
}

impl EditionInputs {
    /// このビルドで分かる値（`online`、この OS で自由指定を出すか）と、`ikari-data` の有無を出す側に置いた値（モジュールの説明）
    pub const fn this_build() -> EditionInputs {
        EditionInputs {
            online: ONLINE,
            data_tool_present: ONLINE,
            user_models_offered: USER_MODELS_OFFERED,
        }
    }
}

/// 入力から、版と出してよい機能を決める（純粋な関数。ADR 0080 §7.2）。
///
/// - 版の名前は、ビルドが取得の核を持つか（`online`）だけで決める（パッケージの中身では変えない）。
/// - 通信する機能（URL からの取得、版の確認）は、核があり、かつ `ikari-data` があるときだけ。
///   URL からの取得は、さらに自由指定のモデルを読める構成のときだけ（取っても読めないものを出さない）。
/// - ファイルからの取り込み（#246）は、通信しないので、いつも出す。
pub const fn capabilities_for(inputs: EditionInputs) -> Capabilities {
    let edition = if inputs.online {
        Edition::Normal
    } else {
        Edition::Offline
    };
    let can_fetch = inputs.online && inputs.data_tool_present;
    Capabilities {
        edition,
        url_model_fetch: can_fetch && inputs.user_models_offered,
        version_check: can_fetch,
        file_import: true,
    }
}

/// このビルドの版と、出してよい機能（[`EditionInputs::this_build`] で [`capabilities_for`] を呼ぶ）
pub fn edition_capabilities() -> Capabilities {
    capabilities_for(EditionInputs::this_build())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(online: bool, data_tool_present: bool, user_models_offered: bool) -> EditionInputs {
        EditionInputs {
            online,
            data_tool_present,
            user_models_offered,
        }
    }

    /// 通常版で、部品がそろっていれば、すべて出す（今の画面の挙動）
    #[test]
    fn normal_edition_with_everything_offers_everything() {
        assert_eq!(
            capabilities_for(inputs(true, true, true)),
            Capabilities {
                edition: Edition::Normal,
                url_model_fetch: true,
                version_check: true,
                file_import: true,
            }
        );
    }

    /// オフライン版は、ほかの入力にかかわらず、通信する機能を出さない。取り込みは出す
    #[test]
    fn offline_never_offers_network_features() {
        for data in [false, true] {
            for user in [false, true] {
                assert_eq!(
                    capabilities_for(inputs(false, data, user)),
                    Capabilities {
                        edition: Edition::Offline,
                        url_model_fetch: false,
                        version_check: false,
                        file_import: true,
                    },
                    "data={data} user={user}"
                );
            }
        }
    }

    /// 通常版でも `ikari-data` が無ければ通信する機能は出さない（版の名前は通常版のまま）
    #[test]
    fn normal_without_the_data_tool_hides_network_features() {
        let c = capabilities_for(inputs(true, false, true));
        assert_eq!(c.edition, Edition::Normal);
        assert!(!c.url_model_fetch && !c.version_check && c.file_import);
    }

    /// 自由指定のモデルを読めない構成では、URL からの取得だけを出さない（版の確認は出す）
    #[test]
    fn url_fetch_needs_the_user_model_cage() {
        let c = capabilities_for(inputs(true, true, false));
        assert!(!c.url_model_fetch && c.version_check && c.file_import);
    }

    /// 引数なしの関数は、ビルドの feature で版が決まる。自由指定は、エンジンと同じ値を読む
    #[test]
    fn this_build_follows_the_online_feature() {
        let user = USER_MODELS_OFFERED;
        let c = edition_capabilities();
        if cfg!(feature = "online") {
            assert_eq!(c, capabilities_for(inputs(true, true, user)));
        } else {
            assert_eq!(c, capabilities_for(inputs(false, false, user)));
        }
    }

    /// 段 2 の檻を解放していない OS（Windows 以外）では、自由指定の取得を出さない
    #[test]
    fn url_fetch_is_hidden_where_user_models_are_not_offered() {
        if !cfg!(windows) {
            assert!(!EditionInputs::this_build().user_models_offered);
            assert!(!edition_capabilities().url_model_fetch);
        }
    }

    /// 殻のスレッドをまたいで渡せる（値の型）
    #[test]
    fn capabilities_are_plain_values() {
        fn assert_traits<T: Copy + Send + Sync + 'static>() {}
        assert_traits::<Capabilities>();
        assert_traits::<Edition>();
        assert_traits::<EditionInputs>();
    }
}
