//! `IClassFactory`（`DllGetClassObject` が返す。テキストサービスを作る）。

use std::ffi::c_void;

use windows::Win32::Foundation::{CLASS_E_NOAGGREGATION, E_POINTER, E_UNEXPECTED};
use windows::Win32::System::Com::{IClassFactory, IClassFactory_Impl};
use windows_core::{BOOL, GUID, IUnknown, Interface, Ref, implement};

use super::dll::{DllRef, lock_server};
use super::guard::guard;
use super::service::TextService;

#[implement(IClassFactory)]
pub struct ClassFactory {
    _dll: DllRef,
}

impl ClassFactory {
    pub fn new() -> Self {
        Self {
            _dll: DllRef::new(),
        }
    }
}

impl IClassFactory_Impl for ClassFactory_Impl {
    fn CreateInstance(
        &self,
        outer: Ref<IUnknown>,
        riid: *const GUID,
        ppv: *mut *mut c_void,
    ) -> windows_core::Result<()> {
        guard("panic in CreateInstance", Err(E_UNEXPECTED.into()), || {
            if ppv.is_null() || riid.is_null() {
                return Err(E_POINTER.into());
            }
            // SAFETY: NULL でないことを上で確かめた。COM の約束で書ける
            unsafe { ppv.write(std::ptr::null_mut()) };
            // 集約（ほかのオブジェクトの一部になること）は受けない
            if outer.is_some() {
                return Err(CLASS_E_NOAGGREGATION.into());
            }
            let service: IUnknown = TextService::new().into();
            // SAFETY: `riid` は読める GUID、`ppv` は書ける場所（上で確かめた）。`query` が AddRef して書く
            unsafe { service.query(riid, ppv) }.ok()
        })
    }

    fn LockServer(&self, lock: BOOL) -> windows_core::Result<()> {
        guard("panic in LockServer", Ok(()), || {
            lock_server(lock.as_bool());
            Ok(())
        })
    }
}
