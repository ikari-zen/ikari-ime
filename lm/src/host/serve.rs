//! 子（`ikari-lm-host`）の側の繰り返し。標準入力の要求を読み、採点して、標準出力に返す。
//!
//! 取り消し（ADR 0074 §3.1）: 読み取りのスレッドが `Cancel { id }` を受けたら、「ここまで取り消し済み」の番号を上げ、
//! 今の採点がその番号以下なら印を立てる（枠を本体に回さない）。本体は、要求を取り出したときと、推論の止めどころで見る。

use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, mpsc};
use std::thread;
use std::time::Duration;

use crate::cancel::CancelToken;
use crate::error::LmError;
use crate::host::proto::{FailKind, PROTOCOL_VERSION, Reply, Request, read_frame, write_frame};
use crate::placement::GpuState;
use crate::scorer::{LmDevice, LmScorer};

const READER_THREAD_NAME: &str = "ikari-lm-host-stdin";
/// 入力が閉じてから `on_input_end` を呼ぶまでの猶予。ふつうは、この間に本体の繰り返しが自分で終わる。
/// 推論の中で固まっているときだけ、`on_input_end`（プロセスを終わらせる）が効く。計測で決めた値ではない
const INPUT_END_GRACE: Duration = Duration::from_secs(2);

/// 読み込みの結果として親に知らせるもの（`Reply::Ready` の中身）
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadyInfo {
    pub device: LmDevice,
    pub gpu: GpuState,
    pub devices: String,
}

/// 読み取りのスレッドと本体が共有する、取り消しの状態
#[derive(Default)]
struct Cancels {
    /// この番号以下の要求は取り消し済み（0 = なし。親の番号は 1 から）
    upto: AtomicU64,
    /// 今の採点の番号と印
    current: Mutex<Option<(u64, CancelToken)>>,
}

impl Cancels {
    fn current(&self) -> MutexGuard<'_, Option<(u64, CancelToken)>> {
        self.current.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 読み取りのスレッドから: `id` とそれより古い要求を取り消す
    fn cancel_upto(&self, id: u64) {
        self.upto.fetch_max(id, Ordering::AcqRel);
        if let Some((running, token)) = self.current().as_ref()
            && *running <= id
        {
            token.cancel();
        }
    }

    fn is_cancelled(&self, id: u64) -> bool {
        id <= self.upto.load(Ordering::Acquire)
    }

    /// 本体から: 採点を始める。始める前に取り消されていれば立った印を返す
    fn begin(&self, id: u64) -> CancelToken {
        let token = CancelToken::new();
        *self.current() = Some((id, token.clone()));
        // 番号の更新（読み取り）→ 今の採点の確認、の順と逆にして、どちらかが必ず気づく
        if self.is_cancelled(id) {
            token.cancel();
        }
        token
    }

    fn end(&self) {
        *self.current() = None;
    }
}

