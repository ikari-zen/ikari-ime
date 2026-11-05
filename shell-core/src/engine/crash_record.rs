//! 異常終了の記録の殻側（ADR 0051 §5.2）。前回の判定（`sh_prev_exit`）と panic の数（`sh_panic`）を持ち、
//! `sh_engine` と同じ相乗りで送る（`EngineLink::diag`）。メモリ上の値だけ。文字は持たない（場所は `runmark` が絞った語）。
//!
//! 送り方の決まり: secure のセッションの事象では `diag` を呼ばないので相乗りしない（NFR-18）。送れなかったら持ち越す。
//! `sh_panic` の `n` は送った分だけ引く（送っている間に増えた分は残す）。

use ikari_proto::v1;
use ikari_runmark::PrevExit;

pub const KIND_PREV_EXIT: &str = "sh_prev_exit";
pub const KIND_PANIC: &str = "sh_panic";

/// `sh_panic` の `thread` の語（許可リストに合わせる）
const THREAD_MAIN: &str = "main";
const THREAD_OTHER: &str = "other";

/// 前回送ってからの panic（最後の場所とスレッドの種類つき）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanicCounts {
    pub n: i64,
    pub loc: String,
    pub main_thread: bool,
}

#[derive(Debug, Default)]
pub struct CrashRecord {
    /// 前回の判定。送るまで持つ（起動ごとに 1 回）
    prev_exit: Option<PrevExit>,
    panics: Option<PanicCounts>,
}

impl CrashRecord {
    pub fn set_prev_exit(&mut self, prev: PrevExit) {
        self.prev_exit = Some(prev);
    }

    /// `thread` は `runmark` の語（`main` 以外は `other` にまとめる）。`loc` は絞り込みずみ
    pub fn note_panic(&mut self, thread: &str, loc: &str) {
        let n = self.panics.as_ref().map_or(0, |p| p.n) + 1;
        self.panics = Some(PanicCounts {
            n,
            loc: loc.to_owned(),
            main_thread: thread == THREAD_MAIN,
        });
    }

    pub fn prev_exit(&self) -> Option<PrevExit> {
        self.prev_exit
    }

    pub fn panics(&self) -> Option<PanicCounts> {
        self.panics.clone()
    }

    pub fn prev_exit_sent(&mut self) {
        self.prev_exit = None;
    }

    /// 送った分を引く。残りがなければ場所も捨てる
    pub fn panics_sent(&mut self, sent: &PanicCounts) {
        if let Some(p) = &mut self.panics {
            p.n -= sent.n;
            if p.n <= 0 {
                self.panics = None;
            }
        }
    }
}

pub fn prev_exit_event(prev: &PrevExit, session_id: u64) -> v1::DiagEvent {
    let mut nums = Vec::new();
    if let Some(sig) = prev.sig {
        nums.push(num("sig", i64::from(sig)));
    }
    if let Some(up_s) = prev.up_s {
        nums.push(num("up_s", i64::try_from(up_s).unwrap_or(i64::MAX)));
    }
    v1::DiagEvent {
        session_id,
        kind: KIND_PREV_EXIT.to_owned(),
        nums,
        tags: vec![tag("exit", prev.exit.as_word())],
    }
}

pub fn panic_event(panics: &PanicCounts, session_id: u64) -> v1::DiagEvent {
    v1::DiagEvent {
        session_id,
        kind: KIND_PANIC.to_owned(),
        nums: vec![num("n", panics.n)],
        tags: vec![
            tag("loc", &panics.loc),
            tag(
                "thread",
                if panics.main_thread {
                    THREAD_MAIN
                } else {
                    THREAD_OTHER
                },
            ),
        ],
    }
}

fn num(key: &str, value: i64) -> v1::DiagNum {
    v1::DiagNum {
        key: key.to_owned(),
        value,
    }
}

fn tag(key: &str, value: &str) -> v1::DiagTag {
    v1::DiagTag {
        key: key.to_owned(),
        value: value.to_owned(),
    }
}
