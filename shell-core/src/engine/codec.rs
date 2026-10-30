//! `Envelope` の組み立てと読み取り、要求の検証、返事の検証（今の Swift の `EnvelopeCodec`、`PostalLookupCodec`、
//! `EmojiTableCodec`。ADR 0004 §5.2「信用しない入力」）。
//!
//! 文字数は Unicode のスカラー値（`char`）で数える。Swift の `String.count`（書記素クラスタ）とは、結合文字や
//! 絵文字の連なりでだけ違う（Rust のほうが多く数えるので、上限の判定は少し厳しくなる）。

use ikari_proto::v1::{self, envelope::Body};
use prost::Message;

use crate::diag::DiagEvent;
use crate::menu::EngineLmState;
use crate::types::{EngineError, EngineErrorCode};

/// `Envelope.schema_version`（proto の `SCHEMA_VERSION`）
pub const SCHEMA_VERSION: u32 = ikari_proto::SCHEMA_VERSION;
/// 返事の検証の上限（エンジンの上限より緩くし、壊れた返事だけを弾く）
pub const MAX_CANDIDATES: usize = 64;
pub const MAX_TEXT_CHARACTERS: usize = 512;
pub const MAX_REPLY_BYTES: usize = 1 << 20;
/// 読みの上限（文字数）
pub const MAX_READING_CHARACTERS: usize = 256;
/// 候補の 1 ページの数の範囲
pub const PAGE_SIZE_RANGE: std::ops::RangeInclusive<u32> = 1..=9;
/// 左文脈は末尾のこの文字数に切る（今の `SurroundingText.maxCharacters`）
pub const MAX_CONTEXT_CHARACTERS: usize = 2000;
/// 文節の数の上限（読みの上限 256 文字と同じ。1 文字ずつの文節が最大）
pub const MAX_SEGMENTS: usize = 256;
/// `SegmentChoice.literal` の上限（文字数）
pub const MAX_LITERAL_CHARACTERS: usize = 256;
/// 再変換の対象の文字数の上限（ADR 0006 §9.2）
pub const MAX_RECONVERT_CHARACTERS: usize = 64;
/// 番号の上限（Swift の `Int32.max`。エンジンの側と合わせる）
pub const MAX_INDEX: u32 = i32::MAX as u32;
/// 郵便番号の返事の上限（ADR 0032 §9.1）
pub const MAX_POSTAL_ADDRESSES: usize = 256;
pub const MAX_POSTAL_ADDRESS_CHARACTERS: usize = 256;
/// 絵文字の表の上限（ADR 0027 §9.2）
pub const MAX_EMOJI_ENTRIES: usize = 8_192;
pub const MAX_EMOJI_NAME_CHARACTERS: usize = 64;
pub const MAX_EMOJI_SCALARS: usize = 32;

fn chars(text: &str) -> usize {
    text.chars().count()
}

// ---------------------------------------------------------------- 要求

pub(crate) fn encode(request_id: u64, body: Body) -> Vec<u8> {
    v1::Envelope {
        schema_version: SCHEMA_VERSION,
        request_id,
        body: Some(body),
    }
    .encode_to_vec()
}

/// 左文脈を末尾 `MAX_CONTEXT_CHARACTERS` 文字に切る（殻の責任だが、ここでも守る）
pub(crate) fn clipped(context: Option<v1::SurroundingText>) -> Option<v1::SurroundingText> {
    context.map(|mut text| {
        let count = chars(&text.left);
        if count > MAX_CONTEXT_CHARACTERS {
            let skip = count - MAX_CONTEXT_CHARACTERS;
            if let Some((start, _)) = text.left.char_indices().nth(skip) {
                text.left = text.left[start..].to_owned();
            }
        }
        text
    })
}

pub(crate) fn is_valid_reading(reading: &str) -> bool {
    !reading.is_empty() && chars(reading) <= MAX_READING_CHARACTERS
}

/// 送れる選択か（個数、通し番号、`literal` の長さ）。`literal` が空なら候補の番号として読む
pub(crate) fn is_valid_choices(choices: &[v1::SegmentChoice]) -> bool {
    choices.len() <= MAX_SEGMENTS
        && choices.iter().all(|choice| {
            if choice.literal.is_empty() {
                choice.candidate_index <= MAX_INDEX
            } else {
                chars(&choice.literal) <= MAX_LITERAL_CHARACTERS
            }
        })
}

// ---------------------------------------------------------------- 返事

