//! 枠の読み書き（ADR 0005 §6.3。今の Swift の `FrameCodec`、`FrameDecoder`）。
//! 枠 = 4 バイト（ビッグエンディアン）の長さ + `Frame` の protobuf。

use prost::Message;

use super::proto::Frame;
use super::protocol::{LENGTH_PREFIX_BYTES, MAX_FRAME_BYTES};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FrameError {
    /// 長さが `MAX_FRAME_BYTES` を超える（壊れた長さを含む）。読み始める前に断る
    TooLarge { length: u64 },
    /// 長さ 0 の枠。正しい `Frame` は必ず 1 つの本体を持つので、中身のない枠は壊れているとみなす
    Empty,
    /// protobuf として読めない
    Malformed,
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge { length } => write!(f, "bridge frame too large: {length}"),
            Self::Empty => f.write_str("empty bridge frame"),
            Self::Malformed => f.write_str("malformed bridge frame"),
        }
    }
}

impl std::error::Error for FrameError {}

/// 長さの接頭辞をつけた枠のバイト列
pub fn encode(frame: &Frame) -> Result<Vec<u8>, FrameError> {
    let length = frame.encoded_len();
    if length == 0 {
        return Err(FrameError::Empty);
    }
    if length > MAX_FRAME_BYTES {
        return Err(FrameError::TooLarge {
            length: length as u64,
        });
    }
    let prefix = u32::try_from(length).map_err(|_| FrameError::TooLarge {
        length: length as u64,
    })?;
    let mut data = Vec::with_capacity(LENGTH_PREFIX_BYTES + length);
    data.extend_from_slice(&prefix.to_be_bytes());
    frame.encode(&mut data).map_err(|_| FrameError::Malformed)?;
    Ok(data)
}

/// 本体（接頭辞を除いた protobuf）を読む。`ShellEvent::BridgeFrame` の 1 枠を受けるときに使う
pub fn decode_body(body: &[u8]) -> Result<Frame, FrameError> {
    if body.is_empty() {
        return Err(FrameError::Empty);
    }
    if body.len() > MAX_FRAME_BYTES {
        return Err(FrameError::TooLarge {
            length: body.len() as u64,
        });
    }
    Frame::decode(body).map_err(|_| FrameError::Malformed)
}

/// バイト列を少しずつ受け取って枠に切り出す（ソケットの読みは枠の境目と合わない）。
/// 一度でも失敗したら、枠の位置が分からなくなるので、以後は同じ失敗を返し続ける（接続を閉じること）。
#[derive(Debug, Default)]
pub struct FrameDecoder {
    buffer: Vec<u8>,
    failure: Option<FrameError>,
}

impl FrameDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// 受け取ったバイト列を足して、そろった枠をすべて返す。足りない分は次の `feed` まで持ち越す
    pub fn feed(&mut self, chunk: &[u8]) -> Result<Vec<Frame>, FrameError> {
        if let Some(failure) = self.failure {
            return Err(failure);
        }
        self.buffer.extend_from_slice(chunk);
        let mut frames = Vec::new();
        let mut consumed = 0;
        let result = loop {
            match next_frame(&self.buffer[consumed..]) {
                Ok(Some((frame, used))) => {
                    frames.push(frame);
                    consumed += used;
                }
                Ok(None) => break Ok(()),
                Err(error) => break Err(error),
            }
        };
        match result {
            Ok(()) => {
                self.buffer.drain(..consumed);
                Ok(frames)
            }
            Err(error) => {
                self.failure = Some(error);
                self.buffer = Vec::new();
                Err(error)
            }
        }
    }

    /// まだ枠にならずに持っているバイト数
    pub fn buffered_byte_count(&self) -> usize {
        self.buffer.len()
    }
}

