//! 版と、画面に出してよい機能の口（ヘッダの `ikari_settings_edition_capabilities`。ADR 0080 §7.2）。
//! 形と値は Windows の `edition_abi.rs` と同じ。
//!
//! 判断の置き場所: Rust（`shell-core` の `edition::capabilities_for`）。ここは入力のうち OS の入口でしか分からないもの
//! （`ikari-data` が束の中にあるか）を調べて渡すだけ。Swift は値でボタンを出す・出さないを決めるだけ。

use ikari_shell_core::edition::{Capabilities, Edition, EditionInputs, capabilities_for};

use crate::guard;
use crate::shell::{IKARI_SHELL_ERR_NULL, IKARI_SHELL_ERR_PANIC, IKARI_SHELL_OK};

/// `IkariEditionCapabilities::edition`: 通常版
pub const IKARI_EDITION_NORMAL: i32 = 0;
/// `IkariEditionCapabilities::edition`: オフライン版
pub const IKARI_EDITION_OFFLINE: i32 = 1;

/// `ikari_settings_edition_capabilities` が書く値（真偽は 0 か 1）
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IkariEditionCapabilities {
    /// `IKARI_EDITION_*`
    pub edition: i32,
    /// 「URL から取る」
    pub url_model_fetch: u8,
    /// 「新しい版を確認」
    pub version_check: u8,
    /// 「ファイルから取り込む」
    pub file_import: u8,
}

impl From<Capabilities> for IkariEditionCapabilities {
    fn from(c: Capabilities) -> Self {
        IkariEditionCapabilities {
            edition: match c.edition {
                Edition::Normal => IKARI_EDITION_NORMAL,
                Edition::Offline => IKARI_EDITION_OFFLINE,
            },
            url_model_fetch: u8::from(c.url_model_fetch),
            version_check: u8::from(c.version_check),
            file_import: u8::from(c.file_import),
        }
    }
}

/// `ikari-data` が束の中にあるか（通常版だけが調べる。オフライン版は取得の部分を持たないので、いつも無い）
#[cfg(feature = "online")]
fn data_tool_present() -> bool {
    crate::fetch_launcher::data_tool_present()
}

#[cfg(not(feature = "online"))]
fn data_tool_present() -> bool {
    false
}

/// このビルドと、この置き場所での値
fn current() -> Capabilities {
    capabilities_for(EditionInputs {
        data_tool_present: data_tool_present(),
        ..EditionInputs::this_build()
    })
}

/// 版と、画面に出してよい機能を `*out` に書く。`out` が null なら `IKARI_SHELL_ERR_NULL`。
///
/// # Safety
/// `out` は null か書き込める。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_settings_edition_capabilities(
    out: *mut IkariEditionCapabilities,
) -> i32 {
    if out.is_null() {
        return IKARI_SHELL_ERR_NULL;
    }
    let out_addr = out as usize;
    guard(IKARI_SHELL_ERR_PANIC, || {
        let value = IkariEditionCapabilities::from(current());
        // SAFETY: `out` は null でなく書き込める（呼び出しの約束）
        unsafe { (out_addr as *mut IkariEditionCapabilities).write(value) };
        IKARI_SHELL_OK
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_out_is_refused() {
        // SAFETY: null を渡したときの約束を確かめる
        assert_eq!(
            unsafe { ikari_settings_edition_capabilities(std::ptr::null_mut()) },
            IKARI_SHELL_ERR_NULL
        );
    }

    /// テストの exe は束の中にないので、通常版でも通信する機能は出さない。取り込みは出す
    #[test]
    fn outside_the_bundle_network_features_are_hidden() {
        let mut out = IkariEditionCapabilities {
            edition: -1,
            url_model_fetch: 9,
            version_check: 9,
            file_import: 9,
        };
        // SAFETY: `out` は書き込める
        assert_eq!(
            unsafe { ikari_settings_edition_capabilities(&mut out) },
            IKARI_SHELL_OK
        );
        let edition = if cfg!(feature = "online") {
            IKARI_EDITION_NORMAL
        } else {
            IKARI_EDITION_OFFLINE
        };
        assert_eq!(
            out,
            IkariEditionCapabilities {
                edition,
                url_model_fetch: 0,
                version_check: 0,
                file_import: 1,
            }
        );
    }

    #[test]
    fn conversion_keeps_every_flag() {
        let c = Capabilities {
            edition: Edition::Normal,
            url_model_fetch: true,
            version_check: false,
            file_import: true,
        };
        assert_eq!(
            IkariEditionCapabilities::from(c),
            IkariEditionCapabilities {
                edition: IKARI_EDITION_NORMAL,
                url_model_fetch: 1,
                version_check: 0,
                file_import: 1,
            }
        );
    }
}