/// 要求を読んで採点し、返す。入力が閉じる（親が終わった・子を手放した）か、出力に書けなくなったら戻る。
///
/// - `load`: モデルを読み込む。失敗したら `Failed(Load)` を 1 枚返して戻る。
/// - `on_input_end`: 入力が閉じて [`INPUT_END_GRACE`] が経ったときに、読み取りのスレッドから呼ぶ。
///   実物はここでプロセスを終わらせる（親が死んだのに、推論の中で固まって残るのを防ぐ）。
///
/// 入力は、読み込みの前から別スレッドで読む（読み込みに時間がかかっている間に親が死んでも気づける）。
pub fn serve<R, W, L, E>(input: R, mut output: W, load: L, on_input_end: E) -> io::Result<()>
where
    R: Read + Send + 'static,
    W: Write,
    L: FnOnce() -> Result<(Box<dyn LmScorer>, ReadyInfo), LmError>,
    E: FnOnce() + Send + 'static,
{
    // 本体に回すのは採点の要求だけ。None は読めない枠（本体が終わる）
    let (tx, rx) = mpsc::channel::<Option<Request>>();
    let cancels = Arc::new(Cancels::default());
    let reader_cancels = Arc::clone(&cancels);
    thread::Builder::new()
        .name(READER_THREAD_NAME.to_owned())
        .spawn(move || {
            let mut input = input;
            while let Ok(Some(frame)) = read_frame(&mut input) {
                let request = match Request::decode(&frame) {
                    Ok(Request::Cancel { id }) => {
                        reader_cancels.cancel_upto(id);
                        continue;
                    }
                    Ok(r) => Some(r),
                    Err(_) => None,
                };
                if tx.send(request).is_err() {
                    return;
                }
            }
            drop(tx);
            thread::sleep(INPUT_END_GRACE);
            on_input_end();
        })?;

    let mut scorer = match load() {
        Ok((scorer, info)) => {
            let ready = Reply::Ready {
                version: PROTOCOL_VERSION,
                device: info.device,
                gpu: info.gpu,
                devices: info.devices,
            };
            write_frame(&mut output, &ready.encode())?;
            scorer
        }
        Err(e) => {
            let failed = Reply::Failed {
                id: 0,
                kind: FailKind::Load,
                message: e.to_string(),
            };
            return write_frame(&mut output, &failed.encode());
        }
    };

    for request in rx {
        // 読めない要求は、親が壊れている。続けずに終わる（親は子がいなくなったと見て、起動し直す）
        let Some(Request::Score {
            id,
            prompt,
            candidates,
        }) = request
        else {
            return Err(io::ErrorKind::InvalidData.into());
        };
        let cancelled = || Reply::Failed {
            id,
            kind: FailKind::Cancelled,
            message: String::new(),
        };
        // 取り出したときに、もう取り消されていれば推論しない
        if cancels.is_cancelled(id) {
            write_frame(&mut output, &cancelled().encode())?;
            continue;
        }
        let token = cancels.begin(id);
        let result = scorer.nll(&prompt, &candidates, &token);
        cancels.end();
        let reply = match result {
            Ok(nll) if nll.len() == candidates.len() => Reply::Scores { id, nll },
            Err(LmError::Cancelled) => cancelled(),
            Ok(_) => Reply::Failed {
                id,
                kind: FailKind::Inference,
                message: "wrong number of scores".to_owned(),
            },
            Err(e) => Reply::Failed {
                id,
                kind: FailKind::Inference,
                message: e.to_string(),
            },
        };
        write_frame(&mut output, &reply.encode())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    struct LenScorer;

    impl LmScorer for LenScorer {
        fn nll(
            &mut self,
            prompt: &str,
            candidates: &[String],
            cancel: &CancelToken,
        ) -> Result<Vec<f32>, LmError> {
            if cancel.is_cancelled() {
                return Err(LmError::Cancelled);
            }
            if prompt == "fail" {
                return Err(LmError::Inference("no".to_owned()));
            }
            Ok(candidates
                .iter()
                .map(|c| c.chars().count() as f32)
                .collect())
        }
    }

    fn info() -> ReadyInfo {
        ReadyInfo {
            device: LmDevice::Cpu,
            gpu: GpuState::NotBuilt,
            devices: "cpu".to_owned(),
        }
    }

    fn requests(list: &[(u64, &str, &[&str])]) -> Vec<u8> {
        let mut bytes = Vec::new();
        for (id, prompt, cands) in list {
            let r = Request::Score {
                id: *id,
                prompt: (*prompt).to_owned(),
                candidates: cands.iter().map(|c| (*c).to_owned()).collect(),
            };
            write_frame(&mut bytes, &r.encode()).unwrap();
        }
        bytes
    }

    fn replies(mut bytes: &[u8]) -> Vec<Reply> {
        let mut out = Vec::new();
        while let Some(frame) = read_frame(&mut bytes).unwrap() {
            out.push(Reply::decode(&frame).unwrap());
        }
        out
    }

    #[test]
    fn answers_each_request_then_returns_when_input_ends() {
        let input = requests(&[
            (1, "p", &["今日", "京"]),
            (2, "fail", &["x"]),
            (3, "p", &[]),
        ]);
        let mut output = Vec::new();
        serve(
            io::Cursor::new(input),
            &mut output,
            || Ok((Box::new(LenScorer) as Box<dyn LmScorer>, info())),
            || {},
        )
        .unwrap();
        let got = replies(&output);
        assert_eq!(got.len(), 4);
        assert!(matches!(
            got[0],
            Reply::Ready {
                version: PROTOCOL_VERSION,
                device: LmDevice::Cpu,
                ..
            }
        ));
        assert_eq!(
            got[1],
            Reply::Scores {
                id: 1,
                nll: vec![2.0, 1.0]
            }
        );
        assert!(matches!(
            got[2],
            Reply::Failed {
                id: 2,
                kind: FailKind::Inference,
                ..
            }
        ));
        assert_eq!(got[3], Reply::Scores { id: 3, nll: vec![] });
    }

    #[test]
    fn a_cancelled_request_gets_one_cancelled_reply_and_newer_ones_are_scored() {
        let mut input = Vec::new();
        write_frame(&mut input, &Request::Cancel { id: 2 }.encode()).unwrap();
        input.extend(requests(&[
            (1, "p", &["a"]),
            (2, "p", &["a"]),
            (3, "p", &["あい"]),
        ]));
        let mut output = Vec::new();
        serve(
            io::Cursor::new(input),
            &mut output,
            || Ok((Box::new(LenScorer) as Box<dyn LmScorer>, info())),
            || {},
        )
        .unwrap();
        let got = replies(&output);
        assert_eq!(got.len(), 4, "{got:?}");
        for (reply, want) in got[1..3].iter().zip([1, 2]) {
            assert!(
                matches!(reply, Reply::Failed { id, kind: FailKind::Cancelled, .. } if *id == want),
                "{reply:?}"
            );
        }
        assert_eq!(
            got[3],
            Reply::Scores {
                id: 3,
                nll: vec![2.0]
            }
        );
    }

    #[test]
    fn load_failure_is_reported_instead_of_ready() {
        let mut output = Vec::new();
        serve(
            io::Cursor::new(Vec::new()),
            &mut output,
            || Err(LmError::Load("broken".to_owned())),
            || {},
        )
        .unwrap();
        let got = replies(&output);
        assert!(matches!(
            got.as_slice(),
            [Reply::Failed {
                id: 0,
                kind: FailKind::Load,
                ..
            }]
        ));
    }

    #[test]
    fn a_malformed_request_ends_the_loop() {
        let mut input = Vec::new();
        write_frame(&mut input, &[0x7f, 1, 2]).unwrap();
        let mut output = Vec::new();
        let result = serve(
            io::Cursor::new(input),
            &mut output,
            || Ok((Box::new(LenScorer) as Box<dyn LmScorer>, info())),
            || {},
        );
        assert!(result.is_err());
    }

    #[test]
    fn input_end_calls_the_hook_after_the_grace() {
        let called = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&called);
        serve(
            io::Cursor::new(Vec::new()),
            Vec::new(),
            || Ok((Box::new(LenScorer) as Box<dyn LmScorer>, info())),
            move || flag.store(true, Ordering::SeqCst),
        )
        .unwrap();
        // 本体は猶予を待たずに戻る。フックは猶予のあとに呼ばれる
        assert!(!called.load(Ordering::SeqCst));
        thread::sleep(INPUT_END_GRACE + Duration::from_millis(500));
        assert!(called.load(Ordering::SeqCst));
    }
}
