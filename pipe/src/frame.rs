//! 枠（ADR 0040 §5）。
//!
//! ```text
//! u32 LE  len    以降のバイト数（種類 + 通し番号 + 本体）。MAX_FRAME_LEN 以下
//! u8      kind   FrameKind
//! u64 LE  seq    クライアントが付ける。Reply と Error は対応する要求の番号を返す
//! [u8]    body   Envelope のバイト列（中は見ない）
//! ```

use std::fmt;
use std::io::{self, Read, Write};

/// 長さの欄の大きさ（バイト）
pub const LEN_PREFIX_BYTES: usize = 4;
/// 長さの欄に入る、種類と通し番号の大きさ（バイト）
pub const KIND_SEQ_BYTES: usize = 1 + 8;
/// `len` の上限（バイト。種類と通し番号を含む）。16 MiB。
///
/// 通常の要求は `engine::MAX_REQUEST_BYTES`（256 KiB）以下で、返事も候補の 1 ページ程度。16 MiB はその数十倍の余裕で、
/// 壊れた長さの欄や悪意のある相手に、巨大なバッファを確保させない大きさ（TIP は全アプリのプロセスに入る）。
/// 超えた枠は読まずに接続を切る。`ImportData`（`engine::MAX_ADMIN_REQUEST_BYTES` = 64 MiB）は、
/// 設定画面のパイプ（W4-s の契約）で別に決める。
pub const MAX_FRAME_LEN: u32 = 16 * 1024 * 1024;

/// 枠の種類。値は線の上の `u8`（0 は使わない）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum FrameKind {
    /// 返事を待つ要求（クライアント → サーバー）
    Request = 1,
    /// 返事を待たない要求（クライアント → サーバー）
    Post = 2,
    /// `Request` への返事（サーバー → クライアント）
    Reply = 3,
    /// `Request` に返事を作れなかった（サーバー → クライアント。本体は空でよい）
    Error = 4,
}

impl FrameKind {
    pub fn from_u8(v: u8) -> Option<FrameKind> {
        match v {
            1 => Some(FrameKind::Request),
            2 => Some(FrameKind::Post),
            3 => Some(FrameKind::Reply),
            4 => Some(FrameKind::Error),
            _ => None,
        }
    }

    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

/// 枠 1 つ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub kind: FrameKind,
    pub seq: u64,
    pub body: Vec<u8>,
}

/// 枠の読み書きの誤り。`Io` 以外は、流れが壊れた（接続を切る）ことを表す。
#[derive(Debug)]
pub enum FrameError {
    Io(io::Error),
    /// `len` が `MAX_FRAME_LEN` を超えた（読むときは本体を読まない。書くときは何も書かない）
    TooLarge(u64),
    /// `len` が種類と通し番号に足りない、種類が未知
    Malformed(&'static str),
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FrameError::Io(e) => write!(f, "io: {e}"),
            FrameError::TooLarge(n) => write!(f, "frame too large: {n} bytes"),
            FrameError::Malformed(m) => write!(f, "malformed frame: {m}"),
        }
    }
}

impl std::error::Error for FrameError {}

impl From<io::Error> for FrameError {
    fn from(e: io::Error) -> Self {
        FrameError::Io(e)
    }
}

/// 枠を 1 回の `write_all` で書き、flush する。
pub fn write_frame<W: Write>(w: &mut W, frame: &Frame) -> Result<(), FrameError> {
    let len = KIND_SEQ_BYTES as u64 + frame.body.len() as u64;
    if len > u64::from(MAX_FRAME_LEN) {
        return Err(FrameError::TooLarge(len));
    }
    // 上の検査で u32 に収まる
    let len32 = len as u32;
    let mut buf = Vec::with_capacity(LEN_PREFIX_BYTES + len as usize);
    buf.extend_from_slice(&len32.to_le_bytes());
    buf.push(frame.kind.as_u8());
    buf.extend_from_slice(&frame.seq.to_le_bytes());
    buf.extend_from_slice(&frame.body);
    w.write_all(&buf)?;
    w.flush()?;
    Ok(())
}

