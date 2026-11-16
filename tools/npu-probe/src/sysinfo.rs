//! 報告の先頭に出す OS の版と CPU の名前（ADR 0075 §4.4）。利用者名、機械名、パスは出さない。

/// OS の版（例: `Windows 11 24H2 (26100.2605)`）
pub fn os_version() -> String {
    imp::os_version().unwrap_or_else(|| "OS 不明".into())
}

/// CPU の名前
pub fn cpu_name() -> String {
    imp::cpu_name().unwrap_or_else(|| "CPU 不明".into())
}

#[cfg(windows)]
mod imp {
    use std::ptr;

    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegGetValueW,
    };

    const CURRENT_VERSION: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
    const CPU_KEY: &str = r"HARDWARE\DESCRIPTION\System\CentralProcessor\0";
    /// この番号から Windows 11（ProductName は 10 のまま返る）
    const WINDOWS_11_BUILD: u32 = 22000;
    /// 読む文字列の長さの上限（UTF-16 の数）
    const MAX_VALUE_CHARS: usize = 512;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn reg_string(key: &str, value: &str) -> Option<String> {
        let (key, value) = (wide(key), wide(value));
        let mut buf = vec![0u16; MAX_VALUE_CHARS];
        let mut size = (buf.len() * size_of::<u16>()) as u32;
        // SAFETY: 文字列は NUL で終わる。`buf` は `size` バイトの書き込める領域
        let rc = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_SZ,
                ptr::null_mut(),
                buf.as_mut_ptr().cast(),
                &mut size,
            )
        };
        if rc != ERROR_SUCCESS {
            return None;
        }
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..len]).trim().to_owned())
    }

    fn reg_dword(key: &str, value: &str) -> Option<u32> {
        let (key, value) = (wide(key), wide(value));
        let mut out = 0u32;
        let mut size = size_of::<u32>() as u32;
        // SAFETY: 文字列は NUL で終わる。`out` は 4 バイトの書き込める領域
        let rc = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_DWORD,
                ptr::null_mut(),
                ptr::from_mut(&mut out).cast(),
                &mut size,
            )
        };
        (rc == ERROR_SUCCESS).then_some(out)
    }

    pub fn os_version() -> Option<String> {
        let build: u32 = reg_string(CURRENT_VERSION, "CurrentBuild")?.parse().ok()?;
        let ubr = reg_dword(CURRENT_VERSION, "UBR").unwrap_or(0);
        let display = reg_string(CURRENT_VERSION, "DisplayVersion").unwrap_or_default();
        let major = if build >= WINDOWS_11_BUILD { 11 } else { 10 };
        Some(format!("Windows {major} {display} ({build}.{ubr})"))
    }

    pub fn cpu_name() -> Option<String> {
        reg_string(CPU_KEY, "ProcessorNameString")
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::ffi::CStr;

    fn sysctl_string(name: &CStr) -> Option<String> {
        let mut len = 0usize;
        // SAFETY: 大きさを聞くだけの呼び出し
        if unsafe {
            libc::sysctlbyname(
                name.as_ptr(),
                std::ptr::null_mut(),
                &mut len,
                std::ptr::null_mut(),
                0,
            )
        } != 0
        {
            return None;
        }
        let mut buf = vec![0u8; len];
        // SAFETY: `buf` は `len` バイトの書き込める領域
        if unsafe {
            libc::sysctlbyname(
                name.as_ptr(),
                buf.as_mut_ptr().cast(),
                &mut len,
                std::ptr::null_mut(),
                0,
            )
        } != 0
        {
            return None;
        }
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        Some(String::from_utf8_lossy(&buf[..end]).trim().to_owned())
    }

    pub fn os_version() -> Option<String> {
        sysctl_string(c"kern.osproductversion").map(|v| format!("macOS {v}"))
    }

    pub fn cpu_name() -> Option<String> {
        sysctl_string(c"machdep.cpu.brand_string")
    }
}

/// ほかの OS（CI の Linux でビルドを通すため）。名前は出さない
#[cfg(not(any(windows, target_os = "macos")))]
mod imp {
    pub fn os_version() -> Option<String> {
        None
    }

    pub fn cpu_name() -> Option<String> {
        None
    }
}
