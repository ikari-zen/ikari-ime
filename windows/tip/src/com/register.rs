//! 登録と解除（`regsvr32` が `DllRegisterServer` / `DllUnregisterServer` を呼ぶ）。
//!
//! 1. COM のクラス: `HKLM\Software\Classes\CLSID\{CLSID}`（既定値は名前）、`InprocServer32`（既定値は DLL のパス、
//!    `ThreadingModel` は `Apartment`。TSF は UI スレッドの STA から呼ぶ）。
//! 2. TSF の入力プロファイル（`ITfInputProcessorProfileMgr::RegisterProfile`。日本語）。
//! 3. TSF のカテゴリ（`ITfCategoryMgr::RegisterCategory`）。
//!
//! `regsvr32` は呼ぶ前に COM（OLE）を初期化する。
//!
//! 更新（ADR 0063 §3 の 1・2）: 新しい版の DLL は版ごとのフォルダ（`<インストール先>\tip\<版>\`）に置かれ、その
//! `DllRegisterServer` が呼ばれる。**先に解除しない**。上の 1 の `InprocServer32` の既定値（とプロファイルのアイコンの
//! パス）を自分の場所に書き換えるだけで、CLSID・プロファイルの GUID・カテゴリは版をまたいで同じ。登録が途中で
//! 失敗したときも、前の登録があれば解除しない（`deploy::cleanup_after_failed_register`）。
//!
//! COM のクラスは `HKEY_LOCAL_MACHINE` に書く（管理者が要る。ADR 0067 §3.3: インストーラは HKCU に触れない）。
//! `HKEY_LOCAL_MACHINE` は HKCU 側の同じキーに書くことがあるので使わない。

use std::os::windows::ffi::OsStrExt;

