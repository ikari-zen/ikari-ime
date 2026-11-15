//! TIP の DLL の置き方と、登録の判断（ADR 0063 §3 の 1・2）。OS の呼び出しは含めない（`com` が値を渡す）。
//!
//! # 置き方
//! DLL は版ごとのフォルダ `<インストール先>\tip\<版>\ikari_tip.dll` に置き、資源（キー割り当ての表など）は
//! **DLL と同じフォルダの `resources`** から読む。版をまたいで資源が混ざらない（すでに動いている古い版は、
//! 古いフォルダの資源を読み続ける）。開発用（`windows/dev/register.ps1`）も同じ形で、`<版>` の代わりに日時を使う。
//!
//! # 更新のときの登録
//! 新しい DLL の `DllRegisterServer` は、`InprocServer32` のパスを自分の場所に書き換えるだけで、先に解除しない
//! （CLSID とプロファイルの GUID は版をまたいで変えない。解除すると、利用者の入力方式の一覧から外れる恐れがある）。
//! 登録が途中で失敗したときも、**前の版の登録があった**なら解除しない（解除すると、動いていた前の版まで
//! 入力方式の一覧から消える）。新しく入れるときだけ、途中まで書いたものを消す。

use std::path::{Path, PathBuf};

/// DLL の隣の資源のディレクトリ
pub const RESOURCES_DIR_NAME: &str = "resources";

/// 登録を始める前の、この TIP の COM のクラスの登録（`InprocServer32`）の有無
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriorRegistration {
    /// 無い（新しく入れる）
    Absent,
    /// ある（更新か、入れ直し）
    Present,
    /// 読めなかった
    Unknown,
}

/// 登録が途中で失敗したときの後始末
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegisterFailureCleanup {
    /// 途中まで書いたものを解除する
    Unregister,
    /// 何もしない（前の版の登録を残す）
    Keep,
}

/// `dll` の資源のディレクトリ（DLL と同じフォルダの `resources`）。DLL のパスに親がなければ `None`。
pub fn resources_dir(dll: &Path) -> Option<PathBuf> {
    dll.parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .map(|dir| dir.join(RESOURCES_DIR_NAME))
}

/// 登録が途中で失敗したときに解除するか。前の登録があるとき、または有無が分からないときは解除しない
/// （分からないときに解除すると、前の版の登録を消しうる）。
pub fn cleanup_after_failed_register(prior: PriorRegistration) -> RegisterFailureCleanup {
    match prior {
        PriorRegistration::Absent => RegisterFailureCleanup::Unregister,
        PriorRegistration::Present | PriorRegistration::Unknown => RegisterFailureCleanup::Keep,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resources_are_next_to_the_versioned_dll() {
        let dll = Path::new("/install/tip/0.3.1/ikari_tip.dll");
        assert_eq!(
            resources_dir(dll),
            Some(PathBuf::from("/install/tip/0.3.1/resources"))
        );
    }

    #[test]
    fn each_version_has_its_own_resources() {
        let old = resources_dir(Path::new("/install/tip/0.3.0/ikari_tip.dll"));
        let new = resources_dir(Path::new("/install/tip/0.3.1/ikari_tip.dll"));
        assert_ne!(old, new);
    }

    #[test]
    fn a_dll_path_without_a_folder_has_no_resources() {
        assert_eq!(resources_dir(Path::new("ikari_tip.dll")), None);
        assert_eq!(resources_dir(Path::new("")), None);
    }

    #[test]
    fn a_failed_first_install_is_undone() {
        assert_eq!(
            cleanup_after_failed_register(PriorRegistration::Absent),
            RegisterFailureCleanup::Unregister
        );
    }

    #[test]
    fn a_failed_update_keeps_the_previous_registration() {
        assert_eq!(
            cleanup_after_failed_register(PriorRegistration::Present),
            RegisterFailureCleanup::Keep
        );
        assert_eq!(
            cleanup_after_failed_register(PriorRegistration::Unknown),
            RegisterFailureCleanup::Keep
        );
    }
}
