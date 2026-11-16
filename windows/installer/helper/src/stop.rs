//! エンジンを止める側の規則（ADR 0068 §6.2、ADR 0067 §7）。OS に依存しない（Mac でテストする）。
//!
//! 流れ（OS 層が行う）: インストール先のエンジンの exe で動いているプロセスを、すべてのセッションから探す →
//! それぞれのユーザーの SID とセッションから止める口の名前を作り、合図する → 決めた時間だけ待つ →
//! 残っていれば強制的に終わらせる。そのあと、子（候補窓、LM の子）が残っていれば終わらせる
//! （ふつうはエンジンの Job Object と一緒に終わっている）。
//!
//! 止める口の名前は `ikari_pipe::name::stop_event_name`（契約。ここでは作り方を変えない）。

use ikari_pipe::name::stop_event_name;

/// エンジンの exe の名前（インストール先の直下。ADR 0067 §3.1）
pub const ENGINE_EXE: &str = "ikari-engine-host.exe";

/// エンジンのあとに、残っていれば終わらせる子の exe（インストール先からの相対。ADR 0067 §3.1）。
/// TODO(ADR 0071): 設定画面の exe の名前が決まったら足す（利用者が開いている窓を閉じることになる）。
pub const CHILD_EXES: &[&str] = &["ikari-lm-host.exe", r"candidate\ikari-candidate.exe"];

/// 合図のあと、エンジンが自分で終わるのを待つ時間（ミリ秒）。
/// TODO(ADR 0068 §6.2): 仮置き（計測なし）。実機で終了にかかる時間を測って決める（実機の手順書の項目 51）。
pub const SIGNAL_WAIT_MS: u32 = 5_000;

/// 強制の終了のあと、プロセスが消えるのを待つ時間（ミリ秒）。TerminateProcess は終わりを待たないので要る。
/// TODO: 仮置き（計測なし）。
pub const TERMINATE_WAIT_MS: u32 = 2_000;

/// 強制的に終わらせたプロセスの終了コード（ログで見分けるための印。engine-host の終了コードと重ならない値）
pub const FORCED_EXIT_CODE: u32 = 0x4B49_0001;

/// 2 つの Win32 のパスが同じ物を指すか（大文字小文字と区切りの違い、`\\?\` の前置きを無視する）。
/// 短い名前（8.3）や、リンクの解決はしない（どちらも QueryFullProcessImageNameW と、インストーラが渡す
/// 長い名前の絶対パス）。
pub fn same_path(a: &str, b: &str) -> bool {
    normalize(a) == normalize(b)
}

fn normalize(path: &str) -> String {
    let path = path.strip_prefix(r"\\?\").unwrap_or(path);
    path.replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}

/// インストール先の中の exe の絶対パス（`relative` は `\` 区切り）
pub fn exe_path(install_dir: &str, relative: &str) -> String {
    format!("{}\\{relative}", install_dir.trim_end_matches(['\\', '/']))
}

/// 相手のユーザーとセッションから、止める口の名前。SID が名前の規則を通らなければ `None`
/// （合図はせず、強制の終了だけになる）。
pub fn event_name_for(sid: &str, session_id: u32) -> Option<String> {
    stop_event_name(sid, session_id).ok()
}

/// 止めた結果（数だけ。パスや SID は出さない）
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Outcome {
    /// 見つけたエンジン
    pub found: usize,
    /// 合図を送れた数
    pub signaled: usize,
    /// 合図のあと自分で終わった数
    pub exited: usize,
    /// 強制的に終わらせた数（エンジンと子の合計）
    pub terminated: usize,
    /// 最後まで残った数（エンジンと子の合計）
    pub remaining: usize,
}

impl Outcome {
    pub fn all_gone(&self) -> bool {
        self.remaining == 0
    }
}

impl std::fmt::Display for Outcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "found={} signaled={} exited={} terminated={} remaining={}",
            self.found, self.signaled, self.exited, self.terminated, self.remaining
        )
    }
}

/// OS のエラー報告の除外に渡す名前として使えるか。除外は名前だけで効くので（パスではない）、
/// 区切りや親への移動を含むもの、`.exe` で終わらないものは断る。
pub fn valid_exe_name(name: &str) -> bool {
    const MAX_NAME: usize = 255;
    !name.is_empty()
        && name.len() <= MAX_NAME
        && name.to_ascii_lowercase().ends_with(".exe")
        && !name.contains(['\\', '/', ':', '"', '*', '?', '<', '>', '|'])
        && !name.contains("..")
        && !name.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_compare_without_case_separators_or_long_prefix() {
        let dir = r"C:\Program Files\ikari-ime";
        let engine = exe_path(dir, ENGINE_EXE);
        assert_eq!(engine, r"C:\Program Files\ikari-ime\ikari-engine-host.exe");
        assert!(same_path(
            &engine,
            r"\\?\c:\program files\IKARI-IME\ikari-engine-host.exe"
        ));
        assert!(same_path(
            &engine,
            "C:/Program Files/ikari-ime/ikari-engine-host.exe"
        ));
        assert!(!same_path(
            &engine,
            r"C:\Users\a\AppData\Local\ikari-ime-dev\install\ikari-engine-host.exe"
        ));
        assert_eq!(
            exe_path(r"C:\Program Files\ikari-ime\", CHILD_EXES[1]),
            r"C:\Program Files\ikari-ime\candidate\ikari-candidate.exe"
        );
    }

    #[test]
    fn event_names_come_from_the_contract() {
        assert_eq!(
            event_name_for("S-1-5-21-1-2-3-1001", 2).as_deref(),
            Some(r"Global\ikari-ime-stop-v1-S-1-5-21-1-2-3-1001-s2")
        );
        assert_eq!(event_name_for(r"S-1\..\x", 1), None);
    }

    #[test]
    fn exe_names_for_error_reporting() {
        assert!(valid_exe_name("ikari-engine-host.exe"));
        assert!(valid_exe_name("IKARI-CANDIDATE.EXE"));
        for bad in [
            "",
            "ikari-engine-host",
            r"candidate\ikari-candidate.exe",
            "a/b.exe",
            "..exe",
            "C:x.exe",
            "a\u{0}.exe",
        ] {
            assert!(!valid_exe_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn outcome_is_done_only_when_nothing_remains() {
        let mut o = Outcome {
            found: 2,
            signaled: 2,
            exited: 1,
            terminated: 1,
            remaining: 0,
        };
        assert!(o.all_gone());
        o.remaining = 1;
        assert!(!o.all_gone());
        assert_eq!(
            o.to_string(),
            "found=2 signaled=2 exited=1 terminated=1 remaining=1"
        );
    }
}
