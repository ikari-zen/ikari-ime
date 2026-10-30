//! `check` と `dict` が共有する、リポジトリの場所・ログ・外部コマンドの実行。

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// 失敗。`code` は終了コード（子プロセスの失敗はそのコード、それ以外は 1）。
#[derive(Debug)]
pub struct Failure {
    pub code: i32,
    pub message: String,
}

impl Failure {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            code: 1,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// リポジトリのルート（xtask の 1 つ上）。
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default()
}

pub fn log(message: &str) {
    println!("==> {message}");
}

pub fn warn(message: &str) {
    eprintln!("warning: {message}");
}

/// コマンドを標準入出力を引き継いで実行し、失敗なら子のコードで `Failure` にする。
pub fn run(command: &mut Command) -> Result<(), Failure> {
    let shown = format!("{command:?}");
    let status = command
        .status()
        .map_err(|e| Failure::new(format!("{shown} を起動できません: {e}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(Failure {
            code: status.code().unwrap_or(1),
            message: format!("{shown} が失敗しました（{status}）"),
        })
    }
}

/// 出力を捨てて、成功したか（起動できなければ false）。
pub fn succeeds_quietly(command: &mut Command) -> bool {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// 標準出力を文字列で得る。失敗や起動不可は None。
pub fn capture(command: &mut Command) -> Option<String> {
    let output = command.stderr(Stdio::null()).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// 作業ツリーを root にした git。
pub fn git(root: &Path) -> Command {
    let mut command = Command::new("git");
    command.arg("-C").arg(root);
    command
}

/// `cargo`（root で実行）。
pub fn cargo(root: &Path) -> Command {
    let mut command = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    command.current_dir(root);
    command
}

/// bash が使えるか（Windows の素の環境では使えない。シェルのスクリプトを呼ぶ検査はそのとき飛ばす）。
pub fn has_bash() -> bool {
    succeeds_quietly(Command::new("bash").arg("--version"))
}
