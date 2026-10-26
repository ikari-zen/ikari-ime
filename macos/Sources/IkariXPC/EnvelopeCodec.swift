// EngineClient の型と protobuf（Envelope）の変換と、返事の検証（ADR 0004 §5.2「信用しない入力」）。
import Foundation
import IkariProto
import IkariShellCore
import SwiftProtobuf

typealias Envelope = Ikari_Engine_V1_Envelope

enum EnvelopeCodec {
    /// `Envelope.schema_version`（proto の `SCHEMA_VERSION` と同じ）
    static let schemaVersion: UInt32 = 1
    /// 返事の検証の上限（エンジンの上限より緩くし、壊れた返事だけを弾く）
    static let maxCandidates = 64
    static let maxTextCharacters = 512
    static let maxReplyBytes = 1 << 20

    // MARK: 要求

    static func envelope(requestID: UInt64, _ build: (inout Envelope) -> Void) -> Envelope {
        var env = Envelope()
        env.schemaVersion = schemaVersion
        env.requestID = requestID
        build(&env)
        return env
    }

    static func encode(_ env: Envelope) -> Data {
        // 自分で作ったメッセージのエンコードは失敗しない（必須フィールドのない proto3）
        (try? env.serializedData()) ?? Data()
    }

    static func surrounding(_ text: IkariShellCore.SurroundingText) -> Ikari_Engine_V1_SurroundingText {
        var out = Ikari_Engine_V1_SurroundingText()
        out.left = text.left
        out.right = text.right
        out.available = text.available
        return out
    }

    // MARK: 返事

    static func decode(_ data: Data, expectedRequestID: UInt64) throws -> Envelope {
        guard data.count <= maxReplyBytes,
            let env = try? Envelope(serializedBytes: data),
            env.requestID == expectedRequestID || env.requestID == 0
        else { throw EngineClientError.invalidReply }
        return env
    }

    /// `ErrorReply` なら対応する `EngineClientError` を投げる。
    static func throwIfError(_ env: Envelope) throws {
        if case .error(let reply)? = env.body {
            throw EngineClientError.engine(errorCode(reply.code))
        }
    }

    static func errorCode(_ code: Ikari_Engine_V1_ErrorCode) -> EngineErrorCode {
        switch code {
        case .schemaMismatch: return .schemaMismatch
        case .badRequest: return .badRequest
        case .unknownSession: return .unknownSession
        case .unknownConversion: return .unknownConversion
        case .engineNotReady: return .engineNotReady
        case .internal: return .internal
        case .staleLayout: return .staleLayout
        default: return .unspecified
        }
    }

    static func lmState(_ state: Ikari_Engine_V1_LmState) -> EngineLmState {
        switch state {
        case .lmDisabled: return .disabled
        case .lmLoading: return .loading
        case .lmReady: return .ready
        case .lmFailed: return .failed
        default: return .unspecified
        }
    }

    static func kind(_ kind: Ikari_Engine_V1_CandidateKind) -> CandidateKind {
        switch kind {
        case .dictionary: return .dictionary
        case .hiragana: return .hiragana
        case .katakana: return .katakana
        case .rewriter: return .rewriter
        case .literal: return .literal
        default: return .unspecified
        }
    }

    static func hello(_ env: Envelope) throws -> EngineHello {
        try throwIfError(env)
        guard case .helloReply(let reply)? = env.body else { throw EngineClientError.invalidReply }
        guard reply.engineVersion.count <= maxTextCharacters, reply.dictLabel.count <= maxTextCharacters else {
            throw EngineClientError.invalidReply
        }
        return EngineHello(engineVersion: reply.engineVersion, dictLabel: reply.dictLabel, lm: lmState(reply.lm))
    }

    /// `convert_reply` / `page_reply` を検証して `ConversionPage` にする。
    static func page(_ env: Envelope, pageReply: Bool) throws -> ConversionPage {
        try throwIfError(env)
        let reply: Ikari_Engine_V1_ConvertReply
        switch env.body {
        case .convertReply(let r)? where !pageReply: reply = r
        case .pageReply(let r)? where pageReply: reply = r
        default: throw EngineClientError.invalidReply
        }
        guard reply.candidates.count <= maxCandidates, reply.pageIndex <= UInt32(Int32.max) else {
            throw EngineClientError.invalidReply
        }
        var items: [CandidateItem] = []
        items.reserveCapacity(reply.candidates.count)
        for candidate in reply.candidates {
            guard !candidate.surface.isEmpty,
                candidate.surface.count <= maxTextCharacters,
                candidate.annotation.count <= maxTextCharacters
            else { throw EngineClientError.invalidReply }
            items.append(
                CandidateItem(surface: candidate.surface, annotation: candidate.annotation, kind: kind(candidate.kind)))
        }
        return ConversionPage(
            conversionID: reply.conversionID,
            pageIndex: Int(reply.pageIndex),
            candidates: items,
            hasNext: reply.hasNext_p,
            lmApplied: reply.lmApplied,
            engineMicros: Int(reply.engineMicros),
            layout: try reply.hasLayout ? layout(reply.layout) : nil
        )
    }

