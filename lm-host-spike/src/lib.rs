//! スパイク（捨てるコード。マージしない）。ADR 0060 Q4 の確定条件:
//! 推論を担う別プロセスが落ちる・応答しなくなっても、エンジン側が生き残り、LM なしで動き続けられるか。
//!
//! `HostScorer` は `LmScorer` として `ZenzRanker` のワーカーに差し込める。
//! `nll()` が `Err` を返すと、ZenzRanker は結果を得られず、辞書だけの順位のまま返す（既存の挙動）。
//! ここで足しているのは「子の起動、締め切り、回収（kill と wait）、連続失敗での断念」だけ。
//!
//! 数値は仮置き（本決定ではない）:
//! - 応答の締め切り: ZenzRanker の `budget` とは別に、ワーカーが永久に塞がらないための上限として持つ。
//! - 連続失敗の上限 3: 一過性（OOM、1 回のクラッシュ）は再起動で直り、決定的な失敗（モデルが壊れている、
//!   GPU ドライバのバグ）は 3 回で見切れる、というだけの仮置き。計測の根拠はない。

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use ikari_lm::{LmError, LmScorer};

/// 子が読み込みを終えたときに 1 行出す印（host.rs と同じ）
pub const READY_LINE: &str = "READY";
/// 失敗の応答の接頭辞（host.rs と同じ）
pub const ERR_PREFIX: &str = "ERR";

/// 仮置き: 連続でこの回数失敗したら、子の起動を諦める
pub const DEFAULT_MAX_CONSECUTIVE_FAILURES: u32 = 3;
/// 仮置き: 1 回の採点要求を待つ上限
pub const DEFAULT_REQUEST_DEADLINE: Duration = Duration::from_millis(500);
/// 仮置き: 起動から READY までの上限（Windows の初回で 901ms を観測しているので余裕を見る）
pub const DEFAULT_READY_DEADLINE: Duration = Duration::from_secs(5);

#[derive(Clone, Debug)]
pub struct HostConfig {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub request_deadline: Duration,
    pub ready_deadline: Duration,
    pub max_consecutive_failures: u32,
}

impl HostConfig {
    pub fn new(program: impl Into<PathBuf>, args: Vec<String>) -> Self {
        Self {
            program: program.into(),
            args,
            request_deadline: DEFAULT_REQUEST_DEADLINE,
            ready_deadline: DEFAULT_READY_DEADLINE,
            max_consecutive_failures: DEFAULT_MAX_CONSECUTIVE_FAILURES,
        }
    }
}

struct Live {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
}

impl Live {
    /// 子を殺して回収する（zombie を残さない）。kill 済みでも失敗は無視する。
    /// stdin を閉じるので、読み取りスレッドは EOF で終わる。
    fn reap(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub struct HostScorer {
    cfg: HostConfig,
    live: Option<Live>,
    consecutive_failures: u32,
    spawns: u32,
}

impl HostScorer {
    /// 子はここでは起動しない（最初の `nll()` で起動する）。
    pub fn new(cfg: HostConfig) -> Self {
        Self {
            cfg,
            live: None,
            consecutive_failures: 0,
            spawns: 0,
        }
    }

    /// 連続失敗が上限に達し、もう子を起動しない状態か
    pub fn gave_up(&self) -> bool {
        self.consecutive_failures >= self.cfg.max_consecutive_failures
    }

    pub fn consecutive_failures(&self) -> u32 {
        self.consecutive_failures
    }

    /// これまでに子を起動した回数（起動に失敗したものも含む）
    pub fn spawns(&self) -> u32 {
        self.spawns
    }

    /// 今の子の pid（テストで外から kill するため）
    pub fn child_id(&self) -> Option<u32> {
        self.live.as_ref().map(|l| l.child.id())
    }

    fn fail(&mut self, msg: impl Into<String>) -> LmError {
        self.consecutive_failures += 1;
        if let Some(live) = self.live.take() {
            live.reap();
        }
        LmError::Inference(msg.into())
    }

    fn spawn(&mut self) -> Result<Live, String> {
        self.spawns += 1;
        let mut child = Command::new(&self.cfg.program)
            .args(&self.cfg.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("spawn: {e}"))?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            let _ = child.wait();
            return Err("pipes unavailable".to_owned());
        };
        // パイプの読みには締め切りがないので、スレッドで読んでチャネルの recv_timeout で待つ。
        // 子が死ぬと EOF でスレッドは終わり、受信側は Disconnected になる。
        let (tx, rx) = mpsc::channel();
        let spawned = thread::Builder::new()
            .name("ikari-lm-host-reader".to_owned())
            .spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    let Ok(line) = line else { break };
                    if tx.send(line).is_err() {
                        break;
                    }
                }
            });
        let live = Live {
            child,
            stdin,
            lines: rx,
        };
        if spawned.is_err() {
            live.reap();
            return Err("reader thread".to_owned());
        }
        match live.lines.recv_timeout(self.cfg.ready_deadline) {
            Ok(l) if l == READY_LINE => Ok(live),
            Ok(l) => {
                live.reap();
                Err(format!("unexpected first line: {l}"))
            }
            Err(e) => {
                live.reap();
                Err(format!("no READY: {e:?}"))
            }
        }
    }
}

impl LmScorer for HostScorer {
    fn nll(&mut self, prompt: &str, candidates: &[String]) -> Result<Vec<f32>, LmError> {
        if self.gave_up() {
            return Err(LmError::Inference("lm-host given up".to_owned()));
        }
        if self.live.is_none() {
            match self.spawn() {
                Ok(l) => self.live = Some(l),
                Err(e) => return Err(self.fail(e)),
            }
        }
        let Some(live) = self.live.as_mut() else {
            return Err(self.fail("no child"));
        };
        let mut line = prompt.replace(['\t', '\n'], " ");
        for c in candidates {
            line.push('\t');
            line.push_str(&c.replace(['\t', '\n'], " "));
        }
        line.push('\n');
        if let Err(e) = live
            .stdin
            .write_all(line.as_bytes())
            .and_then(|()| live.stdin.flush())
        {
            return Err(self.fail(format!("write: {e}")));
        }
        let reply = match live.lines.recv_timeout(self.cfg.request_deadline) {
            Ok(r) => r,
            Err(RecvTimeoutError::Timeout) => return Err(self.fail("request deadline")),
            Err(RecvTimeoutError::Disconnected) => return Err(self.fail("child exited")),
        };
        if reply.starts_with(ERR_PREFIX) {
            // 子は生きていて、1 件の失敗を返した。子は捨てない（連続失敗には数える）
            self.consecutive_failures += 1;
            return Err(LmError::Inference(reply));
        }
        let parsed: Result<Vec<f32>, _> = reply.split_whitespace().map(str::parse).collect();
        match parsed {
            Ok(v) if v.len() == candidates.len() => {
                self.consecutive_failures = 0;
                Ok(v)
            }
            _ => Err(self.fail("malformed reply")),
        }
    }
}

impl Drop for HostScorer {
    fn drop(&mut self) {
        if let Some(live) = self.live.take() {
            live.reap();
        }
    }
}
