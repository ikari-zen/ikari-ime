//! 設定画面がつなぐ相手（engine-host の設定の口）の照合（ADR 0068 §4。S3 の設定画面の側）。**OS に依存しない**。
//!
//! 規則は 3 つの口で共有するので `ikari_pipe::peer` の 1 か所にあり、ここは設定画面を照らす側にした入口だけを持つ。
//! つなぐ前でなく、つないだ直後・何かを送る前に照らす（TIP と同じ。相手の PID はつないで初めて分かる）。
//!
//! OS の呼び出し（相手の PID → 実行ファイルのパス、自分の exe のパス、8.3 の短い名前の展開）は、`link::connect_installed` が
//! `ikari_pipe::os::windows`（`connect_duplex` が返す相手のパス、`long_path_name`）で取り、ここの `verify_engine_path` に渡す。

pub use ikari_pipe::peer::PeerRejection;
use ikari_pipe::peer::{Endpoint, SelfModule, verify_peer_path};

/// つないだ相手（実行ファイルのパス `peer_exe`）が、設定画面の exe（`own_exe`）から決まるインストール先の
/// engine-host かを判定する。
pub fn verify_engine_path(own_exe: &str, peer_exe: &str) -> Result<(), PeerRejection> {
    verify_peer_path(SelfModule::Settings, own_exe, Endpoint::Engine, peer_exe)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWN: &str = r"C:\Program Files\ikari-ime\settings\ikari-settings.exe";

    // 規則の表は `ikari_pipe::peer` のテストにある。ここは入口が口を取り違えていないことだけを見る
    #[test]
    fn the_peer_must_be_the_installed_engine_host() {
        assert_eq!(
            verify_engine_path(OWN, r"C:\Program Files\ikari-ime\ikari-engine-host.exe"),
            Ok(())
        );
        assert_eq!(
            verify_engine_path(
                OWN,
                r"C:\Program Files\ikari-ime\candidate\ikari-candidate.exe"
            ),
            Err(PeerRejection::NotThePeer)
        );
        assert_eq!(
            verify_engine_path(OWN, r"C:\Users\example\ikari-engine-host.exe"),
            Err(PeerRejection::NotThePeer)
        );
        // 設定画面が `settings\` の下にいなければ、インストール先を決めない
        assert_eq!(
            verify_engine_path(
                r"C:\Users\example\ikari-settings.exe",
                r"C:\Program Files\ikari-ime\ikari-engine-host.exe"
            ),
            Err(PeerRejection::SelfPathInvalid)
        );
    }
}
