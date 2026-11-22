//! 版と、画面に出してよい機能の C の ABI（ヘッダの `ikari_settings_edition_capabilities`。ADR 0080 §7.2）。
//! 形と値は macOS の `shell-ffi` の `edition.rs` と同じ。
//!
//! 判断の置き場所: Rust（`shell-core` の `edition::capabilities_for`）。ここは入力のうち OS の入口でしか分からないもの
//! （`ikari-data.exe` がインストール先にあるか）を調べて渡すだけ。画面（C#）は値でボタンを出す・出さないを決めるだけ。

use std::panic::AssertUnwindSafe;

use ikari_shell_core::edition::{Capabilities, Edition, EditionInputs, capabilities_for};

use crate::abi::{IKARI_SETTINGS_CORE_ERR_NULL, IKARI_SETTINGS_CORE_OK, guarded};

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

/// `ikari-data.exe` がインストール先にあるか（通常版だけが調べる。オフライン版は取得の部分を持たないので、いつも無い）
#[cfg(feature = "online")]
fn data_tool_present() -> bool {
    crate::fetch_os::data_tool_present()
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

/// 版と、画面に出してよい機能を `*out` に書く。`out` が null なら `ERR_NULL`。
///
/// # Safety
/// `out` は null か書き込める。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ikari_settings_edition_capabilities(
    out: *mut IkariEditionCapabilities,
) -> i32 {
    if out.is_null() {
        return IKARI_SETTINGS_CORE_ERR_NULL;
    }
    let out_addr = out as usize;
    guarded(AssertUnwindSafe(move || {
        let value = IkariEditionCapabilities::from(current());
        // SAFETY: `out` は null でなく書き込める（呼び出しの約束）
        unsafe { (out_addr as *mut IkariEditionCapabilities).write(value) };
        IKARI_SETTINGS_CORE_OK
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_out_is_refused() {
        // SAFETY: null を渡したときの約束を確かめる
        assert_eq!(
            unsafe { ikari_settings_edition_capabilities(std::ptr::null_mut()) },
            IKARI_SETTINGS_CORE_ERR_NULL
        );
    }

    /// テストの exe はインストール先にないので、通常版でも通信する機能は出さない。取り込みは出す
    #[test]
    fn outside_the_install_dir_network_features_are_hidden() {
        let mut out = IkariEditionCapabilities {
            edition: -1,
            url_model_fetch: 9,
            version_check: 9,
            file_import: 9,
        };
        // SAFETY: `out` は書き込める
        assert_eq!(
            unsafe { ikari_settings_edition_capabilities(&mut out) },
            IKARI_SETTINGS_CORE_OK
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

    /// ヘッダに口と値がある（形は macOS の `ikari_shell.h` と同じ）
    #[test]
    fn header_declares_the_edition_abi() {
        let header = include_str!("../include/ikari_settings_core.h");
        for needle in [
            "int32_t ikari_settings_edition_capabilities(IkariEditionCapabilities *out);",
            "IkariEditionCapabilities",
            &format!("#define IKARI_EDITION_NORMAL {IKARI_EDITION_NORMAL}"),
            &format!("#define IKARI_EDITION_OFFLINE {IKARI_EDITION_OFFLINE}"),
        ] {
            assert!(header.contains(needle), "{needle}");
        }
    }
}
