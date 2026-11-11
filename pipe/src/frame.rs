//! 枠（ADR 0040 §5）。
//!
//! ```text
//! u32 LE  len    以降のバイト数（種類 + 通し番号 + 本体）。MAX_FRAME_LEN 以下
//! u8      kind   FrameKind
//! u64 LE  seq    クライアントが付ける。Reply と Error は対応する要求の番号を返す
//! [u8]    body   Envelope のバイト列（中は見ない）
//! ```

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
    pub fn from_u8(_v: u8) -> Option<FrameKind> {
        todo!()
    }

    pub fn as_u8(self) -> u8 {
        todo!()
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

/// 枠を 1 回の `write_all` で書き、flush する。
pub fn write_frame<W: Write>(_w: &mut W, _frame: &Frame) -> Result<(), FrameError> {
    todo!()
}

/// 枠を 1 つ読む。枠の境目での EOF（相手が閉じた）は `Ok(None)`。枠の途中の EOF は `Io(UnexpectedEof)`。
pub fn read_frame<R: Read>(_r: &mut R) -> Result<Option<Frame>, FrameError> {
    todo!()
}
