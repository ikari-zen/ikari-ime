//! Windows: 自分（`ikari-data`）を、**低い整合性**の子（`ikari-data transfer …`）として起こす（ADR 0076 §3.2）。
//!
//! 起動の作りは `ikari-sandbox`（止めた状態で作り、Job Object に入れてから動かす。継承するハンドルは標準出力の
//! パイプだけ。整合性を低くする。窓を出さない。実行ファイルは絶対パスで明示し、PATH から探さない。シェルを通さない）。
//! ここで決めるのは、この子への求めだけ:
//! - 整合性は低。Job の中のプロセスは子の 1 つだけ。UI の制限は LM の子と同じ。
//! - 標準入力と標準エラーは渡さない。
//! - 環境は、プロキシの変数と OS が動くのに要るものだけ（`child::filter_env`。ADR 0076 §3.4）。

use std::ffi::OsString;
use std::io::{self, PipeReader};
use std::os::windows::io::{AsHandle, OwnedHandle};
use std::path::Path;

use ikari_sandbox::{Confined, Integrity, JobLimits, Launch, Stdio, environment_block, spawn};

use crate::child::filter_env;

/// `transfer` の子の Job の制限（子そのもの 1 つ。窓を持たない）
const TRANSFER_JOB: JobLimits = JobLimits {
    max_processes: 1,
    ui_restricted: true,
};

/// `program args…` を低い整合性で起こす。子の標準出力の読み取り側を返す。子は、捨てると（Job を閉じると）終わる
pub fn spawn_low(program: &Path, args: &[String]) -> io::Result<(Confined, PipeReader)> {
    let env = environment_block(filter_env(std::env::vars_os()))?;
    let args: Vec<OsString> = args.iter().map(OsString::from).collect();
    // 子が書く側（標準出力）。親の端は継承できない形のまま
    let (parent_out, child_out) = io::pipe()?;
    let child_out = OwnedHandle::from(child_out);
    let child = spawn(&Launch {
        program,
        args: &args,
        environment: Some(&env),
        integrity: Integrity::Low,
        job: TRANSFER_JOB,
        stdio: Stdio {
            output: Some(child_out.as_handle()),
            ..Stdio::default()
        },
    })?;
    // 子の側の端は、親では閉じる（閉じないと、子が終わっても読み取りが終わらない）
    drop(child_out);
    Ok((child, parent_out))
}
