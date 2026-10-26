// 診断ログ D4a: 合成 1 回の `sh_episode`（ADR 0034 §4.2）。貯める値と終わった理由、secure で送らないこと。
import Foundation
import Testing

@testable import IkariShellCore

/// 打鍵ごとに 5ms 進む時計
private final class StepClock {
    private var now: UInt64 = 0
    static let stepNanos: UInt64 = 5_000_000
    func tick() -> UInt64 {
        now += Self.stepNanos
        return now
    }
}

/// engine/src/diag_schema.rs の `sh_episode` の許可リスト（数のキーとタグのキー）
private let allowedNums: Set<String> = [
    "keys", "keys_to_conv", "rlen", "dur_ms", "pick_ms", "spaces", "cand_moves", "pages", "pick", "digit_pick",
    "segs", "seg_focus", "seg_resize", "seg_changed", "esc_back", "bs", "wait_us", "fallback",
]
private let allowedTags: Set<String> = ["mode", "start", "end", "prev", "pick_kind", "panel", "ctx"]

private func install(_ clock: StepClock, on h: Harness) {
    h.machine.diagClock = { clock.tick() }
}

@Suite struct DiagEpisodeTests {
    @Test func spaceThenSecondCandidateThenEnter() throws {
        let h = try Harness()
        install(StepClock(), on: h)
        h.type("hinsi")
        #expect(h.engine.diagEvents.isEmpty)  // 打鍵ごとには送らない
        h.press(.space)
        h.press(.space)
        h.press(.enter)

        let events = h.engine.diagEvents
        try #require(events.count == 1)
        let event = events[0]
        #expect(event.kind == "sh_episode")
        #expect(event.sessionID == Harness.sessionID)
        #expect(
            event.tags == [
                "mode": "romaji", "start": "typing", "end": "commit_cand", "pick_kind": "dict", "panel": "none", "ctx": "ok",
            ])
        #expect(event.nums["keys"] == 8)
        #expect(event.nums["keys_to_conv"] == 6)
        #expect(event.nums["rlen"] == 3)
        #expect(event.nums["spaces"] == 2)
        #expect(event.nums["cand_moves"] == 1)
        #expect(event.nums["pick"] == 1)
        #expect(event.nums["segs"] == 1)
        #expect(event.nums["pages"] == 0)
        #expect(event.nums["bs"] == 0)
        #expect(event.nums["fallback"] == 0)
        // 時計は呼ばれるたびに 5ms 進む。値は 0 以上で、書くキーは許可リストの中だけ
        #expect(event.nums["dur_ms"] ?? -1 > 0)
        #expect(event.nums["pick_ms"] ?? -1 > 0)
        #expect(event.nums["wait_us"] ?? -1 >= 0)
        #expect(Set(event.nums.keys).isSubset(of: allowedNums))
        #expect(Set(event.tags.keys).isSubset(of: allowedTags))
    }

    @Test func segmentModeFocusThenSpaceThenEnter() throws {
        let h = try SegmentFixture.wholeCandidates()
        install(StepClock(), on: h)
        // 先頭の文節の候補を 1 つ動かして、確定する（Space のあとの ← で文節のモードに入る）
        h.press(.left)
        h.press(.space)
        h.press(.enter)

        let events = h.engine.diagEvents
        try #require(events.count == 1)
        let event = events[0]
        #expect(event.tags["end"] == "commit_segments")
        #expect(event.nums["segs"] == 2)
        #expect(event.nums["seg_focus"] == 1)
        #expect(event.nums["seg_changed"] == 1)
        #expect(event.nums["seg_resize"] == 0)
        #expect(event.nums["spaces"] == 2)
        #expect(event.nums["cand_moves"] == 1)
        #expect(event.tags["prev"] == nil)
    }

    @Test func escapeBackToReadingThenEscapeClears() throws {
        let h = try Harness()
        install(StepClock(), on: h)
        h.type("hinsi")
        h.press(.space)
        h.press(.escape)  // 変換から読みに戻る
        #expect(h.engine.diagEvents.isEmpty)
        h.press(.escape)  // 読みを消す

        let events = h.engine.diagEvents
        try #require(events.count == 1)
        #expect(events[0].tags["end"] == "esc_clear")
        #expect(events[0].nums["esc_back"] == 1)
        #expect(events[0].nums["spaces"] == 1)
        #expect(events[0].nums["pick"] == nil)
    }

    @Test func enterWithoutConvertingIsCommitKana() throws {
        let h = try Harness()
        install(StepClock(), on: h)
        h.type("hi")
        h.press(.enter)
        let events = h.engine.diagEvents
        try #require(events.count == 1)
        #expect(events[0].tags["end"] == "commit_kana")
        #expect(events[0].nums["keys_to_conv"] == nil)
        #expect(events[0].nums["pick_ms"] == nil)
        #expect(events[0].nums["spaces"] == 0)
    }

    @Test func backspaceToEmptyCountsBackspaces() throws {
        let h = try Harness()
        install(StepClock(), on: h)
        h.type("hik")
        h.press(.backspace)
        h.press(.backspace)
        let events = h.engine.diagEvents
        try #require(events.count == 1)
        #expect(events[0].tags["end"] == "bs_empty")
        #expect(events[0].nums["bs"] == 2)
    }

    @Test func externalCommitRecordsReasonAndPreviousState() throws {
        let composing = try Harness()
        install(StepClock(), on: composing)
        composing.type("hi")
        _ = composing.machine.commitComposition(reason: .deactivate)
        #expect(composing.engine.diagEvents.first?.tags["end"] == "deactivate")
        #expect(composing.engine.diagEvents.first?.tags["prev"] == "composing")

        let converting = try Harness()
        install(StepClock(), on: converting)
        converting.type("hinsi")
        converting.press(.space)
        _ = converting.machine.commitComposition(reason: .modeSwitch)
        #expect(converting.engine.diagEvents.first?.tags["end"] == "mode_switch")
        #expect(converting.engine.diagEvents.first?.tags["prev"] == "converting")

        let segments = try SegmentFixture.converting()
        install(StepClock(), on: segments)
        _ = segments.machine.commitComposition()
        #expect(segments.engine.diagEvents.first?.tags["end"] == "commit_composition")
        #expect(segments.engine.diagEvents.first?.tags["prev"] == "segments")
    }

    @Test func idleKeysPassedThroughMakeNoEpisode() throws {
        let h = try Harness()
        install(StepClock(), on: h)
        h.press(.character("!"))
        h.press(.backspace)
        h.type("hi")
        h.press(.enter)
        // 打鍵 1 回めはアプリに渡した `!` と BackSpace（合成の外）。数えるのは合成を始めてから
        #expect(h.engine.diagEvents.count == 1)
        #expect(h.engine.diagEvents[0].nums["keys"] == 3)
        #expect(h.engine.diagEvents[0].nums["bs"] == 0)
    }

    @Test func secureSessionNeverCollectsOrSends() throws {
        let h = try Harness(secure: true)
        install(StepClock(), on: h)
        h.type("hinsi")
        h.press(.space)
        h.press(.enter)
        #expect(h.engine.diagEvents.isEmpty)
    }

    @Test func turningSecureMidCompositionDropsTheEpisode() throws {
        let h = try Harness()
        install(StepClock(), on: h)
        h.type("hi")
        h.machine.setSecure(true)
        h.press(.character("a"))
        #expect(h.machine.state == .idle)
        #expect(h.engine.diagEvents.isEmpty)
        // secure が終わっても、捨てた合成の数は引き継がない
        h.machine.setSecure(false)
        h.type("hi")
        h.press(.enter)
        #expect(h.engine.diagEvents.count == 1)
        #expect(h.engine.diagEvents[0].nums["keys"] == 3)
    }

    @Test func fallbackCandidatesAreCounted() throws {
        let engine = MockEngine()
        engine.onConvert = { _ in throw EngineClientError.engine(.badRequest) }
        let h = try Harness(engine: engine)
        install(StepClock(), on: h)
        h.type("hinsi")
        h.press(.space)
        h.press(.enter)
        let event = try #require(h.engine.diagEvents.first)
        #expect(event.nums["fallback"] == 1)
        #expect(event.tags["pick_kind"] == "fallback")
        #expect(event.tags["end"] == "commit_cand")
    }
}
