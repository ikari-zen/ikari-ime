//! エンジンと `ikari-lm-host` の間の取り決め（ADR 0060 Q4）。標準入出力のパイプに、枠つきのバイト列を流す。
//!
//! - 枠: 長さ（u32、リトルエンディアン）+ 中身。中身は [`MAX_FRAME_BYTES`] まで。超えたら相手が壊れたと見なす。
//! - 親 → 子（stdin）: [`Request`]。**親が stdin を閉じたら、子は終わる**（終了の合図は別に持たない）。
//! - 子 → 親（stdout）: [`Reply`]。最初の 1 枚は `Ready`（読み込みが済んだ）か `Failed`（読み込めなかった）。
//!   以降は、要求 1 つにつき、同じ `id` の `Scores` か `Failed` を 1 枚。
//! - 文字列は長さ（u32）+ UTF-8。プロンプトは改行やタブを含みうるので、行では区切らない。
//! - NLL は f32 のビット列のまま運ぶ（同じプロセスで採点したときと値が変わらない。無限大も通る）。
//! - 版（[`PROTOCOL_VERSION`]）は `Ready` に入れる。違えば親は子を使わない。子はエンジンと同じ場所に置き、
//!   一緒に入れ替える前提（別々に更新しない）。

use std::io::{self, Read, Write};

use crate::placement::GpuState;
use crate::scorer::LmDevice;

/// 取り決めの版。枠や欄の意味を変えたら上げる
pub const PROTOCOL_VERSION: u16 = 1;
/// 枠の中身の上限（バイト）。プロンプト（左文脈 40 字ほど + 読み 256 字まで）と候補 32 個が十分に入る
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;

const TAG_SCORE: u8 = 0x01;
const TAG_READY: u8 = 0x81;
const TAG_SCORES: u8 = 0x82;
const TAG_FAILED: u8 = 0x83;

const DEVICE_UNKNOWN: u8 = 0;
const DEVICE_CPU: u8 = 1;
const DEVICE_GPU: u8 = 2;

const FAIL_LOAD: u8 = 1;
const FAIL_INFERENCE: u8 = 2;

/// 失敗の文面の上限（バイト）。超えた分は切る
const MAX_MESSAGE_BYTES: usize = 512;

/// 親 → 子
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    /// `prompt` の続きとして各候補を出す確率の負の対数を求める（`LmScorer::nll`）
    Score {
        id: u64,
        prompt: String,
        candidates: Vec<String>,
    },
}

/// 失敗の種類
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailKind {
    /// モデルを読み込めなかった（`Ready` の代わりに 1 枚だけ来る。`id` は 0）
    Load,
    /// この要求の採点に失敗した（子は生きていて、次の要求を受ける）
    Inference,
}

/// 子 → 親
#[derive(Clone, Debug, PartialEq)]
pub enum Reply {
    /// 読み込みが済んだ。装置と GPU の扱い（`lm_load` のログの語。ADR 0059 §2.3）を運ぶ
    Ready {
        version: u16,
        device: LmDevice,
        gpu: GpuState,
        /// 登録されたデバイスの要約（`cpu,vulkan:IntegratedGpu`）
        devices: String,
    },
    /// 要求 `id` の結果。長さは候補の数と同じ
    Scores { id: u64, nll: Vec<f32> },
    Failed {
        id: u64,
        kind: FailKind,
        /// 説明。エンジンはログに書かない（パスを含みうる。NFR-19）
        message: String,
    },
}

/// 中身を読めなかった
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Malformed(pub &'static str);

impl std::fmt::Display for Malformed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "malformed lm-host frame: {}", self.0)
    }
}

impl std::error::Error for Malformed {}

fn put_str(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u32).to_le_bytes());
    out.extend_from_slice(s.as_bytes());
}

fn truncate_message(s: &str) -> &str {
    if s.len() <= MAX_MESSAGE_BYTES {
        return s;
    }
    let mut end = MAX_MESSAGE_BYTES;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

struct Cursor<'a> {
    rest: &'a [u8],
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], Malformed> {
        if self.rest.len() < n {
            return Err(Malformed("truncated"));
        }
        let (head, tail) = self.rest.split_at(n);
        self.rest = tail;
        Ok(head)
    }

    fn u8(&mut self) -> Result<u8, Malformed> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, Malformed> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Result<u32, Malformed> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64(&mut self) -> Result<u64, Malformed> {
        let b = self.take(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        Ok(u64::from_le_bytes(a))
    }

    fn string(&mut self) -> Result<String, Malformed> {
        let n = self.u32()? as usize;
        let bytes = self.take(n)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| Malformed("not utf-8"))
    }

    /// 残りに入りうる数か（1 件が `each` バイト以上。壊れた数で大きな確保をしない）
    fn count(&mut self, each: usize) -> Result<usize, Malformed> {
        let n = self.u32()? as usize;
        if n > self.rest.len() / each {
            return Err(Malformed("count exceeds frame"));
        }
        Ok(n)
    }

    fn end(&self) -> Result<(), Malformed> {
        if self.rest.is_empty() {
            Ok(())
        } else {
            Err(Malformed("trailing bytes"))
        }
    }
}

