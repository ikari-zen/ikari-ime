//! macOS の LM の檻役の受け口（ADR 0077 §3.2、§3.5、§6）。
//!
//! - 殻が仲立ちした檻役の口は、FFI（`ikari_engine_attach_lm_cage`）から [`Engine::attach_lm_cage`] で入る。
//!   エンジンに 1 つの [`CageSlot`] に入れ、子の起動の口（`HostLauncher::Cage`）がそれを引く。
//! - 製品の macOS は檻役だけを使う（[`product_launcher`]）。テストと開発用のビルド、macOS 以外は、自分で起動する。
//! - `HelloReply.lm_cage` の値（[`hello_lm_cage`]）。檻役を使うビルドは口があれば `LEVEL2`、無ければ `NONE`。使わないビルド（テスト、`dev-knobs`、macOS 以外）は `LM_CAGE_UNSPECIFIED`（古いエンジンと同じ）。
//!
//! 判断の置き場所: Rust。Swift は口をエンジンに渡すだけ（XPC サービスを名前で探せるのはアプリ本体だけ。OS の制約）。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ikari_proto::v1 as pb;
use ikari_stack::lm_cage::{CAGE_SERVICE_BUNDLE, CageClient, CageSlot, HandshakeError};
use ikari_stack::{HostLauncher, lm_host_exe_name};

use crate::Engine;

/// 檻役だけを使うビルドか（macOS の製品。ADR 0077 §6）。テストと開発用のビルド（`dev-knobs`）は、
/// 自分で起動する（`lm` の `os/apple.rs` の `direct_spawn_allowed` と同じ条件。FC-4）
pub(crate) const fn product_uses_cage() -> bool {
    cfg!(all(
        target_vendor = "apple",
        not(any(test, feature = "dev-knobs"))
    ))
}

/// 製品の子の起動の口。macOS の製品は檻役だけ（口が無ければ LM なし。段 1 に落ちない）、ほかは自分で起動する
pub(crate) fn product_launcher(slot: &Arc<CageSlot>) -> HostLauncher {
    launcher_for(product_uses_cage(), slot)
}

fn launcher_for(uses_cage: bool, slot: &Arc<CageSlot>) -> HostLauncher {
    if uses_cage {
        HostLauncher::Cage(Arc::clone(slot))
    } else {
        HostLauncher::Direct
    }
}

/// 檻役を使うビルドの、子（`ikari-lm-host`）の場所。子はエンジンの束ではなく檻役の束にある（ADR 0077 §3.4）ので、
/// エンジンの隣を探すと見つからず、LM を組まない（`lm=failed`）。エンジンはこのパスを起動しない（起動は檻役が
/// 自分の束の中の絶対パスで行う）。組み立てに「別プロセスの LM」を組ませるための値で、ファイルの有無は見ない
/// （Sandbox のエンジンからは、ほかの束の中を見られるとは限らない）。檻役を使わないビルドは None
pub(crate) fn caged_host_program(engine_exe: &Path) -> Option<PathBuf> {
    caged_host_program_with(product_uses_cage(), engine_exe)
}

fn caged_host_program_with(uses_cage: bool, engine_exe: &Path) -> Option<PathBuf> {
    if !uses_cage {
        return None;
    }
    // `…/XPCServices/IkariEngine.xpc/Contents/MacOS/IkariEngine` → `…/XPCServices`
    let services = engine_exe.ancestors().nth(4)?;
    Some(
        services
            .join(CAGE_SERVICE_BUNDLE)
            .join("Contents/MacOS")
            .join(lm_host_exe_name()),
    )
}

/// `HelloReply.lm_cage`。檻役を使うビルドは、口があれば `LEVEL2`、無ければ `NONE`（設定画面の「檻なし」）。
/// 使わないビルドは `UNSPECIFIED`（古いエンジンと同じ）
pub(crate) fn hello_lm_cage(slot: &CageSlot) -> pb::LmCage {
    cage_report(product_uses_cage(), slot)
}

fn cage_report(uses_cage: bool, slot: &CageSlot) -> pb::LmCage {
    match (uses_cage, slot.current()) {
        (false, _) => pb::LmCage::Unspecified,
        (true, Some(_)) => pb::LmCage::Level2,
        (true, None) => pb::LmCage::None,
    }
}

/// 檻役の口を待っているか（檻役を使うビルドで、口がまだ無い）。待っている間は読み込みを始めない
/// （起動できないので、読み込みの失敗に数えられて `lm=failed` になり、口が届いても再試行の間隔まで待たされる）。
/// 口が入ったら [`Engine::attach_lm_cage`] が読み込みを始める
pub(crate) fn waiting_for_cage(slot: &CageSlot) -> bool {
    waiting_for_cage_with(product_uses_cage(), slot)
}

fn waiting_for_cage_with(uses_cage: bool, slot: &CageSlot) -> bool {
    uses_cage && slot.current().is_none()
}

/// 自由指定のモデルを、いま使ってよいか（ADR 0077 §6）。檻役を使うビルドは、口がある（確かめ済み）ときだけ。
/// ほかは、段 2 を張れるかを起動の側が決める（Windows。ADR 0072 §12.8 の 1）
#[allow(dead_code)] // D-4（自由指定のモデルの選択）で使う
pub(crate) fn user_model_usable(slot: &CageSlot) -> bool {
    user_model_usable_with(product_uses_cage(), slot)
}

