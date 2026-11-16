//! TIP がつなぐ相手（エンジン、候補窓）の照合（ADR 0068 §2 の 2・§5。#409）。**OS に依存しない**（Mac でテストする）。
//!
//! 規則は 3 つの口で共有するので `ikari_pipe::peer` の 1 か所に置き、ここは TIP を照らす側にした入口だけを持つ。
//! OS の呼び出し（相手の PID → 実行ファイルのパス、DLL 自身のパス、8.3 の短い名前の展開）は COM の層
//! （`com::connector`）が行う。

pub use ikari_pipe::peer::{
    CANDIDATE_EXE_NAME, ENGINE_EXE_NAME, PeerRejection, TIP_DIR_NAME, TIP_LAYOUTS, TipLayout,
};
use ikari_pipe::peer::{Endpoint, SelfModule, verify_peer_path};

/// つないだ相手（実行ファイルのパス `peer_exe`）が、TIP の DLL（`tip_dll`）から決まるエンジンかを判定する。
pub fn verify_engine_path(tip_dll: &str, peer_exe: &str) -> Result<(), PeerRejection> {
    verify_peer_path(SelfModule::Tip, tip_dll, Endpoint::Engine, peer_exe)
}

/// つないだ相手（実行ファイルのパス `peer_exe`）が、TIP の DLL（`tip_dll`）から決まる候補窓かを判定する（S4。
/// ADR 0068 §5）。Windows の候補窓への接続（`com::connector::CandidatePipeConnector`）が、送る前に使う。
pub fn verify_candidate_path(tip_dll: &str, peer_exe: &str) -> Result<(), PeerRejection> {
    verify_peer_path(SelfModule::Tip, tip_dll, Endpoint::Candidate, peer_exe)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DLL: &str = r"C:\Program Files\ikari-ime\tip\0.3.1\ikari_tip.dll";

    // 規則の表は `ikari_pipe::peer` のテストにある。ここは入口が口を取り違えていないことだけを見る
    #[test]
    fn entry_points_pick_their_endpoint() {
        let engine = r"C:\Program Files\ikari-ime\ikari-engine-host.exe";
        let candidate = r"C:\Program Files\ikari-ime\candidate\ikari-candidate.exe";
        assert_eq!(verify_engine_path(DLL, engine), Ok(()));
        assert_eq!(
            verify_engine_path(DLL, candidate),
            Err(PeerRejection::NotThePeer)
        );
        assert_eq!(verify_candidate_path(DLL, candidate), Ok(()));
        assert_eq!(
            verify_candidate_path(DLL, engine),
            Err(PeerRejection::NotThePeer)
        );
    }
}