/// 返事のバイト列を読む。大きすぎる、読めない、要求番号が違う（0 は許す）なら `InvalidReply`
pub(crate) fn decode(bytes: &[u8], expected_request_id: u64) -> Result<v1::Envelope, EngineError> {
    if bytes.len() > MAX_REPLY_BYTES {
        return Err(EngineError::InvalidReply);
    }
    let envelope = v1::Envelope::decode(bytes).map_err(|_| EngineError::InvalidReply)?;
    if envelope.request_id != expected_request_id && envelope.request_id != 0 {
        return Err(EngineError::InvalidReply);
    }
    Ok(envelope)
}

pub fn error_code(code: i32) -> EngineErrorCode {
    match v1::ErrorCode::try_from(code) {
        Ok(v1::ErrorCode::SchemaMismatch) => EngineErrorCode::SchemaMismatch,
        Ok(v1::ErrorCode::BadRequest) => EngineErrorCode::BadRequest,
        Ok(v1::ErrorCode::UnknownSession) => EngineErrorCode::UnknownSession,
        Ok(v1::ErrorCode::UnknownConversion) => EngineErrorCode::UnknownConversion,
        Ok(v1::ErrorCode::EngineNotReady) => EngineErrorCode::EngineNotReady,
        Ok(v1::ErrorCode::Internal) => EngineErrorCode::Internal,
        Ok(v1::ErrorCode::StaleLayout) => EngineErrorCode::StaleLayout,
        Ok(v1::ErrorCode::Unspecified) | Err(_) => EngineErrorCode::Unspecified,
    }
}

/// `HelloReply.lm` をメニューの LM の状態にする（今の `EnvelopeCodec.lmState`）
pub fn lm_state(reply: &v1::HelloReply) -> EngineLmState {
    match v1::LmState::try_from(reply.lm) {
        Ok(v1::LmState::LmDisabled) => EngineLmState::Disabled,
        Ok(v1::LmState::LmLoading) => EngineLmState::Loading,
        Ok(v1::LmState::LmReady) => EngineLmState::Ready,
        Ok(v1::LmState::LmFailed) => EngineLmState::Failed,
        Ok(v1::LmState::Unspecified) | Err(_) => EngineLmState::Unspecified,
    }
}

/// 殻の診断の 1 件（`diag::DiagEvent`）を `EngineLink::diag` に渡すメッセージにする。キーの昇順（`BTreeMap` の順）
pub fn diag_message(event: &DiagEvent) -> v1::DiagEvent {
    v1::DiagEvent {
        session_id: event.session_id,
        kind: event.kind.clone(),
        nums: event
            .nums
            .iter()
            .map(|(key, value)| v1::DiagNum {
                key: key.clone(),
                value: *value,
            })
            .collect(),
        tags: event
            .tags
            .iter()
            .map(|(key, value)| v1::DiagTag {
                key: key.clone(),
                value: value.clone(),
            })
            .collect(),
    }
}

/// `ErrorReply` なら対応する `EngineError::Engine` を返す。そうでなければ本体を返す
pub(crate) fn body(envelope: v1::Envelope) -> Result<Body, EngineError> {
    match envelope.body {
        Some(Body::Error(reply)) => Err(EngineError::Engine(error_code(reply.code))),
        Some(body) => Ok(body),
        None => Err(EngineError::InvalidReply),
    }
}

/// 返事が `code` の `ErrorReply` か
pub(crate) fn is_error(envelope: &v1::Envelope, code: v1::ErrorCode) -> bool {
    matches!(&envelope.body, Some(Body::Error(reply)) if reply.code == code as i32)
}

pub(crate) fn hello(envelope: v1::Envelope) -> Result<v1::HelloReply, EngineError> {
    let Body::HelloReply(reply) = body(envelope)? else {
        return Err(EngineError::InvalidReply);
    };
    if chars(&reply.engine_version) > MAX_TEXT_CHARACTERS
        || chars(&reply.dict_label) > MAX_TEXT_CHARACTERS
    {
        return Err(EngineError::InvalidReply);
    }
    Ok(reply)
}

fn check_candidates(candidates: &[v1::Candidate]) -> Result<(), EngineError> {
    if candidates.len() > MAX_CANDIDATES {
        return Err(EngineError::InvalidReply);
    }
    let broken = candidates.iter().any(|candidate| {
        candidate.surface.is_empty()
            || chars(&candidate.surface) > MAX_TEXT_CHARACTERS
            || chars(&candidate.annotation) > MAX_TEXT_CHARACTERS
    });
    if broken {
        Err(EngineError::InvalidReply)
    } else {
        Ok(())
    }
}

