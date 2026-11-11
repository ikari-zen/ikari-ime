//! 登録と解除（`regsvr32` が `DllRegisterServer` / `DllUnregisterServer` を呼ぶ）。
//!
//! 1. COM のクラス: `HKCR\CLSID\{CLSID}`（既定値は名前）、`InprocServer32`（既定値は DLL のパス、
//!    `ThreadingModel` は `Apartment`。TSF は UI スレッドの STA から呼ぶ）。
//! 2. TSF の入力プロファイル（`ITfInputProcessorProfileMgr::RegisterProfile`。日本語）。
//! 3. TSF のカテゴリ（`ITfCategoryMgr::RegisterCategory`）。
//!
//! `regsvr32` は呼ぶ前に COM（OLE）を初期化する。
//!
//! TODO(S9): いまは HKCR（`HKLM\Software\Classes`。管理者が要る）に書く。ユーザーごとの登録（HKCU）で TSF が
//! 読むか、更新のときの置き換えとあわせて W5 の実機で確かめる。

use std::os::windows::ffi::OsStrExt;

use windows::Win32::Foundation::{
    E_INVALIDARG, E_UNEXPECTED, ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::System::Registry::{
    HKEY_CLASSES_ROOT, REG_SZ, RegDeleteTreeW, RegSetKeyValueW,
};
use windows::Win32::UI::Input::KeyboardAndMouse::HKL;
use windows::Win32::UI::TextServices::{
    CLSID_TF_CategoryMgr, CLSID_TF_InputProcessorProfiles, GUID_TFCAT_TIP_KEYBOARD,
    GUID_TFCAT_TIPCAP_IMMERSIVESUPPORT, GUID_TFCAT_TIPCAP_SYSTRAYSUPPORT, ITfCategoryMgr,
    ITfInputProcessorProfileMgr,
};
use windows_core::{GUID, HSTRING, PCWSTR};

use super::guids::{CLSID_TEXT_SERVICE, GUID_PROFILE_JAPANESE};
use super::{dll, host};

/// 入力メニューと設定に出る名前
const DISPLAY_NAME: &str = "ikari";
/// `ThreadingModel`
const THREADING_MODEL: &str = "Apartment";
/// 日本語（`MAKELANGID(LANG_JAPANESE, SUBLANG_JAPANESE_JAPAN)`）
const LANGID_JAPANESE: u16 = 0x0411;
/// アイコンは DLL の資源の 1 つ目（資源はまだない。W4 でアイコンを入れる）
const ICON_INDEX: u32 = 0;

/// 登録するカテゴリ。キーボードの TIP であること、タスクバーの入力の表示、Windows 8 以降のアプリの対応。
///
/// IMMERSIVESUPPORT は、ストアのアプリ（app container）でも読まれるようにする印。1.0.0 の範囲（ADR 0040 §2.3）は
/// デスクトップのアプリだけで、app container からはパイプにつながらない（W1-b の ACL）。つながらないときは
/// 漢字に変換できないだけで、キーは失わない。外すかは S7 の実機で決める。
const CATEGORIES: [GUID; 3] = [
    GUID_TFCAT_TIP_KEYBOARD,
    GUID_TFCAT_TIPCAP_SYSTRAYSUPPORT,
    GUID_TFCAT_TIPCAP_IMMERSIVESUPPORT,
];

pub fn register() -> windows_core::Result<()> {
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

fn clsid_key() -> String {
    format!("CLSID\\{{{CLSID_TEXT_SERVICE:?}}}")
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
            HKEY_CLASSES_ROOT,
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
    let status = unsafe { RegDeleteTreeW(HKEY_CLASSES_ROOT, &key) };
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
