//! DLL の入口（COM の in-proc サーバー）。
//!
//! - `DllMain`: 自分のモジュールのハンドルを覚える（登録でパスを書くため）。
//! - `DllGetClassObject`: この TIP の CLSID のときだけ `IClassFactory` を返す。
//! - `DllCanUnloadNow`: 生きているオブジェクトと `LockServer` がなければ外してよい。
//! - `DllRegisterServer` / `DllUnregisterServer`: `register` に任せる。
//!
//! すべて `guard` で包む（panic は失敗の HRESULT にする）。

use std::ffi::c_void;
use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};

use windows::Win32::Foundation::{
    CLASS_E_CLASSNOTAVAILABLE, E_POINTER, E_UNEXPECTED, HINSTANCE, HMODULE, S_FALSE, S_OK,
};
use windows::Win32::System::Com::IClassFactory;
use windows::Win32::System::LibraryLoader::DisableThreadLibraryCalls;
use windows::Win32::System::SystemServices::DLL_PROCESS_ATTACH;
use windows_core::{BOOL, GUID, HRESULT, Interface};

use super::factory::ClassFactory;
use super::guard::guard;
use super::guids::CLSID_TEXT_SERVICE;
use super::{log, register};

/// 自分のモジュールのハンドル（`DllMain` の `DLL_PROCESS_ATTACH` で入れる。0 はまだ）
static MODULE: AtomicIsize = AtomicIsize::new(0);
/// 生きている COM のオブジェクトの数（`DllRef` が数える）
static LIVE_OBJECTS: AtomicUsize = AtomicUsize::new(0);
/// `IClassFactory::LockServer(TRUE)` の数
static SERVER_LOCKS: AtomicUsize = AtomicUsize::new(0);

/// 自分のモジュールのハンドル。`DllMain` の前（ありえない）は `None`。
pub fn module() -> Option<HMODULE> {
    match MODULE.load(Ordering::Acquire) {
        0 => None,
        raw => Some(HMODULE(raw as *mut c_void)),
    }
}

/// COM のオブジェクトに持たせる印。生きている間、DLL を外させない。
pub struct DllRef(());

impl DllRef {
    pub fn new() -> Self {
        LIVE_OBJECTS.fetch_add(1, Ordering::AcqRel);
        Self(())
    }
}

impl Drop for DllRef {
    fn drop(&mut self) {
        LIVE_OBJECTS.fetch_sub(1, Ordering::AcqRel);
    }
}

/// `LockServer` の数を増減する。
pub fn lock_server(lock: bool) {
    if lock {
        SERVER_LOCKS.fetch_add(1, Ordering::AcqRel);
    } else {
        // 釣り合わない `LockServer(FALSE)` で下回らないようにする
        let _ = SERVER_LOCKS.try_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_sub(1));
    }
}

/// # Safety
/// OS のローダーが呼ぶ。`module` は自分のモジュール。ローダーの錠の中なので、ほかの DLL を読む処理をしない。
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllMain(
    module: HINSTANCE,
    reason: u32,
    _reserved: *mut c_void,
) -> BOOL {
    guard("panic in DllMain", BOOL::from(true), || {
        if reason == DLL_PROCESS_ATTACH {
            MODULE.store(module.0 as isize, Ordering::Release);
            // スレッドごとの通知は使わない（受けると、スレッドの作成のたびにローダーの錠の中で呼ばれる）
            // SAFETY: `module` はローダーが渡した自分のハンドル。失敗しても動作に影響しない
            if let Err(e) = unsafe { DisableThreadLibraryCalls(HMODULE(module.0)) } {
                log::error("DisableThreadLibraryCalls failed", &e);
            }
        }
        BOOL::from(true)
    })
}

/// # Safety
/// COM が呼ぶ。`rclsid` と `riid` は読める GUID、`ppv` は書ける場所（NULL は E_POINTER で返す）。
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllGetClassObject(
    rclsid: *const GUID,
    riid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    guard("panic in DllGetClassObject", E_UNEXPECTED, || {
        if ppv.is_null() || rclsid.is_null() || riid.is_null() {
            return E_POINTER;
        }
        // SAFETY: NULL でないことを上で確かめた。COM の約束で書ける
        unsafe { ppv.write(std::ptr::null_mut()) };
        // SAFETY: NULL でないことを上で確かめた。COM の約束で読める
        if unsafe { *rclsid } != CLSID_TEXT_SERVICE {
            return CLASS_E_CLASSNOTAVAILABLE;
        }
        let factory: IClassFactory = ClassFactory::new().into();
        // SAFETY: `riid` は読める GUID、`ppv` は書ける場所（上で確かめた）。`query` が AddRef して書く
        unsafe { factory.query(riid, ppv) }
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    guard("panic in DllCanUnloadNow", S_FALSE, || {
        let idle =
            LIVE_OBJECTS.load(Ordering::Acquire) == 0 && SERVER_LOCKS.load(Ordering::Acquire) == 0;
        if idle { S_OK } else { S_FALSE }
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn DllRegisterServer() -> HRESULT {
    guard(
        "panic in DllRegisterServer",
        E_UNEXPECTED,
        || match register::register() {
            Ok(()) => S_OK,
            Err(e) => {
                log::error("DllRegisterServer failed", &e);
                // 途中まで書いたものを消す（消せないものは残る。解除の誤りは捨てる）
                let _ = register::unregister();
                e.code()
            }
        },
    )
}

#[unsafe(no_mangle)]
pub extern "system" fn DllUnregisterServer() -> HRESULT {
    guard(
        "panic in DllUnregisterServer",
        E_UNEXPECTED,
        || match register::unregister() {
            Ok(()) => S_OK,
            Err(e) => {
                log::error("DllUnregisterServer failed", &e);
                e.code()
            }
        },
    )
}
