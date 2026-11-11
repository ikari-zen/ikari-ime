//! 名前付きパイプの OS 層（ADR 0040 §2.2）。
//!
//! - `windows`（`cfg(windows)`）: Win32 の名前付きパイプで `Duplex` と `Listener` を実装する。`unsafe` はここだけ。
//! - ここ（OS に依存しない）: パイプに付ける DACL の文字列（SDDL）を作る。Mac でもテストする。

#[cfg(windows)]
#[allow(unsafe_code)]
pub mod windows;

/// パイプの DACL（SDDL）の先頭。`P` は親からの継承を受けない（保護する）。
const DACL_PREFIX: &str = "D:P";
/// 読み書きを許す権利（SDDL）。パイプでは `GW`（`FILE_GENERIC_WRITE`）に `FILE_CREATE_PIPE_INSTANCE`
/// （`FILE_APPEND_DATA` と同じ値）が入るので、自分のユーザーは同じ名前の次のインスタンスを作れる。
const READ_WRITE_RIGHTS: &str = "GRGW";
/// SYSTEM（SDDL の別名）。
const SYSTEM_ALIAS: &str = "SY";

/// パイプに付ける DACL の SDDL。自分のユーザー（`user_sid`）と SYSTEM だけに読み書きを許す。
///
/// ほかのユーザー、Everyone、匿名、ネットワークからは開けない（ACE が無いものは拒否される）。
/// `user_sid` は `S-` で始まり、英数字と `-` だけのものだけを受ける（SDDL の区切りを作らせない）。
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn pipe_dacl_sddl(user_sid: &str) -> Option<String> {
    let valid = user_sid.starts_with("S-")
        && user_sid.len() > "S-".len()
        && user_sid
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-');
    if !valid {
        return None;
    }
    // TODO(S7): AppContainer の TIP（ストアのアプリなど）から開けるようにするなら、次の ACE を足す
    // （`AC` = ALL APPLICATION PACKAGES = S-1-15-2-1。低い整合性のプロセスには、さらに `S:(ML;;NW;;;LW)` の
    // 必須ラベルが要る）。ADR 0040 では未決なので、既定は許さない:
    //   format!("(A;;{READ_WRITE_RIGHTS};;;AC)")
    Some(format!(
        "{DACL_PREFIX}(A;;{READ_WRITE_RIGHTS};;;{user_sid})(A;;{READ_WRITE_RIGHTS};;;{SYSTEM_ALIAS})"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SID: &str = "S-1-5-21-1004336348-1177238915-682003330-1001";

    #[test]
    fn allows_only_the_user_and_system() {
        assert_eq!(
            pipe_dacl_sddl(SID).unwrap(),
            "D:P(A;;GRGW;;;S-1-5-21-1004336348-1177238915-682003330-1001)(A;;GRGW;;;SY)"
        );
    }

    #[test]
    fn rejects_sids_that_could_break_the_sddl() {
        for sid in ["", "S-", "1-5", "S-1)(A;;GA;;;WD", "S-1;5", "S-1 5"] {
            assert_eq!(pipe_dacl_sddl(sid), None, "{sid:?}");
        }
    }
}
