//! 名前付きパイプの OS 層（ADR 0040 §2.2）。
//!
//! - `windows`（`cfg(windows)`）: Win32 の名前付きパイプで `Duplex` と `Listener` を実装する。`unsafe` はここだけ。
//! - ここ（OS に依存しない）: パイプに付ける DACL の文字列（SDDL）を作る。Mac でもテストする。

#[cfg(windows)]
#[allow(unsafe_code)]
pub mod windows;

/// パイプの持ち主を指す SDDL の先頭（`O:<SID>`）。作る者が自分の SID を持ち主に書く。既定の持ち主は、昇格した
/// 管理者のトークンでは Administrators になりうるので、明示する。つなぐ側は、この持ち主が自分かを確かめる（FB-1）。
const OWNER_PREFIX: &str = "O:";
/// パイプの DACL（SDDL）の先頭。`P` は親からの継承を受けない（保護する）。
const DACL_PREFIX: &str = "D:P";
/// 整合性のラベル（SDDL の SACL）。中より低い整合性のプロセスからは、書き込みと読み取りの両方で開けない（FP-1。
/// ラベルが無いと、低い整合性は読み取りだけでつなげる）。`NW` = 上への書き込みを止める、`NR` = 読み取りを止める、
/// `ME` = 中の整合性。
const MANDATORY_LABEL: &str = "S:(ML;;NWNR;;;ME)";
/// 読み書きを許す権利（SDDL）。パイプでは `GW`（`FILE_GENERIC_WRITE`）に `FILE_CREATE_PIPE_INSTANCE`
/// （`FILE_APPEND_DATA` と同じ値）が入るので、自分のユーザーは同じ名前の次のインスタンスを作れる。
const READ_WRITE_RIGHTS: &str = "GRGW";
/// SYSTEM（SDDL の別名）。
const SYSTEM_ALIAS: &str = "SY";

/// パイプに付ける DACL の SDDL。自分のユーザー（`user_sid`）と SYSTEM だけに読み書きを許す。
///
/// ほかのユーザー、Everyone、匿名、ネットワークからは開けない（ACE が無いものは拒否される）。持ち主は自分のユーザー、
/// 整合性のラベルは中（中より低いプロセスは読み取りでも開けない）。
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
    // 必須ラベルが要る。今は `MANDATORY_LABEL` が中より低いものを止めているので、そのときは見直す）。ADR 0040 では未決なので、既定は許さない:
    //   format!("(A;;{READ_WRITE_RIGHTS};;;AC)")
    Some(format!(
        "{OWNER_PREFIX}{user_sid}{DACL_PREFIX}(A;;{READ_WRITE_RIGHTS};;;{user_sid})(A;;{READ_WRITE_RIGHTS};;;{SYSTEM_ALIAS}){MANDATORY_LABEL}"
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
            "O:S-1-5-21-1004336348-1177238915-682003330-1001D:P\
             (A;;GRGW;;;S-1-5-21-1004336348-1177238915-682003330-1001)(A;;GRGW;;;SY)\
             S:(ML;;NWNR;;;ME)"
        );
    }

    #[test]
    fn rejects_sids_that_could_break_the_sddl() {
        for sid in ["", "S-", "1-5", "S-1)(A;;GA;;;WD", "S-1;5", "S-1 5"] {
            assert_eq!(pipe_dacl_sddl(sid), None, "{sid:?}");
        }
    }
}
