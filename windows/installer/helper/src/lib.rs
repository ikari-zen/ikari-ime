//! `ikari-setup-helper`: Windows のインストーラ（`windows/installer/ikari.iss`）が管理者で呼ぶ道具。
//!
//! 判断の置き場所: Rust（検査の規則、止める相手の選び方、名前の検査はこの crate の純粋な部分。Mac でテストする）。
//! Inno Setup の Pascal は、順番と画面だけを持つ。この道具に任せる理由: (1) ACL と他のセッションのプロセスは
//! Pascal から扱いにくく、テストできない。(2) Inno の Setup は 32 ビットで動くので、機械全体のレジストリ
//! （エラー報告の除外）を書くと 32 ビットの側に入りうる。この道具は 64 ビットで動く。(3) PowerShell は、
//! 利用者の場所からモジュールを読むことがあり、管理者で動かすと P1 に反する。
//!
//! 置き場所: インストール中は、インストーラが Program Files の下に作る作業用のフォルダ（管理者だけが書ける）。
//! 削除のときは、インストール先の中の写し。利用者の TEMP には置かない（ADR 0067 §3.3 の 4 つ目）。

pub mod acl;
pub mod stop;
pub mod tree;

/// 終了コード（ikari.iss の定数と同じ値。xtask の検査が照らす）
pub mod exit {
    /// 成功
    pub const OK: u8 = 0;
    /// Windows 以外
    pub const UNSUPPORTED: u8 = 1;
    /// 置き場所の検査に通らない
    pub const UNTRUSTED: u8 = 2;
    /// 止めきれなかったプロセスが残る
    pub const REMAINING: u8 = 3;
    /// OS の呼び出しの失敗
    pub const OS_ERROR: u8 = 4;
    /// 引数の誤り
    pub const USAGE: u8 = 64;
}

/// 下位コマンド
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// `check-dir <インストール先>`: 置き場所の検査（ADR 0067 §3.1）
    CheckDir(String),
    /// `stop <インストール先>`: エンジンを止める（ADR 0068 §6.2）
    Stop(String),
    /// `wer-exclude <exe の名前>...`: OS のエラー報告から外す（ADR 0073 §4）
    WerExclude(Vec<String>),
    /// `wer-include <exe の名前>...`: 外したのを戻す（削除のとき）
    WerInclude(Vec<String>),
}

pub const USAGE: &str = "usage: ikari-setup-helper (check-dir <dir> | stop <dir> | wer-exclude <exe>... | wer-include <exe>...)";

/// 引数を解く。パスは絶対パスだけを受ける（カレントディレクトリに頼らない。R2）。
pub fn parse(args: &[String]) -> Result<Command, &'static str> {
    match args {
        [cmd, dir] if cmd == "check-dir" => absolute(dir).map(Command::CheckDir),
        [cmd, dir] if cmd == "stop" => absolute(dir).map(Command::Stop),
        [cmd, names @ ..] if cmd == "wer-exclude" || cmd == "wer-include" => {
            if names.is_empty() || !names.iter().all(|n| stop::valid_exe_name(n)) {
                return Err("exe names only (no paths)");
            }
            let names = names.to_vec();
            Ok(if cmd == "wer-exclude" {
                Command::WerExclude(names)
            } else {
                Command::WerInclude(names)
            })
        }
        _ => Err(USAGE),
    }
}

/// `C:\…` の形の絶対パスだけ（UNC、デバイスのパス、相対は断る）
fn absolute(path: &str) -> Result<String, &'static str> {
    let bytes = path.as_bytes();
    let drive =
        bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\';
    if drive && !path.contains("..") && !path.chars().any(char::is_control) {
        Ok(path.to_string())
    } else {
        Err("an absolute drive path is required")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_each_command() {
        let dir = r"C:\Program Files\ikari-ime";
        assert_eq!(
            parse(&args(&["check-dir", dir])),
            Ok(Command::CheckDir(dir.into()))
        );
        assert_eq!(parse(&args(&["stop", dir])), Ok(Command::Stop(dir.into())));
        assert_eq!(
            parse(&args(&["wer-exclude", "a.exe", "b.exe"])),
            Ok(Command::WerExclude(vec!["a.exe".into(), "b.exe".into()]))
        );
        assert_eq!(
            parse(&args(&["wer-include", "a.exe"])),
            Ok(Command::WerInclude(vec!["a.exe".into()]))
        );
    }

    #[test]
    fn refuses_relative_unc_and_parent_paths() {
        for bad in [
            r"ikari-ime",
            r"\\server\share\ikari-ime",
            r"\\?\C:\Program Files\ikari-ime",
            r"C:\Program Files\..\Users\a",
            "C:",
        ] {
            assert!(parse(&args(&["check-dir", bad])).is_err(), "{bad}");
        }
    }

    #[test]
    fn refuses_paths_for_error_reporting_and_unknown_commands() {
        assert!(parse(&args(&["wer-exclude"])).is_err());
        assert!(parse(&args(&["wer-exclude", r"C:\x\a.exe"])).is_err());
        assert!(parse(&args(&["run", "x"])).is_err());
        assert!(parse(&args(&[])).is_err());
        assert!(parse(&args(&["stop", r"C:\a", "extra"])).is_err());
    }
}
