import Foundation
import IkariProto
import Testing

@testable import IkariBridge

private func settingsFrame(id: UInt64 = 1, key: String = "paging.size", value: String = "5") -> Frame {
    var entry = Ikari_Bridge_V1_SettingEntry()
    entry.key = key
    entry.value = value
    var update = Ikari_Bridge_V1_UpdateSettings()
    update.set = [entry]
    var frame = Frame()
    frame.requestID = id
    frame.updateSettings = update
    return frame
}

private func prefix(_ length: UInt32) -> Data {
    Data([UInt8(length >> 24 & 0xFF), UInt8(length >> 16 & 0xFF), UInt8(length >> 8 & 0xFF), UInt8(length & 0xFF)])
}

@Suite struct FrameCodecTests {
    @Test func encodesABigEndianLengthFollowedByTheMessage() throws {
        let frame = settingsFrame()
        let body = try frame.serializedData()
        let data = try FrameCodec.encode(frame)
        #expect(data.prefix(4) == prefix(UInt32(body.count)))
        #expect(data.dropFirst(4) == body)
    }

    @Test func roundTripsOneFrame() throws {
        var decoder = FrameDecoder()
        let frames = try decoder.feed(try FrameCodec.encode(settingsFrame(id: 9)))
        #expect(frames == [settingsFrame(id: 9)])
        #expect(decoder.bufferedByteCount == 0)
    }

    @Test func reassemblesAFrameSplitAtEveryByte() throws {
        let data = try FrameCodec.encode(settingsFrame(id: 3))
        var decoder = FrameDecoder()
        var frames: [Frame] = []
        for byte in data { frames += try decoder.feed(Data([byte])) }
        #expect(frames == [settingsFrame(id: 3)])
    }

    @Test func splitsSeveralFramesDeliveredInOneChunk() throws {
        let first = try FrameCodec.encode(settingsFrame(id: 1))
        let second = try FrameCodec.encode(settingsFrame(id: 2, key: "paging.wrap", value: "false"))
        let third = try FrameCodec.encode(settingsFrame(id: 3))
        var decoder = FrameDecoder()
        // 3 つ目は途中までしか来ていない
        let frames = try decoder.feed(first + second + third.prefix(5))
        #expect(frames.map(\.requestID) == [1, 2])
        #expect(decoder.bufferedByteCount == 5)
        #expect(try decoder.feed(third.dropFirst(5)).map(\.requestID) == [3])
    }

    @Test func waitsForTheRestWhenOnlyAPartOfTheLengthHasArrived() throws {
        var decoder = FrameDecoder()
        #expect(try decoder.feed(Data([0, 0])).isEmpty)
        #expect(decoder.bufferedByteCount == 2)
    }

    @Test func acceptsALengthAtTheLimitButWaitsForTheBody() throws {
        var decoder = FrameDecoder()
        #expect(try decoder.feed(prefix(UInt32(BridgeLimits.maxFrameBytes))).isEmpty)
    }

    @Test func rejectsALengthOverTheLimitBeforeReadingTheBody() {
        var decoder = FrameDecoder()
        let length = UInt32(BridgeLimits.maxFrameBytes) + 1
        #expect(throws: FrameError.tooLarge(length: UInt64(length))) { try decoder.feed(prefix(length)) }
    }

    @Test func rejectsABrokenLengthThatLooksLikeAHugeNumber() {
        var decoder = FrameDecoder()
        #expect(throws: FrameError.tooLarge(length: UInt64(UInt32.max))) {
            try decoder.feed(Data([0xFF, 0xFF, 0xFF, 0xFF]))
        }
    }

    @Test func rejectsAZeroLengthFrame() {
        var decoder = FrameDecoder()
        #expect(throws: FrameError.empty) { try decoder.feed(prefix(0)) }
    }

    @Test func rejectsABodyThatIsNotAProtobufMessage() {
        var decoder = FrameDecoder()
        // フィールド 10（hello）の長さ指定が本体より長い
        let garbage = Data([0x52, 0x7F, 0x01])
        #expect(throws: FrameError.malformed) { try decoder.feed(prefix(UInt32(garbage.count)) + garbage) }
    }

    @Test func staysFailedAfterTheFirstError() throws {
        var decoder = FrameDecoder()
        #expect(throws: FrameError.empty) { try decoder.feed(prefix(0)) }
        // 枠の位置が分からなくなっているので、正しい枠が来ても読まない
        #expect(throws: FrameError.empty) { try decoder.feed(try FrameCodec.encode(settingsFrame())) }
    }

    @Test func refusesToEncodeAnEmptyFrame() {
        #expect(throws: FrameError.empty) { try FrameCodec.encode(Frame()) }
    }

    @Test func refusesToEncodeABodyOverTheLimit() {
        var export = Ikari_Engine_V1_ExportReply()
        export.content = Data(count: BridgeLimits.maxFrameBytes + 1)
        var envelope = EngineEnvelope()
        envelope.exportReply = export
        var frame = Frame()
        frame.engine = envelope
        #expect(throws: FrameError.self) { try FrameCodec.encode(frame) }
    }

    @Test func carriesAnEngineEnvelopeUnchanged() throws {
        var import_ = Ikari_Engine_V1_ImportData()
        import_.content = Data((0..<1000).map { UInt8($0 % 251) })
        import_.dryRun = true
        var envelope = EngineEnvelope()
        envelope.schemaVersion = 1
        envelope.requestID = 77
        envelope.importData = import_
        var frame = Frame()
        frame.requestID = 5
        frame.engine = envelope
        var decoder = FrameDecoder()
        let decoded = try decoder.feed(try FrameCodec.encode(frame))
        #expect(decoded == [frame])
    }
}
