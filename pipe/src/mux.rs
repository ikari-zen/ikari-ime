//! 返事待ちの表（通し番号 → 待っているスレッド）。

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, MutexGuard};

/// 待っているスレッドに渡す結果。
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    Reply(Vec<u8>),
    /// サーバーが `Error` を返した
    Rejected,
    /// 接続が切れた
    Disconnected,
}

#[derive(Default)]
struct State {
    waiting: HashMap<u64, Sender<Outcome>>,
    closed: bool,
}

/// 返事待ちの表。`close_all` のあとは `register` が失敗する（閉じたあとに待ち始めて永久に待つのを防ぐ）。
#[derive(Default)]
pub(crate) struct Pending {
    state: Mutex<State>,
}

impl Pending {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `seq` の返事を待つ受け口を作る。閉じたあとは `None`。
    pub(crate) fn register(&self, seq: u64) -> Option<Receiver<Outcome>> {
        let mut state = self.lock();
        if state.closed {
            return None;
        }
        let (tx, rx) = mpsc::channel();
        state.waiting.insert(seq, tx);
        Some(rx)
    }

    /// `seq` を待つスレッドに結果を渡す。待っていなければ（時間切れで取り消した、知らない番号）`false`。
    pub(crate) fn complete(&self, seq: u64, outcome: Outcome) -> bool {
        let tx = self.lock().waiting.remove(&seq);
        tx.is_some_and(|tx| tx.send(outcome).is_ok())
    }

    /// 待つのをやめる（時間切れ、送れなかった）。
    pub(crate) fn cancel(&self, seq: u64) {
        self.lock().waiting.remove(&seq);
    }

    /// 閉じ、待っているすべてのスレッドに `Disconnected` を渡す。
    pub(crate) fn close_all(&self) {
        let waiting = {
            let mut state = self.lock();
            state.closed = true;
            std::mem::take(&mut state.waiting)
        };
        for (_, tx) in waiting {
            let _ = tx.send(Outcome::Disconnected);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_reaches_only_the_matching_seq() {
        let p = Pending::default();
        let r1 = p.register(1).unwrap();
        let r2 = p.register(2).unwrap();
        assert!(p.complete(2, Outcome::Reply(b"two".to_vec())));
        assert_eq!(r2.recv().unwrap(), Outcome::Reply(b"two".to_vec()));
        assert!(r1.try_recv().is_err());
        assert!(!p.complete(2, Outcome::Rejected), "a seq completes once");
        assert!(!p.complete(99, Outcome::Rejected), "unknown seq is ignored");
    }

    #[test]
    fn cancel_drops_a_late_reply() {
        let p = Pending::default();
        let _rx = p.register(1).unwrap();
        p.cancel(1);
        assert!(!p.complete(1, Outcome::Reply(Vec::new())));
    }

    #[test]
    fn close_all_wakes_everyone_and_refuses_new_waiters() {
        let p = Pending::default();
        let r1 = p.register(1).unwrap();
        let r2 = p.register(2).unwrap();
        p.close_all();
        assert_eq!(r1.recv().unwrap(), Outcome::Disconnected);
        assert_eq!(r2.recv().unwrap(), Outcome::Disconnected);
        assert!(p.register(3).is_none());
    }
}