/// `convert_reply`（`page_reply` が偽）または `page_reply`（真）を検証する
pub(crate) fn page(
    envelope: v1::Envelope,
    page_reply: bool,
) -> Result<v1::ConvertReply, EngineError> {
    let reply = match body(envelope)? {
        Body::ConvertReply(reply) if !page_reply => reply,
        Body::PageReply(reply) if page_reply => reply,
        _ => return Err(EngineError::InvalidReply),
    };
    if reply.page_index > MAX_INDEX {
        return Err(EngineError::InvalidReply);
    }
    check_candidates(&reply.candidates)?;
    if let Some(layout) = &reply.layout {
        check_layout(layout)?;
    }
    Ok(reply)
}

fn check_segment_page(reply: &v1::SegmentPageReply) -> Result<(), EngineError> {
    if reply.page_index > MAX_INDEX || reply.segment as usize > MAX_SEGMENTS {
        return Err(EngineError::InvalidReply);
    }
    check_candidates(&reply.candidates)
}

/// 区切りを検証する。文節がない、番号が範囲外、文字列が壊れている返事は信用しない
fn check_layout(reply: &v1::SegmentLayout) -> Result<(), EngineError> {
    let count = reply.segments.len();
    if !(1..=MAX_SEGMENTS).contains(&count) || reply.focused as usize >= count {
        return Err(EngineError::InvalidReply);
    }
    let broken = reply.segments.iter().any(|info| {
        info.reading.is_empty()
            || info.surface.is_empty()
            || chars(&info.reading) > MAX_TEXT_CHARACTERS
            || chars(&info.surface) > MAX_TEXT_CHARACTERS
    });
    if broken {
        return Err(EngineError::InvalidReply);
    }
    if let Some(page) = &reply.focused_page {
        check_segment_page(page)?;
    }
    Ok(())
}

pub(crate) fn segment_page(envelope: v1::Envelope) -> Result<v1::SegmentPageReply, EngineError> {
    let Body::SegmentPageReply(reply) = body(envelope)? else {
        return Err(EngineError::InvalidReply);
    };
    check_segment_page(&reply)?;
    Ok(reply)
}

pub(crate) fn segment_layout(envelope: v1::Envelope) -> Result<v1::SegmentLayout, EngineError> {
    let Body::SegmentLayout(reply) = body(envelope)? else {
        return Err(EngineError::InvalidReply);
    };
    check_layout(&reply)?;
    Ok(reply)
}

/// `postal_lookup_reply` を検証する。件数と、住所ごとの空でないこと・長さを確かめる
pub(crate) fn postal_lookup(envelope: v1::Envelope) -> Result<Vec<String>, EngineError> {
    let Body::PostalLookupReply(reply) = body(envelope)? else {
        return Err(EngineError::InvalidReply);
    };
    if reply.addresses.len() > MAX_POSTAL_ADDRESSES {
        return Err(EngineError::InvalidReply);
    }
    let broken = reply
        .addresses
        .iter()
        .any(|address| address.is_empty() || chars(address) > MAX_POSTAL_ADDRESS_CHARACTERS);
    if broken {
        return Err(EngineError::InvalidReply);
    }
    Ok(reply.addresses)
}

