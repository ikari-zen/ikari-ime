//! `ikari-lm-cage`: macOS の LM の檻役（XPC サービス `IkariLMCage.xpc` の実行ファイル。ADR 0077）。
//!
//! 殻が名前の口で匿名の口をもらい、エンジンに渡す。エンジンは匿名の口で、子（`ikari-lm-host`。この束の中）の起動と
//! 止めるを頼む。檻役は判断を持たず、決まった子を決まった引数で起動し、止めて回収するだけ（`ikari_lm::host::cage`）。
//! 子は檻役の App Sandbox を引き継ぐ。macOS 以外では何もせずに終わる。

#[cfg(target_vendor = "apple")]
fn main() {
    ikari_lm::host::cage::run_service()
}

#[cfg(not(target_vendor = "apple"))]
fn main() -> std::process::ExitCode {
    eprintln!("ikari-lm-cage: macOS only");
    std::process::ExitCode::FAILURE
}