impl Request {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        match self {
            Request::Score {
                id,
                prompt,
                candidates,
            } => {
                out.push(TAG_SCORE);
                out.extend_from_slice(&id.to_le_bytes());
                put_str(&mut out, prompt);
                out.extend_from_slice(&(candidates.len() as u32).to_le_bytes());
                for c in candidates {
                    put_str(&mut out, c);
                }
            }
        }
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Request, Malformed> {
        let mut c = Cursor { rest: bytes };
        match c.u8()? {
            TAG_SCORE => {
                let id = c.u64()?;
                let prompt = c.string()?;
                let n = c.count(4)?;
                let mut candidates = Vec::with_capacity(n);
                for _ in 0..n {
                    candidates.push(c.string()?);
                }
                c.end()?;
                Ok(Request::Score {
                    id,
                    prompt,
                    candidates,
                })
            }
            _ => Err(Malformed("unknown request tag")),
        }
    }
}

impl Reply {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        match self {
            Reply::Ready {
                version,
                device,
                gpu,
                devices,
            } => {
                out.push(TAG_READY);
                out.extend_from_slice(&version.to_le_bytes());
                out.push(match device {
                    LmDevice::Unknown => DEVICE_UNKNOWN,
                    LmDevice::Cpu => DEVICE_CPU,
                    LmDevice::Gpu => DEVICE_GPU,
                });
                put_str(&mut out, gpu.word());
                put_str(&mut out, truncate_message(devices));
            }
            Reply::Scores { id, nll } => {
                out.push(TAG_SCORES);
                out.extend_from_slice(&id.to_le_bytes());
                out.extend_from_slice(&(nll.len() as u32).to_le_bytes());
                for v in nll {
                    out.extend_from_slice(&v.to_bits().to_le_bytes());
                }
            }
            Reply::Failed { id, kind, message } => {
                out.push(TAG_FAILED);
                out.extend_from_slice(&id.to_le_bytes());
                out.push(match kind {
                    FailKind::Load => FAIL_LOAD,
                    FailKind::Inference => FAIL_INFERENCE,
                });
                put_str(&mut out, truncate_message(message));
            }
        }
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Reply, Malformed> {
        let mut c = Cursor { rest: bytes };
        match c.u8()? {
            TAG_READY => {
                let version = c.u16()?;
                let device = match c.u8()? {
                    DEVICE_CPU => LmDevice::Cpu,
                    DEVICE_GPU => LmDevice::Gpu,
                    DEVICE_UNKNOWN => LmDevice::Unknown,
                    _ => return Err(Malformed("unknown device")),
                };
                // 版が違う相手の語は読めなくてよい（版の照合は呼び出し側がする）
                let gpu = GpuState::parse(&c.string()?).unwrap_or(GpuState::Unknown);
                let devices = c.string()?;
                c.end()?;
                Ok(Reply::Ready {
                    version,
                    device,
                    gpu,
                    devices,
                })
            }
            TAG_SCORES => {
                let id = c.u64()?;
                let n = c.count(4)?;
                let mut nll = Vec::with_capacity(n);
                for _ in 0..n {
                    nll.push(f32::from_bits(c.u32()?));
                }
                c.end()?;
                Ok(Reply::Scores { id, nll })
            }
            TAG_FAILED => {
                let id = c.u64()?;
                let kind = match c.u8()? {
                    FAIL_LOAD => FailKind::Load,
                    FAIL_INFERENCE => FailKind::Inference,
                    _ => return Err(Malformed("unknown failure kind")),
                };
                let message = c.string()?;
                c.end()?;
                Ok(Reply::Failed { id, kind, message })
            }
            _ => Err(Malformed("unknown reply tag")),
        }
    }
}

/// 枠を 1 枚書いて flush する
pub fn write_frame(w: &mut impl Write, payload: &[u8]) -> io::Result<()> {
    if payload.len() > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "lm-host frame too large",
        ));
    }
    w.write_all(&(payload.len() as u32).to_le_bytes())?;
    w.write_all(payload)?;
    w.flush()
}