/// `emoji_table_reply` を検証する。件数、名前と絵文字の長さ、名前が UTF-8 のバイト順で真に昇順
/// （二分探索の前提。重複もなし）であることを確かめる
pub(crate) fn emoji_table(envelope: v1::Envelope) -> Result<v1::EmojiTableReply, EngineError> {
    let Body::EmojiTableReply(reply) = body(envelope)? else {
        return Err(EngineError::InvalidReply);
    };
    if reply.entries.len() > MAX_EMOJI_ENTRIES || chars(&reply.data_label) > MAX_TEXT_CHARACTERS {
        return Err(EngineError::InvalidReply);
    }
    let mut previous: Option<&str> = None;
    for entry in &reply.entries {
        if entry.name.is_empty()
            || chars(&entry.name) > MAX_EMOJI_NAME_CHARACTERS
            || entry.emoji.is_empty()
            || chars(&entry.emoji) > MAX_EMOJI_SCALARS
        {
            return Err(EngineError::InvalidReply);
        }
        if previous.is_some_and(|prev| prev.as_bytes() >= entry.name.as_bytes()) {
            return Err(EngineError::InvalidReply);
        }
        previous = Some(&entry.name);
    }
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope(body: Body) -> v1::Envelope {
        v1::Envelope {
            schema_version: SCHEMA_VERSION,
            request_id: 0,
            body: Some(body),
        }
    }

    fn error(code: v1::ErrorCode) -> v1::Envelope {
        envelope(Body::Error(v1::ErrorReply {
            code: code as i32,
            message: String::new(),
        }))
    }

    fn postal(addresses: &[&str]) -> v1::Envelope {
        envelope(Body::PostalLookupReply(v1::PostalLookupReply {
            addresses: addresses.iter().map(|s| (*s).to_owned()).collect(),
        }))
    }

    fn emoji(entries: &[(&str, &str)]) -> v1::Envelope {
        envelope(Body::EmojiTableReply(v1::EmojiTableReply {
            entries: entries
                .iter()
                .map(|(name, emoji)| v1::EmojiEntry {
                    name: (*name).to_owned(),
                    emoji: (*emoji).to_owned(),
                })
                .collect(),
            data_label: "emoji-data 16.0".into(),
        }))
    }

    #[test]
    fn clipping_keeps_the_last_two_thousand_characters() {
        let left = "あ".repeat(2500) + "終";
        let text = clipped(Some(v1::SurroundingText {
            left,
            right: "右".into(),
            available: true,
        }))
        .unwrap();
        assert_eq!(text.left.chars().count(), 2000);
        assert!(text.left.ends_with('終'));
        assert_eq!(text.right, "右");
        let short = clipped(Some(v1::SurroundingText {
            left: "短い".into(),
            ..Default::default()
        }))
        .unwrap();
        assert_eq!(short.left, "短い");
        assert_eq!(clipped(None), None);
    }

    #[test]
    fn decode_rejects_oversized_broken_and_mismatched_replies() {
        let ok = encode(5, Body::Ack(v1::Ack {}));
        assert!(decode(&ok, 5).is_ok());
        assert!(decode(&encode(0, Body::Ack(v1::Ack {})), 5).is_ok());
        assert_eq!(decode(&ok, 6), Err(EngineError::InvalidReply));
        assert_eq!(decode(&[0xFF, 0xFF], 1), Err(EngineError::InvalidReply));
        assert_eq!(
            decode(&vec![0u8; MAX_REPLY_BYTES + 1], 1),
            Err(EngineError::InvalidReply)
        );
    }

    #[test]
    fn error_codes_map_one_to_one_and_unknown_is_unspecified() {
        assert_eq!(
            error_code(v1::ErrorCode::StaleLayout as i32),
            EngineErrorCode::StaleLayout
        );
        assert_eq!(error_code(999), EngineErrorCode::Unspecified);
        assert_eq!(
            hello(error(v1::ErrorCode::SchemaMismatch)),
            Err(EngineError::Engine(EngineErrorCode::SchemaMismatch))
        );
    }

    #[test]
    fn lm_states_map_one_to_one_and_unknown_is_unspecified() {
        let reply = |lm: i32| v1::HelloReply {
            lm,
            ..Default::default()
        };
        assert_eq!(
            lm_state(&reply(v1::LmState::LmReady as i32)),
            EngineLmState::Ready
        );
        assert_eq!(
            lm_state(&reply(v1::LmState::LmLoading as i32)),
            EngineLmState::Loading
        );
        assert_eq!(
            lm_state(&reply(v1::LmState::LmFailed as i32)),
            EngineLmState::Failed
        );
        assert_eq!(
            lm_state(&reply(v1::LmState::LmDisabled as i32)),
            EngineLmState::Disabled
        );
        assert_eq!(lm_state(&reply(99)), EngineLmState::Unspecified);
    }

    #[test]
    fn diag_events_keep_keys_in_order() {
        let mut event = DiagEvent::new(3, "sh_episode");
        event.nums.insert("b".into(), 2);
        event.nums.insert("a".into(), 1);
        event.tags.insert("mode".into(), "romaji".into());
        let message = diag_message(&event);
        assert_eq!(message.session_id, 3);
        assert_eq!(message.kind, "sh_episode");
        let keys: Vec<_> = message
            .nums
            .iter()
            .map(|n| (n.key.as_str(), n.value))
            .collect();
        assert_eq!(keys, [("a", 1), ("b", 2)]);
        assert_eq!(message.tags[0].value, "romaji");
    }

    #[test]
    fn choices_are_checked_for_count_index_and_literal_length() {
        let candidate = |index| v1::SegmentChoice {
            candidate_index: index,
            literal: String::new(),
        };
        let literal = |text: String| v1::SegmentChoice {
            candidate_index: 0,
            literal: text,
        };
        assert!(is_valid_choices(&[candidate(4), literal("ヒンシ".into())]));
        assert!(!is_valid_choices(&[candidate(MAX_INDEX + 1)]));
        assert!(is_valid_choices(&[literal("あ".repeat(256))]));
        assert!(!is_valid_choices(&[literal("あ".repeat(257))]));
        assert!(is_valid_choices(&vec![candidate(0); MAX_SEGMENTS]));
        assert!(!is_valid_choices(&vec![candidate(0); MAX_SEGMENTS + 1]));
    }

    // ---- 郵便番号（PostalLookupCodecTests）

    #[test]
    fn postal_decodes_addresses_in_order_and_an_empty_list_is_fine() {
        assert_eq!(
            postal_lookup(postal(&["東京都千代田区千代田", "東京都千代田区丸の内"])).unwrap(),
            vec!["東京都千代田区千代田", "東京都千代田区丸の内"]
        );
        assert_eq!(postal_lookup(postal(&[])).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn postal_rejects_empty_or_too_long_addresses_and_too_many() {
        assert_eq!(postal_lookup(postal(&[""])), Err(EngineError::InvalidReply));
        let long = "あ".repeat(MAX_POSTAL_ADDRESS_CHARACTERS + 1);
        assert_eq!(
            postal_lookup(postal(&[&long])),
            Err(EngineError::InvalidReply)
        );
        let many = vec!["住所"; MAX_POSTAL_ADDRESSES + 1];
        assert_eq!(postal_lookup(postal(&many)), Err(EngineError::InvalidReply));
    }

    #[test]
    fn postal_accepts_the_limits() {
        let edge = "あ".repeat(MAX_POSTAL_ADDRESS_CHARACTERS);
        let all = vec![edge.as_str(); MAX_POSTAL_ADDRESSES];
        assert_eq!(
            postal_lookup(postal(&all)).unwrap().len(),
            MAX_POSTAL_ADDRESSES
        );
    }

    #[test]
    fn postal_rejects_the_wrong_reply_and_maps_errors() {
        assert_eq!(
            postal_lookup(envelope(Body::Ack(v1::Ack {}))),
            Err(EngineError::InvalidReply)
        );
        assert_eq!(
            postal_lookup(error(v1::ErrorCode::BadRequest)),
            Err(EngineError::Engine(EngineErrorCode::BadRequest))
        );
    }

    // ---- 絵文字の表（EmojiTableCodecTests）

    #[test]
    fn emoji_decodes_entries_and_the_label_and_an_empty_table_is_fine() {
        let table = emoji_table(emoji(&[("+1", "👍"), ("smile", "😄")])).unwrap();
        assert_eq!(table.entries.len(), 2);
        assert_eq!(table.data_label, "emoji-data 16.0");
        assert!(emoji_table(emoji(&[])).unwrap().entries.is_empty());
    }

    #[test]
    fn emoji_sequences_with_many_scalars_are_accepted() {
        // 家族の ZWJ の連なり（7 スカラー）
        let family = "👨\u{200D}👩\u{200D}👧\u{200D}👦";
        assert!(emoji_table(emoji(&[("family", family)])).is_ok());
    }

    #[test]
    fn emoji_rejects_entries_that_break_the_contract() {
        let long_name = "a".repeat(MAX_EMOJI_NAME_CHARACTERS + 1);
        let long_emoji = "😀".repeat(MAX_EMOJI_SCALARS + 1);
        let cases: Vec<Vec<(&str, &str)>> = vec![
            vec![("", "😀")],
            vec![("a", "")],
            vec![(&long_name, "😀")],
            vec![("a", &long_emoji)],
            vec![("b", "😀"), ("a", "😀")],
            vec![("a", "😀"), ("a", "😀")],
        ];
        for entries in cases {
            assert_eq!(
                emoji_table(emoji(&entries)),
                Err(EngineError::InvalidReply),
                "{entries:?}"
            );
        }
    }

    #[test]
    fn emoji_rejects_too_many_entries() {
        let names: Vec<String> = (0..=MAX_EMOJI_ENTRIES)
            .map(|i| format!("e{i:05}"))
            .collect();
        let entries: Vec<(&str, &str)> = names.iter().map(|n| (n.as_str(), "😀")).collect();
        assert_eq!(emoji_table(emoji(&entries)), Err(EngineError::InvalidReply));
    }

    #[test]
    fn emoji_rejects_the_wrong_reply_and_maps_errors() {
        assert_eq!(
            emoji_table(envelope(Body::Ack(v1::Ack {}))),
            Err(EngineError::InvalidReply)
        );
        assert_eq!(
            emoji_table(error(v1::ErrorCode::Internal)),
            Err(EngineError::Engine(EngineErrorCode::Internal))
        );
    }
}
