//! 殻の判断と 1 本の口（ADR 0041 §4.1。レーン L-A2）。
//!
//! 今の `IkariInputController`・`AppContext`・`SettingsStore`・`BridgeServer` の OS に依存しない判断
//! （`selectMode`、`leaveCurrentMode`、`syncSettings`、`AnchorMemory`、`isAvailable`、`nextSessionID`、メニューの命令、タイマー）を持ち、
//! 状態機械（`machine`）、エンジン（`engine`）、設定（`settings`）、Bridge（`bridge`）をつなぐ。
//!
//! **契約**: `Shell::new`、`Shell::dispatch`、`Shell::engine`、`ShellConfig`、`SettingsSource`、`ShellError` は M1-a で固めた。

use std::path::PathBuf;
use std::sync::Arc;

use crate::engine::EngineLink;
use crate::host::{Clock, EngineTransport, TextAccess};
use crate::settings::SettingsError;
use crate::types::{ShellAction, ShellEvent};

/// 設定の読み元。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsSource {
    /// `settings.tsv` のファイル（本番）。`SettingsFileChanged` で読み直す
    File(PathBuf),
    /// 文字列（シナリオ、テスト）
    Inline(String),
}

/// `Shell::new` の設定（ADR 0041 §4.4 の `ikari_shell_new(config, …)` の `config`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellConfig {
    /// 殻の版（エンジンとの `hello` の照合に使う）
    pub shell_version: String,
    pub settings: SettingsSource,
    /// 同梱の資源の置き場所（`keymap/<preset>.tsv` など。macOS は `.app` の `Resources`）
    pub resources_dir: PathBuf,
}

/// `Shell::new` の誤り。
#[derive(Debug)]
pub enum ShellError {
    Settings(SettingsError),
    /// 同梱の資源（キー割り当ての表など）が読めない
    Resources(std::io::Error),
}

impl std::fmt::Display for ShellError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Settings(error) => write!(f, "settings: {error}"),
            Self::Resources(error) => write!(f, "resources: {error}"),
        }
    }
}

impl std::error::Error for ShellError {}

/// 殻の本体。1 つのスレッドから呼ぶ（`Send` だが `Sync` ではない。ADR 0041 §4.2）。
pub struct Shell {
    // M1-a の stub では使わない。L-A2 が使う
    #[allow(dead_code)]
    config: ShellConfig,
    #[allow(dead_code)]
    clock: Arc<dyn Clock>,
    engine: Arc<EngineLink>,
}

impl Shell {
    /// 設定とキー割り当てを読み、エンジンへの口を作る（エンジンにはまだつながない）。
    pub fn new(
        config: ShellConfig,
        transport: Box<dyn EngineTransport>,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, ShellError> {
        let engine = Arc::new(EngineLink::new(transport, Arc::clone(&clock)));
        Ok(Self {
            config,
            clock,
            engine,
        })
    }

    /// OS のイベントを 1 つ処理し、OS 側が順に実行する動作の列を返す。
    /// 周辺テキストと選択は、要るときだけ `text` から同期に取る。
    pub fn dispatch(&mut self, _event: ShellEvent, _text: &mut dyn TextAccess) -> Vec<ShellAction> {
        Vec::new()
    }

    /// エンジンへの口。落ちた通知（`EngineLink::notify_down`）を別のスレッドから呼ぶために、`Arc` を複製して持つ。
    pub fn engine(&self) -> &Arc<EngineLink> {
        &self.engine
    }
}