use windows::Win32::Foundation::{
    E_INVALIDARG, E_UNEXPECTED, ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::System::Registry::{
    HKEY_LOCAL_MACHINE, REG_SZ, RRF_RT_REG_SZ, RegDeleteTreeW, RegGetValueW, RegSetKeyValueW,
};
use windows::Win32::UI::Input::KeyboardAndMouse::HKL;
use windows::Win32::UI::TextServices::{
    CLSID_TF_CategoryMgr, CLSID_TF_InputProcessorProfiles, GUID_TFCAT_TIP_KEYBOARD,
    GUID_TFCAT_TIPCAP_IMMERSIVESUPPORT, GUID_TFCAT_TIPCAP_INPUTMODECOMPARTMENT,
    GUID_TFCAT_TIPCAP_SYSTRAYSUPPORT, ITfCategoryMgr, ITfInputProcessorProfileMgr,
};
use windows_core::{GUID, HSTRING, PCWSTR};

use super::guids::{CLSID_TEXT_SERVICE, GUID_PROFILE_JAPANESE};
use super::{dll, host, log};
use crate::deploy::{self, PriorRegistration, RegisterFailureCleanup};

/// `HKEY_LOCAL_MACHINE` の下のクラスの登録の根
const CLASSES_SUBKEY: &str = "Software\\Classes";
/// 入力メニューと設定に出る名前
const DISPLAY_NAME: &str = "ikari";
/// `ThreadingModel`
const THREADING_MODEL: &str = "Apartment";
/// 日本語（`MAKELANGID(LANG_JAPANESE, SUBLANG_JAPANESE_JAPAN)`）
const LANGID_JAPANESE: u16 = 0x0411;
/// アイコンは DLL の資源の 1 つ目（資源はまだない。W4 でアイコンを入れる）
const ICON_INDEX: u32 = 0;

/// 登録するカテゴリ。キーボードの TIP であること、タスクバーの入力の表示、Windows 8 以降のアプリの対応、
/// input mode compartment への対応。
///
/// INPUTMODECOMPARTMENT は「この TIP は input mode compartment（OPENCLOSE、INPUTMODE_CONVERSION）で
/// 入力モードを表す」という宣言（Microsoft の predefined category values）。無いと OS のインジケーターが
/// compartment の変化を表示に反映せず、英語のモードや IME オフでも「あ」のままになる疑い（#370、#371。
/// 実機で確かめる）。Mozc、weasel、corvusskk も登録している。
///
/// IMMERSIVESUPPORT は、ストアのアプリ（app container）でも読まれるようにする印。1.0.0 の範囲（ADR 0040 §2.3）は
/// デスクトップのアプリだけで、app container からはパイプにつながらない（W1-b の ACL）。つながらないときは
/// 漢字に変換できないだけで、キーは失わない。外すかは S7 の実機で決める。
const CATEGORIES: [GUID; 4] = [
    GUID_TFCAT_TIP_KEYBOARD,
    GUID_TFCAT_TIPCAP_SYSTRAYSUPPORT,
    GUID_TFCAT_TIPCAP_IMMERSIVESUPPORT,
    GUID_TFCAT_TIPCAP_INPUTMODECOMPARTMENT,
];

/// 登録する（更新のときは、前の版の登録を自分の場所に向け直す）。失敗したら、新しく入れるときだけ途中まで
/// 書いたものを消す（消せないものは残る。解除の誤りは捨てる）。
pub fn register() -> windows_core::Result<()> {
    let prior = prior_registration();
    let result = register_all();
    if result.is_err() {
        match deploy::cleanup_after_failed_register(prior) {
            RegisterFailureCleanup::Unregister => {
                let _ = unregister();
            }
            RegisterFailureCleanup::Keep => {
                log::event("register failed; keeping the previous registration");
            }
        }
    }
    result
}

fn register_all() -> windows_core::Result<()> {
    let path = module_path()?;
    register_class(&path)?;
    register_profile(&path)?;
    register_categories()?;
    Ok(())
}

/// 登録の逆。無いものは消せたとみなす（何度呼んでもよい）。最初の誤りを返し、残りも試す。
pub fn unregister() -> windows_core::Result<()> {
    let categories = unregister_categories();
    let profile = unregister_profile();
    let class = unregister_class();
    categories.and(profile).and(class)
}

/// `HKEY_LOCAL_MACHINE` の下の、クラスのキー。
fn clsid_key() -> String {
    clsid_key_for(&format!("{CLSID_TEXT_SERVICE:?}"))
}

fn clsid_key_for(clsid: &str) -> String {
    format!("{CLASSES_SUBKEY}\\CLSID\\{{{clsid}}}")
}

/// 登録を始める前の `InprocServer32` の既定値の有無（値は読まない）。
fn prior_registration() -> PriorRegistration {
    let key = HSTRING::from(format!("{}\\InprocServer32", clsid_key()));
    // SAFETY: `key` は NUL で終わる UTF-16 で、呼び出しの間生きている。値の名前は既定値（null）。
    // データとその大きさの受け口を渡さない（有無だけを見る。pvData が null なら pcbData も null でよい）
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            &key,
            PCWSTR::null(),
            RRF_RT_REG_SZ,
            None,
            None,
            None,
        )
    };
    if status == ERROR_SUCCESS {
        PriorRegistration::Present
    } else if status == ERROR_FILE_NOT_FOUND {
        PriorRegistration::Absent
    } else {
        PriorRegistration::Unknown
    }
}

fn check(status: WIN32_ERROR) -> windows_core::Result<()> {
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(status.to_hresult().into())
    }
}

/// 自分の DLL のパス（UTF-16。NUL を含まない）。
fn module_path() -> windows_core::Result<Vec<u16>> {
    let path = dll::module()
        .and_then(|module| host::module_file(Some(module)))
        .ok_or_else(|| windows_core::Error::from(E_UNEXPECTED))?;
    Ok(path.as_os_str().encode_wide().collect())
}

