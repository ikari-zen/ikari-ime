//! macOS の LM の檻役の受け口（ADR 0077 §3.2、§3.5、§6）。**契約（第 4 弾）。中身はレーン D-3-mac**。
//!
//! - 殻が仲立ちした檻役の口は、FFI（`ikari_engine_attach_lm_cage`）から [`Engine::attach_lm_cage`] で入る。
//!   エンジンに 1 つの [`CageSlot`] に入れ、子の起動の口（`HostLauncher::Cage`）がそれを引く。
//! - 製品の macOS は檻役だけを使う（[`product_launcher`]）。今のスタブは、今までどおり自分で起動する。
//! - `HelloReply.lm_cage` の値（[`hello_lm_cage`]）。今のスタブは `LM_CAGE_UNSPECIFIED`（古いエンジンと同じ）。
//!
//! 判断の置き場所: Rust。Swift は口をエンジンに渡すだけ（XPC サービスを名前で探せるのはアプリ本体だけ。OS の制約）。

use std::sync::Arc;

use ikari_proto::v1 as pb;
use ikari_stack::HostLauncher;
use ikari_stack::lm_cage::{CageClient, CageSlot};

use crate::Engine;

/// 製品の子の起動の口（契約のスタブ: 今は自分で起動する。D-3-mac で、macOS の製品は `HostLauncher::Cage(slot)`）
pub(crate) fn product_launcher(slot: &Arc<CageSlot>) -> HostLauncher {
    let _ = slot;
    HostLauncher::Direct
}

/// `HelloReply.lm_cage`（契約のスタブ: 今は `LM_CAGE_UNSPECIFIED`）
pub(crate) fn hello_lm_cage(slot: &CageSlot) -> pb::LmCage {
    let _ = slot;
    pb::LmCage::Unspecified
}

impl Engine {
    /// 檻役の口を入れる（前の口は捨てる）。口の先の署名要件の確かめは、口を作る側（`ikari_stack::lm_cage`）が
    /// 済ませてから渡す（D-3-mac）。確かめていない口を渡さない
    pub fn attach_lm_cage(&self, client: Arc<dyn CageClient>) {
        self.inner.lm_cage.attach(client);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "D-3-mac: macOS の製品は檻役の口だけを使い、口が無ければ LM なし（段 1 に落ちない）"]
    fn macos_product_uses_only_the_cage() {
        unimplemented!("D-3-mac")
    }

    #[test]
    #[ignore = "D-3-mac: 口が無い・確かめに失敗したら HelloReply.lm_cage は LM_CAGE_NONE"]
    fn hello_reports_no_cage_without_an_endpoint() {
        unimplemented!("D-3-mac")
    }
}