/// 枠を 1 つ読む。枠の境目での EOF（相手が閉じた）は `Ok(None)`。枠の途中の EOF は `Io(UnexpectedEof)`。
pub fn read_frame<R: Read>(r: &mut R) -> Result<Option<Frame>, FrameError> {
    let mut len_buf = [0u8; LEN_PREFIX_BYTES];
    let mut got = 0;
    while got < LEN_PREFIX_BYTES {
        match r.read(&mut len_buf[got..]) {
            Ok(0) if got == 0 => return Ok(None),
            Ok(0) => return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into()),
            Ok(n) => got += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e.into()),
        }
    }
    let len = u32::from_le_bytes(len_buf);
    if len > MAX_FRAME_LEN {
        return Err(FrameError::TooLarge(u64::from(len)));
    }
    if (len as usize) < KIND_SEQ_BYTES {
        return Err(FrameError::Malformed("length is shorter than kind and seq"));
    }
    // 長さの欄の値だけ先に確保しない（相手が嘘の長さを書いても、届いた分しか使わない）
    let mut payload = Vec::new();
    r.take(u64::from(len)).read_to_end(&mut payload)?;
    if payload.len() != len as usize {
        return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into());
    }
    let kind = FrameKind::from_u8(payload[0]).ok_or(FrameError::Malformed("unknown kind"))?;
    let mut seq_buf = [0u8; 8];
    seq_buf.copy_from_slice(&payload[1..KIND_SEQ_BYTES]);
    let body = payload.split_off(KIND_SEQ_BYTES);
    Ok(Some(Frame {
        kind,
        seq: u64::from_le_bytes(seq_buf),
        body,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(frame: &Frame) -> Vec<u8> {
        let mut v = Vec::new();
        write_frame(&mut v, frame).unwrap();
        v
    }

    fn frame(kind: FrameKind, seq: u64, body: &[u8]) -> Frame {
        Frame {
            kind,
            seq,
            body: body.to_vec(),
        }
    }

    #[test]
    fn wire_layout_is_len_kind_seq_body() {
        let bytes = encode(&frame(FrameKind::Request, 0x0102, b"ab"));
        assert_eq!(
            bytes,
            [11, 0, 0, 0, 1, 0x02, 0x01, 0, 0, 0, 0, 0, 0, b'a', b'b']
        );
    }

    #[test]
    fn round_trips_every_kind_and_empty_body() {
        for kind in [
            FrameKind::Request,
            FrameKind::Post,
            FrameKind::Reply,
            FrameKind::Error,
        ] {
            for body in [&b""[..], b"x", &[0u8; 1000][..]] {
                let f = frame(kind, u64::MAX, body);
                let bytes = encode(&f);
                assert_eq!(read_frame(&mut bytes.as_slice()).unwrap(), Some(f));
            }
        }
    }

    #[test]
    fn reads_consecutive_frames_then_clean_eof() {
        let mut bytes = encode(&frame(FrameKind::Post, 1, b"a"));
        bytes.extend(encode(&frame(FrameKind::Reply, 2, b"bc")));
        let mut r = bytes.as_slice();
        assert_eq!(read_frame(&mut r).unwrap().unwrap().seq, 1);
        assert_eq!(read_frame(&mut r).unwrap().unwrap().seq, 2);
        assert!(read_frame(&mut r).unwrap().is_none());
    }

    #[test]
    fn body_at_the_limit_is_accepted_and_one_more_is_not() {
        let max_body = MAX_FRAME_LEN as usize - KIND_SEQ_BYTES;
        let ok = frame(FrameKind::Request, 1, &vec![7; max_body]);
        let bytes = encode(&ok);
        assert_eq!(read_frame(&mut bytes.as_slice()).unwrap(), Some(ok));

        let too_big = frame(FrameKind::Request, 1, &vec![7; max_body + 1]);
        let mut sink = Vec::new();
        assert!(matches!(
            write_frame(&mut sink, &too_big),
            Err(FrameError::TooLarge(_))
        ));
        assert!(sink.is_empty(), "nothing is written for a too large frame");
    }

    #[test]
    fn oversized_length_is_rejected_without_reading_the_body() {
        let mut bytes = (MAX_FRAME_LEN + 1).to_le_bytes().to_vec();
        bytes.extend([1; 4]);
        assert!(matches!(
            read_frame(&mut bytes.as_slice()),
            Err(FrameError::TooLarge(n)) if n == u64::from(MAX_FRAME_LEN) + 1
        ));
        let huge = u32::MAX.to_le_bytes();
        assert!(matches!(
            read_frame(&mut huge.as_slice()),
            Err(FrameError::TooLarge(_))
        ));
    }

    #[test]
    fn short_length_is_malformed() {
        for len in [0u32, 1, 8] {
            let mut bytes = len.to_le_bytes().to_vec();
            bytes.extend(vec![1; len as usize]);
            assert!(matches!(
                read_frame(&mut bytes.as_slice()),
                Err(FrameError::Malformed(_))
            ));
        }
    }

    #[test]
    fn unknown_kind_is_malformed() {
        for kind in [0u8, 5, 255] {
            let mut bytes = 9u32.to_le_bytes().to_vec();
            bytes.push(kind);
            bytes.extend([0; 8]);
            assert!(matches!(
                read_frame(&mut bytes.as_slice()),
                Err(FrameError::Malformed(_))
            ));
        }
    }

    #[test]
    fn truncated_frames_are_unexpected_eof() {
        let bytes = encode(&frame(FrameKind::Request, 1, b"abcdef"));
        // 長さの欄の途中、長さのあと、本体の途中
        for cut in [1, 3, 4, 8, bytes.len() - 1] {
            match read_frame(&mut &bytes[..cut]) {
                Err(FrameError::Io(e)) => {
                    assert_eq!(e.kind(), io::ErrorKind::UnexpectedEof, "cut {cut}")
                }
                other => panic!("cut {cut}: {other:?}"),
            }
        }
    }
}