    // MARK: 文節と再変換（ADR 0006 §5.2）

    /// 文節の数の上限（読みの上限 256 文字と同じ。1 文字ずつの文節が最大）
    static let maxSegments = 256
    /// `SegmentChoice.literal` の上限（文字数）
    static let maxLiteralCharacters = 256

    static func choice(_ value: SegmentChoiceValue) -> Ikari_Engine_V1_SegmentChoice {
        var out = Ikari_Engine_V1_SegmentChoice()
        switch value {
        case .candidate(let index): out.candidateIndex = UInt32(clamping: max(0, index))
        case .literal(let text): out.literal = text
        }
        return out
    }

    /// 送れる選択か（個数、通し番号、`literal` の長さ。空の `literal` は「候補の番号」と区別できないので送らない）
    static func isValid(_ choices: [SegmentChoiceValue]) -> Bool {
        guard choices.count <= maxSegments else { return false }
        return choices.allSatisfy {
            switch $0 {
            case .candidate(let index): (0...Int(Int32.max)).contains(index)
            case .literal(let text): !text.isEmpty && text.count <= maxLiteralCharacters
            }
        }
    }

    static func candidates(_ source: [Ikari_Engine_V1_Candidate]) throws -> [CandidateItem] {
        guard source.count <= maxCandidates else { throw EngineClientError.invalidReply }
        return try source.map { candidate in
            guard !candidate.surface.isEmpty,
                candidate.surface.count <= maxTextCharacters,
                candidate.annotation.count <= maxTextCharacters
            else { throw EngineClientError.invalidReply }
            return CandidateItem(surface: candidate.surface, annotation: candidate.annotation, kind: kind(candidate.kind))
        }
    }

    static func segmentPage(_ reply: Ikari_Engine_V1_SegmentPageReply) throws -> SegmentPage {
        guard reply.pageIndex <= UInt32(Int32.max), reply.segment <= UInt32(maxSegments) else {
            throw EngineClientError.invalidReply
        }
        return SegmentPage(
            segment: Int(reply.segment),
            pageIndex: Int(reply.pageIndex),
            candidates: try candidates(reply.candidates),
            hasNext: reply.hasNext_p,
            lmApplied: reply.lmApplied,
            engineMicros: Int(reply.engineMicros)
        )
    }

    /// 区切りを検証して `SegmentLayoutInfo` にする。文節がない、番号が範囲外、文字列が壊れている返事は信用しない
    static func layout(_ reply: Ikari_Engine_V1_SegmentLayout) throws -> SegmentLayoutInfo {
        guard (1...maxSegments).contains(reply.segments.count), Int(reply.focused) < reply.segments.count else {
            throw EngineClientError.invalidReply
        }
        let segments = try reply.segments.map { info -> SegmentSummary in
            guard !info.reading.isEmpty, !info.surface.isEmpty,
                info.reading.count <= maxTextCharacters, info.surface.count <= maxTextCharacters
            else { throw EngineClientError.invalidReply }
            return SegmentSummary(reading: info.reading, surface: info.surface)
        }
        return SegmentLayoutInfo(
            conversionID: reply.conversionID,
            layoutRev: reply.layoutRev,
            segments: segments,
            focused: Int(reply.focused),
            focusedPage: try reply.hasFocusedPage ? segmentPage(reply.focusedPage) : nil,
            engineMicros: Int(reply.engineMicros)
        )
    }

    static func segmentPageReply(_ env: Envelope) throws -> SegmentPage {
        try throwIfError(env)
        guard case .segmentPageReply(let reply)? = env.body else { throw EngineClientError.invalidReply }
        return try segmentPage(reply)
    }

    static func segmentLayoutReply(_ env: Envelope) throws -> SegmentLayoutInfo {
        try throwIfError(env)
        guard case .segmentLayout(let reply)? = env.body else { throw EngineClientError.invalidReply }
        return try layout(reply)
    }
}