/// 枠を 1 枚読む。枠の境目で流れが終わっていれば `None`（相手が閉じた）
pub fn read_frame(r: &mut impl Read) -> io::Result<Option<Vec<u8>>> {
    let mut len = [0u8; 4];
    let mut filled = 0;
    while filled < len.len() {
        match r.read(&mut len[filled..]) {
            Ok(0) if filled == 0 => return Ok(None),
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => filled += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    let len = u32::from_le_bytes(len) as usize;
    if len > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "lm-host frame too large",
        ));
    }
    let mut payload = vec![0u8; len];
    r.read_exact(&mut payload)?;
    Ok(Some(payload))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn score() -> Request {
        Request::Score {
            id: 7,
            // 改行、タブ、zenz のタグ（U+EE00 台）がそのまま通る
            prompt: "\u{EE02}左の\n文脈\t\u{EE00}よみ\u{EE01}".to_owned(),
            candidates: vec!["今日".to_owned(), String::new(), "京".to_owned()],
        }
    }

    #[test]
    fn request_round_trips() {
        let r = score();
        assert_eq!(Request::decode(&r.encode()), Ok(r));
    }

    #[test]
    fn replies_round_trip_with_exact_floats() {
        let nll = vec![0.1_f32, f32::INFINITY, 12.345_678, -0.0, f32::MIN_POSITIVE];
        let r = Reply::Scores {
            id: u64::MAX,
            nll: nll.clone(),
        };
        let Ok(Reply::Scores { nll: back, .. }) = Reply::decode(&r.encode()) else {
            panic!("decode failed");
        };
        let bits = |v: &[f32]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
        assert_eq!(bits(&back), bits(&nll));

        for r in [
            Reply::Ready {
                version: PROTOCOL_VERSION,
                device: LmDevice::Gpu,
                gpu: GpuState::FallbackLoadFailed,
                devices: "cpu,vulkan:IntegratedGpu".to_owned(),
            },
            Reply::Failed {
                id: 0,
                kind: FailKind::Load,
                message: "読めない".to_owned(),
            },
            Reply::Failed {
                id: 3,
                kind: FailKind::Inference,
                message: String::new(),
            },
        ] {
            assert_eq!(Reply::decode(&r.encode()), Ok(r));
        }
    }

    #[test]
    fn decode_rejects_truncated_trailing_and_unknown() {
        let bytes = score().encode();
        for cut in 0..bytes.len() {
            assert!(Request::decode(&bytes[..cut]).is_err(), "cut at {cut}");
        }
        let mut extra = bytes.clone();
        extra.push(0);
        assert_eq!(Request::decode(&extra), Err(Malformed("trailing bytes")));
        assert!(Request::decode(&[0x7f]).is_err());
        assert!(Reply::decode(&[0x01]).is_err());
        // 候補の数が枠に入りきらない値なら、確保する前に断る
        let mut huge = vec![TAG_SCORES];
        huge.extend_from_slice(&1u64.to_le_bytes());
        huge.extend_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(Reply::decode(&huge), Err(Malformed("count exceeds frame")));
    }

    #[test]
    fn long_messages_are_cut_at_a_char_boundary() {
        let r = Reply::Failed {
            id: 1,
            kind: FailKind::Inference,
            message: "あ".repeat(1000),
        };
        let Ok(Reply::Failed { message, .. }) = Reply::decode(&r.encode()) else {
            panic!("decode failed");
        };
        assert!(message.len() <= MAX_MESSAGE_BYTES);
        assert!(message.chars().all(|c| c == 'あ'));
    }

    #[test]
    fn frames_round_trip_and_report_a_clean_end() {
        let mut buf = Vec::new();
        write_frame(&mut buf, b"abc").unwrap();
        write_frame(&mut buf, b"").unwrap();
        let mut r = buf.as_slice();
        assert_eq!(read_frame(&mut r).unwrap().as_deref(), Some(&b"abc"[..]));
        assert_eq!(read_frame(&mut r).unwrap().as_deref(), Some(&b""[..]));
        assert_eq!(read_frame(&mut r).unwrap(), None);
    }

    #[test]
    fn frames_reject_oversize_and_mid_frame_end() {
        let too_big = ((MAX_FRAME_BYTES + 1) as u32).to_le_bytes();
        assert!(read_frame(&mut too_big.as_slice()).is_err());
        assert!(write_frame(&mut Vec::new(), &vec![0u8; MAX_FRAME_BYTES + 1]).is_err());
        // 長さの途中、中身の途中で終わるのは、きれいな終わりではない
        assert!(read_frame(&mut [1u8, 0].as_slice()).is_err());
        assert!(read_frame(&mut [4u8, 0, 0, 0, 1].as_slice()).is_err());
    }
}
