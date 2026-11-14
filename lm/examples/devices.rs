//! 登録された LM のデバイスを出す（ADR 0059 §2.6）。モデルは読まない。
//!
//! 出力: `lm devices=cpu,vulkan:IntegratedGpu`。CPU のデバイスが 1 つも無ければ、終了コード 1。
//! Vulkan のビルドでは、この exe の隣の `backends` フォルダからバックエンドを読む（GPU が無い機械でも CPU は出る）。

use std::process::ExitCode;

fn main() -> ExitCode {
    match ikari_lm::probe_devices() {
        Ok(devices) => {
            println!("lm devices={devices}");
            if devices.split(',').any(|d| d == "cpu") {
                ExitCode::SUCCESS
            } else {
                eprintln!("no CPU device is registered");
                ExitCode::FAILURE
            }
        }
        Err(e) => {
            eprintln!("backend init failed: {e}");
            ExitCode::FAILURE
        }
    }
}
