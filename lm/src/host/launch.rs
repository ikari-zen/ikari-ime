//! 子の起動の口と、檻の求め（契約。第 4 弾）。
//!
//! 2 つの軸を分けて持つ（レーンを分けるため）:
//! - [`HostCage`]（どの段の檻を求めるか）は、モデルの出どころで決まる。決めるのはエンジンのモデルの解決
//!   （レーン D-4。プリセットは段 1、自由指定は段 2 だけ。ADR 0072 §12.8 の 1）。
//! - [`HostLauncher`]（だれが子を起動するか）は、OS と配置で決まる。決めるのはエンジンの檻役の受け口
//!   （レーン D-3-mac。macOS の製品は檻役、ほかは自分で起動する。ADR 0077 §6）。
//!
//! macOS では、段 2 は「檻役が起動した子」（ADR 0072 §3）。`HostLauncher::Direct` のまま段 2 を求めると、
//! 今までどおり起動しない（失敗に数える）。

use std::sync::Arc;

use crate::host::cage::CageSlot;
use crate::host::scorer::HostCage;

/// だれが子を起動するか
#[derive(Clone, Debug, Default)]
pub enum HostLauncher {
    /// エンジンが自分で起動する（Windows の製品、テスト、開発。macOS の製品では使わない。ADR 0077 §6）
    #[default]
    Direct,
    /// 檻役に頼む（macOS の段 2。口が無ければ「檻なし」で起動しない）
    Cage(Arc<CageSlot>),
}

/// 子の檻（求める段と、起動の口）。`ikari-stack` の `StackOptions::lm_isolation` で渡す
#[derive(Clone, Debug, Default)]
pub struct LmIsolation {
    pub cage: HostCage,
    pub launcher: HostLauncher,
}