/// 先頭の 1 枠と、使ったバイト数。足りなければ `None`
fn next_frame(buffer: &[u8]) -> Result<Option<(Frame, usize)>, FrameError> {
    let Some(prefix) = buffer.first_chunk::<LENGTH_PREFIX_BYTES>() else {
        return Ok(None);
    };
    let length = u64::from(u32::from_be_bytes(*prefix));
    if length > MAX_FRAME_BYTES as u64 {
        return Err(FrameError::TooLarge { length });
    }
    if length == 0 {
        return Err(FrameError::Empty);
    }
    let total = LENGTH_PREFIX_BYTES + length as usize;
    let Some(body) = buffer.get(LENGTH_PREFIX_BYTES..total) else {
        return Ok(None);
    };
    let frame = Frame::decode(body).map_err(|_| FrameError::Malformed)?;
    Ok(Some((frame, total)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::proto::{SettingEntry, UpdateSettings, frame::Body};
    use ikari_proto::v1;

    fn settings_frame(id: u64, key: &str, value: &str) -> Frame {
        Frame {
            request_id: id,
            body: Some(Body::UpdateSettings(UpdateSettings {
                set: vec![SettingEntry {
                    key: key.into(),
                    value: value.into(),
                }],
                reset: vec![],
            })),
        }
    }

    fn frame(id: u64) -> Frame {
        settings_frame(id, "paging.size", "5")
    }

    fn prefix(length: u32) -> Vec<u8> {
        length.to_be_bytes().to_vec()
    }

    #[test]
    fn encodes_a_big_endian_length_followed_by_the_message() {
        let body = frame(1).encode_to_vec();
        let data = encode(&frame(1)).unwrap();
        assert_eq!(data[..4], prefix(body.len() as u32)[..]);
        assert_eq!(data[4..], body[..]);
    }

    #[test]
    fn round_trips_one_frame() {
        let mut decoder = FrameDecoder::new();
        assert_eq!(
            decoder.feed(&encode(&frame(9)).unwrap()).unwrap(),
            [frame(9)]
        );
        assert_eq!(decoder.buffered_byte_count(), 0);
    }

    #[test]
    fn reassembles_a_frame_split_at_every_byte() {
        let data = encode(&frame(3)).unwrap();
        let mut decoder = FrameDecoder::new();
        let mut frames = Vec::new();
        for byte in data {
            frames.extend(decoder.feed(&[byte]).unwrap());
        }
        assert_eq!(frames, [frame(3)]);
    }

    #[test]
    fn splits_several_frames_delivered_in_one_chunk() {
        let first = encode(&frame(1)).unwrap();
        let second = encode(&settings_frame(2, "paging.wrap", "false")).unwrap();
        let third = encode(&frame(3)).unwrap();
        let mut decoder = FrameDecoder::new();
        // 3 つ目は途中までしか来ていない
        let chunk = [first, second, third[..5].to_vec()].concat();
        let ids: Vec<_> = decoder
            .feed(&chunk)
            .unwrap()
            .iter()
            .map(|f| f.request_id)
            .collect();
        assert_eq!(ids, [1, 2]);
        assert_eq!(decoder.buffered_byte_count(), 5);
        let rest: Vec<_> = decoder
            .feed(&third[5..])
            .unwrap()
            .iter()
            .map(|f| f.request_id)
            .collect();
        assert_eq!(rest, [3]);
    }

    #[test]
    fn waits_for_the_rest_when_only_a_part_of_the_length_has_arrived() {
        let mut decoder = FrameDecoder::new();
        assert!(decoder.feed(&[0, 0]).unwrap().is_empty());
        assert_eq!(decoder.buffered_byte_count(), 2);
    }

    #[test]
    fn accepts_a_length_at_the_limit_but_waits_for_the_body() {
        let mut decoder = FrameDecoder::new();
        assert!(
            decoder
                .feed(&prefix(MAX_FRAME_BYTES as u32))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn rejects_a_length_over_the_limit_before_reading_the_body() {
        let mut decoder = FrameDecoder::new();
        let length = MAX_FRAME_BYTES as u32 + 1;
        assert_eq!(
            decoder.feed(&prefix(length)),
            Err(FrameError::TooLarge {
                length: u64::from(length)
            })
        );
    }

    #[test]
    fn rejects_a_broken_length_that_looks_like_a_huge_number() {
        let mut decoder = FrameDecoder::new();
        assert_eq!(
            decoder.feed(&[0xFF; 4]),
            Err(FrameError::TooLarge {
                length: u64::from(u32::MAX)
            })
        );
    }

    #[test]
    fn rejects_a_zero_length_frame() {
        assert_eq!(FrameDecoder::new().feed(&prefix(0)), Err(FrameError::Empty));
    }

    #[test]
    fn rejects_a_body_that_is_not_a_protobuf_message() {
        // 欄 10（hello）の長さ指定が本体より長い
        let garbage = [0x52, 0x7F, 0x01];
        let data = [prefix(garbage.len() as u32), garbage.to_vec()].concat();
        assert_eq!(FrameDecoder::new().feed(&data), Err(FrameError::Malformed));
        assert_eq!(decode_body(&garbage), Err(FrameError::Malformed));
    }

    #[test]
    fn stays_failed_after_the_first_error() {
        let mut decoder = FrameDecoder::new();
        assert_eq!(decoder.feed(&prefix(0)), Err(FrameError::Empty));
        // 枠の位置が分からなくなっているので、正しい枠が来ても読まない
        assert_eq!(
            decoder.feed(&encode(&frame(1)).unwrap()),
            Err(FrameError::Empty)
        );
    }

    #[test]
    fn refuses_to_encode_an_empty_frame() {
        assert_eq!(encode(&Frame::default()), Err(FrameError::Empty));
        assert_eq!(decode_body(&[]), Err(FrameError::Empty));
    }

    #[test]
    fn refuses_to_encode_a_body_over_the_limit() {
        let envelope = v1::Envelope {
            body: Some(v1::envelope::Body::ExportReply(v1::ExportReply {
                content: vec![0; MAX_FRAME_BYTES + 1],
                ..Default::default()
            })),
            ..Default::default()
        };
        let frame = Frame {
            request_id: 0,
            body: Some(Body::Engine(envelope)),
        };
        assert!(matches!(encode(&frame), Err(FrameError::TooLarge { .. })));
    }

    #[test]
    fn carries_an_engine_envelope_unchanged() {
        let envelope = v1::Envelope {
            schema_version: 1,
            request_id: 77,
            body: Some(v1::envelope::Body::ImportData(v1::ImportData {
                content: (0..1000).map(|i| (i % 251) as u8).collect(),
                dry_run: true,
            })),
        };
        let frame = Frame {
            request_id: 5,
            body: Some(Body::Engine(envelope)),
        };
        let data = encode(&frame).unwrap();
        assert_eq!(FrameDecoder::new().feed(&data).unwrap(), std::slice::from_ref(&frame));
        assert_eq!(decode_body(&data[4..]).unwrap(), frame);
    }
}
