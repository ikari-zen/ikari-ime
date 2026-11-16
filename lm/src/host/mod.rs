//! LM の推論を別プロセス `ikari-lm-host` に頼む（ADR 0060 Q4）。
//!
//! - 親（エンジン）の側: [`host_loader`] が `ZenzRanker` に渡す `LmLoader` を作る。呼び出し側は、同じプロセスで
//!   動かすとき（`LlamaScorer::loader_with`）と同じ形で使う。子の起動、締め切り、回収（kill と wait）、
//!   連続失敗での断念は、この中で持つ。断念はメモリだけで覚える（エンジンを起動し直せば、もう一度試す）。
//! - 子の側: [`serve`] が、標準入力の要求を読んで採点し、標準出力に返す。
//! - 取り決め（枠とメッセージ）は [`proto`]。
//!
//! 子が落ちる・固まる・壊れた返事をするときは、`LmScorer::nll` が `Err` を返し、`ZenzRanker` は何もせずに戻る
//! （候補は辞書から作ったまま。LM は並べ替えにだけ使う）。

mod os;
pub mod proto;
mod scorer;
mod serve;

pub use scorer::{HostCage, HostHealth, HostLimits, HostSpec, host_loader};
pub use serve::{ReadyInfo, serve};

/// 子の実行ファイルの名前（拡張子なし）。エンジンの実行ファイルと同じフォルダに置く
pub const HOST_EXE_STEM: &str = "ikari-lm-host";

/// 子の引数: モデルのパス
pub const ARG_MODEL: &str = "--model";
/// 子の引数: GPU の扱い。[`ARG_GPU_ON`] / [`ARG_GPU_OFF`] は設定 `engine.lm_gpu`（同じプロセスで読むときと同じ意味。
/// Metal のビルドは `off` でも GPU に載せる）。[`ARG_GPU_CPU`] は、ビルドに関わらず CPU だけで読む
/// （GPU の子が失敗し続けたあとの試し直し）
pub const ARG_GPU: &str = "--gpu";
pub const ARG_GPU_ON: &str = "on";
pub const ARG_GPU_OFF: &str = "off";
pub const ARG_GPU_CPU: &str = "cpu";

/// LM の子の檻の段 2（AppContainer）のプロファイルの名前（利用者ごとに 1 つ。インストーラの helper も同じ名前で消す）
pub const CAGE_PROFILE_NAME: &str = "io.github.ikari-zen.lm-host";

/// モデルのファイルから、段 2 の子への読み取りの許可を外す（Windows。モデルを置き換えるとき・消すときにエンジンが
/// 呼ぶ。許可が無くても成功。ADR 0072 §12.8）。ほかの OS では何もしない
pub fn revoke_cage_read(model: &std::path::Path) -> std::io::Result<()> {
    os::revoke_cage_read(model)
}

/// LM の子の檻の段 2（AppContainer）のプロファイルを消す（Windows。アンインストールのとき。無くても成功）。
/// プロファイルは利用者ごとにあり、消えるのは呼んだ利用者のものだけ（ADR 0072 §12）
#[cfg(windows)]
pub fn delete_cage_profile() -> std::io::Result<()> {
    os::delete_cage_profile()
}

/// 子の実行ファイルの名前（Windows は `.exe` つき）
pub fn host_exe_name() -> String {
    format!("{HOST_EXE_STEM}{}", std::env::consts::EXE_SUFFIX)
}