fn user_model_usable_with(uses_cage: bool, slot: &CageSlot) -> bool {
    !uses_cage || slot.current().is_some()
}

impl Engine {
    /// 殻から届いた檻役の口を確かめ（`hello` の返事の送り手の署名要件と版。FC-1）、通れば入れる（前の口は捨てる）。
    /// 通らなければ届いた口を捨て、今の口はそのまま。確かめる前の口には `hello` しか送らない。
    /// 入ったら、読み込みを始める（口を待つ間は始めない。[`waiting_for_cage`]）
    pub fn attach_lm_cage(&self, client: Arc<dyn CageClient>) -> Result<(), HandshakeError> {
        self.inner.lm_cage.attach_unverified(client)?;
        self.inner.ensure_warm();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use ikari_stack::lm_cage::{
        CAGE_PROTOCOL_VERSION, CageChild, CageLaunch, CageReply, HelloAnswer, PeerCheck,
    };

    use super::*;

    /// `hello` に本物らしく答えるだけの口（起動は受けない）
    #[derive(Debug)]
    struct Good;

    impl CageClient for Good {
        fn hello(&self) -> io::Result<HelloAnswer> {
            Ok(HelloAnswer {
                peer: PeerCheck::Verified,
                reply: Some(CageReply::Ok),
                version: Some(u64::from(CAGE_PROTOCOL_VERSION)),
            })
        }

        fn launch(&self, _launch: CageLaunch) -> io::Result<Box<dyn CageChild>> {
            Err(io::Error::from(io::ErrorKind::Unsupported))
        }
    }

    #[derive(Debug)]
    struct Forged;

    impl CageClient for Forged {
        fn hello(&self) -> io::Result<HelloAnswer> {
            Ok(HelloAnswer {
                peer: PeerCheck::Rejected,
                reply: Some(CageReply::Ok),
                version: Some(u64::from(CAGE_PROTOCOL_VERSION)),
            })
        }

        fn launch(&self, _launch: CageLaunch) -> io::Result<Box<dyn CageChild>> {
            unreachable!("a rejected cage must never get a launch")
        }
    }

    #[test]
    fn macos_product_uses_only_the_cage() {
        let slot = Arc::new(CageSlot::default());
        assert!(matches!(launcher_for(true, &slot), HostLauncher::Cage(_)));
        assert!(matches!(launcher_for(false, &slot), HostLauncher::Direct));
        // このテストのビルド（cfg(test)）は直接。製品の判定は、テストと dev-knobs を除く macOS だけ
        assert!(matches!(product_launcher(&slot), HostLauncher::Direct));
        assert!(!product_uses_cage());
    }

    #[test]
    fn hello_reports_no_cage_without_an_endpoint() {
        let slot = CageSlot::default();
        assert_eq!(cage_report(true, &slot), pb::LmCage::None);
        // 確かめに失敗した口は入らず、「檻なし」のまま
        let engine_slot = Arc::new(CageSlot::default());
        assert!(engine_slot.attach_unverified(Arc::new(Forged)).is_err());
        assert_eq!(cage_report(true, &engine_slot), pb::LmCage::None);
        // 確かめた口が入れば段 2
        engine_slot.attach_unverified(Arc::new(Good)).unwrap();
        assert_eq!(cage_report(true, &engine_slot), pb::LmCage::Level2);
        // 口が切れたら、また「檻なし」
        engine_slot.detach();
        assert_eq!(cage_report(true, &engine_slot), pb::LmCage::None);
        // 檻役を使わないビルドは、口の有無に関わらず未指定
        assert_eq!(cage_report(false, &engine_slot), pb::LmCage::Unspecified);
    }

    #[test]
    fn caged_builds_point_the_host_into_the_cage_bundle() {
        let exe = Path::new(
            "/A/ikari.app/Contents/XPCServices/IkariEngine.xpc/Contents/MacOS/IkariEngine",
        );
        let want = Path::new("/A/ikari.app/Contents/XPCServices")
            .join(CAGE_SERVICE_BUNDLE)
            .join("Contents/MacOS")
            .join(lm_host_exe_name());
        assert_eq!(caged_host_program_with(true, exe), Some(want));
        assert_eq!(caged_host_program_with(false, exe), None);
        assert_eq!(caged_host_program_with(true, Path::new("/x")), None);
        // このテストのビルドは檻役を使わない（隣を探す）
        assert_eq!(caged_host_program(exe), None);
    }

    #[test]
    fn caged_builds_do_not_load_until_the_cage_arrives() {
        let slot = CageSlot::default();
        assert!(waiting_for_cage_with(true, &slot));
        assert!(!waiting_for_cage_with(false, &slot));
        slot.attach_unverified(Arc::new(Good)).unwrap();
        assert!(!waiting_for_cage_with(true, &slot));
        assert!(!waiting_for_cage(&CageSlot::default()));
    }

    #[test]
    fn user_models_need_the_cage_on_macos_only() {
        let slot = CageSlot::default();
        assert!(!user_model_usable_with(true, &slot));
        assert!(user_model_usable_with(false, &slot));
        slot.attach_unverified(Arc::new(Good)).unwrap();
        assert!(user_model_usable_with(true, &slot));
    }
}
