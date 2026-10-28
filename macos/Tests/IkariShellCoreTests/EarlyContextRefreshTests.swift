// 合成の最初の数打鍵は、左文脈を読み直す（#1。確定の文字がアプリに入る前に 1 打鍵目の文脈を読むため）
import Testing

@testable import IkariShellCore

@Suite struct EarlyContextRefreshTests {
    static func prefetches(_ h: Harness) -> [ConversionRequest] {
        h.engine.calls.compactMap { if case .prefetch(let request) = $0 { request } else { nil } }
    }

    @Test func grownLeftOnSecondKeystrokeIsSentWithANewRevision() throws {
        let h = try Harness()
        h.surroundingText = SurroundingText(left: "", available: true)
        h.type("k")
        h.surroundingText = SurroundingText(left: "確定", available: true)
        h.type("a")
        let requests = Self.prefetches(h)
        let sent = try #require(requests.last)
        #expect(requests.count == 1)
        #expect(sent.context.left == "確定")
        #expect(sent.contextRev == 2)
        #expect(sent.reading == "か")
    }

    @Test func unchangedLeftKeepsRevisionAndSendsNoExtraPrefetch() throws {
        let h = try Harness()
        h.type("ka")
        let before = Self.prefetches(h)
        h.type("k")  // 3 打鍵目。かなは変わらず、左も同じ
        let after = Self.prefetches(h)
        #expect(after.count == before.count)
        #expect(Set(after.map(\.contextRev)).count == 1)
    }

    @Test func changedLeftWithSameReadingResendsPrefetch() throws {
        let h = try Harness()
        h.type("ka")
        let before = Self.prefetches(h).count
        h.surroundingText = SurroundingText(left: "変わった", available: true)
        h.type("k")
        #expect(Self.prefetches(h).count == before + 1)
        #expect(Self.prefetches(h).last?.context.left == "変わった")
    }

    @Test func stopsReadingAfterTheConfiguredKeystrokes() throws {
        let h = try Harness()
        h.type(String(repeating: "a", count: InputStateMachine.contextRefreshKeystrokes))
        let reads = h.surroundingFetches
        #expect(reads == InputStateMachine.contextRefreshKeystrokes)
        h.surroundingText = SurroundingText(left: "後から", available: true)
        h.type("aaa")
        #expect(h.surroundingFetches == reads)
        #expect(Self.prefetches(h).last?.context.left != "後から")
    }

    @Test func secureNeverReads() throws {
        let h = try Harness(secure: true)
        h.type("aaaa")
        #expect(h.surroundingFetches == 0)
        #expect(Self.prefetches(h).isEmpty)
    }

    @Test func counterResetsOnNextComposition() throws {
        let h = try Harness()
        h.type("aaaa")
        h.press(.enter)
        let reads = h.surroundingFetches
        h.type("aa")
        #expect(h.surroundingFetches == reads + 2)
    }
}