/// REG_SZ の値を書く（キーがなければ作る）。`name` が `None` なら既定値。
fn set_string(subkey: &str, name: Option<&str>, value: &[u16]) -> windows_core::Result<()> {
    let subkey = HSTRING::from(subkey);
    let name = name.map(HSTRING::from);
    // REG_SZ は NUL で終わる文字列のバイト列
    let mut data: Vec<u16> = value.to_vec();
    data.push(0);
    let bytes = u32::try_from(data.len() * size_of::<u16>())
        .map_err(|_| windows_core::Error::from(E_INVALIDARG))?;
    let name_ptr = name.as_ref().map_or(PCWSTR::null(), |n| PCWSTR(n.as_ptr()));
    // SAFETY: `subkey` と `name` は NUL で終わる UTF-16 で、呼び出しの間生きている。
    // `data` は `bytes` バイトの読める領域で、呼び出しの間生きている
    let status = unsafe {
        RegSetKeyValueW(
            HKEY_LOCAL_MACHINE,
            &subkey,
            name_ptr,
            REG_SZ.0,
            Some(data.as_ptr().cast()),
            bytes,
        )
    };
    check(status)
}

fn register_class(path: &[u16]) -> windows_core::Result<()> {
    let key = clsid_key();
    let name: Vec<u16> = DISPLAY_NAME.encode_utf16().collect();
    set_string(&key, None, &name)?;
    let inproc = format!("{key}\\InprocServer32");
    set_string(&inproc, None, path)?;
    let model: Vec<u16> = THREADING_MODEL.encode_utf16().collect();
    set_string(&inproc, Some("ThreadingModel"), &model)
}

fn unregister_class() -> windows_core::Result<()> {
    let key = HSTRING::from(clsid_key());
    // SAFETY: `key` は NUL で終わる UTF-16 で、呼び出しの間生きている
    let status = unsafe { RegDeleteTreeW(HKEY_LOCAL_MACHINE, &key) };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(());
    }
    check(status)
}

fn profile_mgr() -> windows_core::Result<ITfInputProcessorProfileMgr> {
    // SAFETY: COM は呼ぶ側（regsvr32）が初期化している。CLSID は OS の定数
    unsafe { CoCreateInstance(&CLSID_TF_InputProcessorProfiles, None, CLSCTX_INPROC_SERVER) }
}

fn category_mgr() -> windows_core::Result<ITfCategoryMgr> {
    // SAFETY: 同上
    unsafe { CoCreateInstance(&CLSID_TF_CategoryMgr, None, CLSCTX_INPROC_SERVER) }
}

fn register_profile(path: &[u16]) -> windows_core::Result<()> {
    let manager = profile_mgr()?;
    let description: Vec<u16> = DISPLAY_NAME.encode_utf16().collect();
    // SAFETY: GUID は定数への参照、文字列は呼び出しの間生きているスライス
    unsafe {
        manager.RegisterProfile(
            &CLSID_TEXT_SERVICE,
            LANGID_JAPANESE,
            &GUID_PROFILE_JAPANESE,
            &description,
            path,
            ICON_INDEX,
            HKL::default(),
            0,
            true,
            0,
        )
    }
}

fn unregister_profile() -> windows_core::Result<()> {
    let manager = profile_mgr()?;
    // SAFETY: GUID は定数への参照
    unsafe {
        manager.UnregisterProfile(
            &CLSID_TEXT_SERVICE,
            LANGID_JAPANESE,
            &GUID_PROFILE_JAPANESE,
            0,
        )
    }
}

fn register_categories() -> windows_core::Result<()> {
    let manager = category_mgr()?;
    for category in &CATEGORIES {
        // SAFETY: GUID は定数への参照
        unsafe { manager.RegisterCategory(&CLSID_TEXT_SERVICE, category, &CLSID_TEXT_SERVICE)? };
    }
    Ok(())
}

fn unregister_categories() -> windows_core::Result<()> {
    let manager = category_mgr()?;
    let mut first_error = Ok(());
    for category in &CATEGORIES {
        // SAFETY: GUID は定数への参照
        let result = unsafe {
            manager.UnregisterCategory(&CLSID_TEXT_SERVICE, category, &CLSID_TEXT_SERVICE)
        };
        if first_error.is_ok() {
            first_error = result;
        }
    }
    first_error
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clsid_key_is_under_software_classes() {
        assert_eq!(clsid_key_for("ABC"), "Software\\Classes\\CLSID\\{ABC}");
    }

    #[test]
    fn clsid_key_uses_registered_clsid() {
        let key = clsid_key();
        assert!(key.starts_with("Software\\Classes\\CLSID\\{"));
        assert!(key.ends_with('}'));
    }
}
