//! パイプの受け手（ADR 0062 §5.3、§7.3）。OS に依存しない（受け口は `pipe::Listener` で受ける。Mac は `pipe::mem` で試す）。
//!
//! - 受け付けは `pipe::Server`（候補窓の口の上限。接続の数、枠の大きさ、枠の途中の締め切り。ADR 0068 §7）。
//!   高い権限の送り手は、名前付きパイプの OS 層が受け付けで断る（ADR 0068 §2 の 5）。
//! - 接続ごとに番号を付け、届いた枠の本体を `CandidateState` に渡す。接続が切れたら（接続のスレッドが終わって
//!   ハンドラが落ちたら）知らせる。
//! - 描く状態が変わったら `RenderSink` を呼ぶ。状態の錠を持ったまま呼ぶので、知らせは `revision` の順に届く
//!   （知らせの中で核を呼び返さないこと。画面は UI のスレッドに戻して描くだけ）。

use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;

use ikari_pipe::{Listener, Port, Server, Shutdown};

use crate::command::RenderState;
use crate::state::{CandidateState, ConnId, Note};

/// 描く状態の知らせ先。
pub type RenderSink = Arc<dyn Fn(&RenderState) + Send + Sync>;
/// 記録の知らせ先（固定の語と番号だけ）。
pub type NoteSink = Arc<dyn Fn(Note) + Send + Sync>;

/// 受け付けのスレッドの名前
const ACCEPT_THREAD_NAME: &str = "ikari-candidate-accept";
/// 最初の接続の番号（0 は使わない）
const FIRST_CONN: ConnId = 1;

struct Shared {
    state: Mutex<CandidateState>,
    render: RenderSink,
    note: Option<NoteSink>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, CandidateState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn apply(&self, f: impl FnOnce(&mut CandidateState) -> crate::state::Outcome) {
        let mut state = self.lock();
        let out = f(&mut state);
        if let (Some(note), Some(sink)) = (out.note, self.note.as_ref()) {
            sink(note);
        }
        if let Some(render) = out.render {
            (self.render)(&render);
        }
    }
}

/// 接続 1 本の持ち物。落ちたら（接続が終わったら）切れたことを知らせる。
struct ConnGuard {
    id: ConnId,
    shared: Arc<Shared>,
}

impl Drop for ConnGuard {
    fn drop(&mut self) {
        let id = self.id;
        self.shared.apply(|s| s.disconnected(id));
    }
}

/// 動いている受け手。`stop` で止める（落としても止める）。
pub struct Running {
    closer: Option<Box<dyn Shutdown>>,
    thread: Option<JoinHandle<io::Result<()>>>,
}

impl Running {
    /// 受け付けを止め、残っている接続を閉じて、スレッドを待つ。
    pub fn stop(mut self) -> io::Result<()> {
        self.stop_inner()
    }

    fn stop_inner(&mut self) -> io::Result<()> {
        if let Some(closer) = self.closer.take() {
            closer.shutdown();
        }
        match self.thread.take() {
            Some(thread) => thread
                .join()
                .unwrap_or_else(|_| Err(io::Error::other("accept thread panicked"))),
            None => Ok(()),
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.stop_inner();
    }
}

/// 受け手を別のスレッドで始める。`closer` は受け口を閉じる手段（`None` なら、受け口が自分で閉じるまで動く）。
pub fn spawn<L>(
    listener: L,
    closer: Option<Box<dyn Shutdown>>,
    render: RenderSink,
    note: Option<NoteSink>,
) -> io::Result<Running>
where
    L: Listener + 'static,
{
    let shared = Arc::new(Shared {
        state: Mutex::new(CandidateState::new()),
        render,
        note,
    });
    let next = Arc::new(AtomicU64::new(FIRST_CONN));
    let thread = std::thread::Builder::new()
        .name(ACCEPT_THREAD_NAME.to_owned())
        .spawn(move || {
            Server::for_port(listener, Port::Candidate).serve(move || {
                let id = next.fetch_add(1, Ordering::Relaxed);
                shared.apply(|s| {
                    s.connected(id);
                    crate::state::Outcome::default()
                });
                let guard = ConnGuard {
                    id,
                    shared: Arc::clone(&shared),
                };
                move |bytes: &[u8]| {
                    guard.shared.apply(|s| s.received(guard.id, bytes));
                    // 片道。返事はしない
                    None
                }
            })
        })?;
    Ok(Running {
        closer,
        thread: Some(thread),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{CandidateCommand, Hide, Show, candidate_command};
    use ikari_pipe::Client;
    use ikari_pipe::mem::listener;
    use ikari_proto::shell::v1::{CandidateItem, CandidatePanelModel, PanelSide, Rect};
    use prost::Message;
    use std::sync::mpsc;
    use std::time::Duration;

    const WAIT: Duration = Duration::from_secs(5);

    fn show_bytes(seq: u64, generation: u64) -> Vec<u8> {
        CandidateCommand {
            seq,
            op: Some(candidate_command::Op::Show(Show {
                generation,
                model: Some(CandidatePanelModel {
                    candidates: vec![CandidateItem {
                        surface: "漢字".to_owned(),
                        ..Default::default()
                    }],
                    ..Default::default()
                }),
                anchor: Some(Rect {
                    x: 10.0,
                    y: 10.0,
                    width: 1.0,
                    height: 20.0,
                }),
                work_area: Some(Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 800.0,
                    height: 600.0,
                }),
                side: PanelSide::Below.into(),
                scale: 1.0,
            })),
        }
        .encode_to_vec()
    }

    fn start() -> (
        Running,
        ikari_pipe::mem::MemConnector,
        mpsc::Receiver<RenderState>,
    ) {
        let (l, connector) = listener();
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let render: RenderSink = Arc::new(move |r: &RenderState| {
            let _ = tx.lock().unwrap().send(r.clone());
        });
        (spawn(l, None, render, None).unwrap(), connector, rx)
    }

    #[test]
    fn a_posted_show_reaches_the_sink_and_disconnect_hides() {
        let (running, connector, rx) = start();
        let client = Client::new(connector.connect().unwrap(), None);
        client.post(&show_bytes(1, 1)).unwrap();
        let r = rx.recv_timeout(WAIT).unwrap();
        assert!(r.visible);
        assert_eq!(r.model.unwrap().candidates[0].surface, "漢字");
        drop(client);
        let r = rx.recv_timeout(WAIT).unwrap();
        assert!(!r.visible);
        drop(connector);
        running.stop().unwrap();
    }

    #[test]
    fn hide_from_another_connection_is_ignored() {
        let (running, connector, rx) = start();
        let a = Client::new(connector.connect().unwrap(), None);
        let b = Client::new(connector.connect().unwrap(), None);
        a.post(&show_bytes(1, 1)).unwrap();
        assert!(rx.recv_timeout(WAIT).unwrap().visible);
        let hide = |seq| {
            CandidateCommand {
                seq,
                op: Some(candidate_command::Op::Hide(Hide { generation: 0 })),
            }
            .encode_to_vec()
        };
        // B の Hide は効かない。A の Hide は効く（どちらが先に届いても、A の分だけが知らせになる）
        b.post(&hide(1)).unwrap();
        a.post(&hide(2)).unwrap();
        let r = rx.recv_timeout(WAIT).unwrap();
        assert!(!r.visible);
        assert_eq!(r.revision, 2);
        drop((a, b));
        drop(connector);
        running.stop().unwrap();
    }
}
