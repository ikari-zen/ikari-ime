// 候補ウィンドウの見せ方と選び方（ADR 0014。B27）。↑↓、3 件から 9 件への展開、番号キー。
import Testing

@testable import IkariShellCore

@Suite struct CandidateWindowTests {
    /// 9 件の 1 ページ目と、9 件の 2 ページ目
    static func surfaces(_ prefix: String, _ count: Int) -> [CandidateItem] {
        (1...count).map { CandidateItem(surface: "\(prefix)\($0)") }
    }

    static func engine(pages: [[CandidateItem]] = [surfaces("a", 9), surfaces("b", 9)]) -> MockEngine {
        let engine = MockEngine()
        let converted = pages.enumerated().map { index, candidates in
            ConversionPage(conversionID: 7, pageIndex: index, candidates: candidates, hasNext: index + 1 < pages.count)
        }
        engine.onConvert = { _ in converted[0] }
        engine.onPage = { _, index in
            guard converted.indices.contains(index) else { throw EngineClientError.engine(.badRequest) }
            return converted[index]
        }
        return engine
    }

    static func converting(settings: Settings = .default, engine: MockEngine = engine()) throws -> Harness {
        let h = try Harness(settings: settings, engine: engine)
        h.type("hi")
        h.press(.space)
        return h
    }

    static func panel(_ actions: [KeyAction]) -> CandidatePanelModel? {
        for action in actions {
            switch action {
            case .showCandidates(let model), .moveSelection(let model): return model
            default: continue
            }
        }
        return nil
    }

    // MARK: ↑↓

    @Test func downAndUpMoveLikeSpaceAndShiftSpace() throws {
        let viaArrows = try Self.converting()
        let viaSpace = try Self.converting()
        for _ in 0..<5 {
            #expect(viaArrows.press(.down) == viaSpace.press(.space))
        }
        for _ in 0..<3 {
            #expect(viaArrows.press(.up) == viaSpace.press(.space, .shift))
        }
        #expect(viaArrows.converting == viaSpace.converting)
    }

    @Test func upOnTheFirstCandidateDoesNothing() throws {
        let h = try Self.converting()
        #expect(h.press(.up) == [.consume])
        #expect(h.converting?.selected == 0)
    }

    @Test func downBeforeConvertingPassesThroughWhenIdleAndIsSwallowedWhenComposing() throws {
        let h = try Harness()
        #expect(h.press(.down) == [.passThrough])
        #expect(h.press(.up) == [.passThrough])
        h.type("hinsi")
        #expect(h.press(.down) == [.consume])  // 変換は始まらない
        #expect(h.press(.up) == [.consume])
        #expect(h.machine.state == .composing(kana: "ひんし", pending: ""))
    }

    @Test func downIsSwallowedInAnAsciiRun() throws {
        let h = try Harness()
        h.press(.character("A"), .shift)
        #expect(h.press(.down) == [.consume])
    }

    @Test func shiftArrowsDoNothingWhileConverting() throws {
        let h = try Self.converting()
        #expect(h.press(.down, .shift) == [.consume])
        #expect(h.press(.up, .shift) == [.consume])
        #expect(h.converting?.selected == 0)
    }

    @Test func unbindingTheArrowsLeavesThemSwallowed() throws {
        let h = try Self.converting(settings: Settings(keymapOverrides: [.candidateNext: "none", .candidatePrevious: "none"]))
        #expect(h.press(.down) == [.consume])
        #expect(h.converting?.selected == 0)
    }

    @Test func candidateNextCanBeBoundToAnotherKey() throws {
        let h = try Self.converting(settings: Settings(keymapOverrides: [.candidateNext: "ctrl+n"]))
        h.press(.character("n"), .control)
        #expect(h.converting?.selected == 1)
        // 元のキーは外れる
        #expect(h.press(.down) == [.consume])
        #expect(h.converting?.selected == 1)
    }

    @Test func arrowsDoNotEnterTheClauseMode() throws {
        let h = try SegmentFixture.wholeCandidates()
        h.press(.down)
        h.press(.up)
        let state = try #require(h.converting)
        #expect(state.wholeLayout != nil)
        #expect(state.segments.count == 1)
    }

    @Test func arrowsMoveTheFocusedSegmentsCandidatesInTheClauseMode() throws {
        let h = try SegmentFixture.converting()
        h.press(.right)  // 文節のモードへ。2 文節目にフォーカス
        let focused = try #require(h.converting).focused
        h.press(.down)
        let state = try #require(h.converting)
        #expect(state.focused == focused)
        #expect(state.selected == 1)
        h.press(.up)
        #expect(h.converting?.selected == 0)
    }

    // MARK: 上に出たときの ↑↓（ADR 0014 の決定の節「上に出たときの ↑↓」）

    @Test func arrowsFollowTheScreenWhenThePanelIsBelow() throws {
        let h = try Self.converting()
        h.machine.setPanelSide(.below)
        h.press(.down)
        #expect(h.converting?.selected == 1)
        h.press(.up)
        #expect(h.converting?.selected == 0)
    }

    @Test func arrowsAreSwappedWhenThePanelIsAbove() throws {
        let h = try Self.converting()
        h.machine.setPanelSide(.above)
        h.press(.up)
        #expect(h.converting?.selected == 1)
        h.press(.up)
        #expect(h.converting?.selected == 2)
        h.press(.down)
        #expect(h.converting?.selected == 1)
    }

    @Test func arrowsAreBelowBeforeThePanelIsPlaced() throws {
        let h = try Self.converting()
        #expect(h.machine.panelSide == .below)
        h.press(.down)
        #expect(h.converting?.selected == 1)
    }

    @Test func spaceIsUnchangedWhenThePanelIsAbove() throws {
        let h = try Self.converting()
        h.machine.setPanelSide(.above)
        h.press(.space)
        #expect(h.converting?.selected == 1)
        h.press(.space, .shift)
        #expect(h.converting?.selected == 0)
    }

    @Test func reboundCandidateNextIsNotReversedWhenAbove() throws {
        let h = try Self.converting(settings: Settings(keymapOverrides: [.candidateNext: "ctrl+n"]))
        h.machine.setPanelSide(.above)
        h.press(.character("n"), .control)
        #expect(h.converting?.selected == 1)
        h.press(.up)  // ↑↓ は既定でなくなったので、反転せず「前」のまま
        #expect(h.converting?.selected == 0)
    }

    @Test func arrowsStaySwappedAfterExpandingToNine() throws {
        let h = try Self.converting()
        h.machine.setPanelSide(.above)
        h.press(.up)
        h.press(.up)
        h.press(.up)  // 3 件目から進んで広げる
        #expect(h.converting?.selected == 3)
        #expect(h.converting?.segments.first?.expanded == true)
        h.press(.down)
        #expect(h.converting?.selected == 2)
    }

    // MARK: 3 件から 9 件へ

    @Test func theFirstPageShowsThreeAndReservesNineRows() throws {
        let h = try Harness(engine: Self.engine())
        h.type("hi")
        let model = try #require(Self.panel(h.press(.space)))
        #expect(model.candidates.map(\.surface) == ["a1", "a2", "a3"])
        #expect(model.reservedRows == 9)
    }

    @Test func goingPastTheThirdExpandsToNineAndKeepsTheSelectionMoving() throws {
        let h = try Self.converting()
        h.press(.down)
        let third = try #require(Self.panel(h.press(.down)))
        #expect(third.candidates.count == 3 && third.selected == 2)
        let expanded = try #require(Self.panel(h.press(.down)))
        #expect(expanded.candidates.count == 9)
        #expect(expanded.selected == 3)
        #expect(expanded.reservedRows == 9)
        #expect(h.converting?.selected == 3)
    }

    @Test func spaceExpandsInTheSameWayAsDown() throws {
        let h = try Self.converting()
        h.press(.space)
        h.press(.space)
        let expanded = try #require(Self.panel(h.press(.space)))
        #expect(expanded.candidates.count == 9)
    }

    @Test func movingBackStaysExpanded() throws {
        let h = try Self.converting()
        for _ in 0..<3 { h.press(.down) }
        let back = try #require(Self.panel(h.press(.up)))
        #expect(back.candidates.count == 9)
        #expect(back.selected == 2)
    }

    @Test func afterNineTheNextPageShowsNineAtATime() throws {
        let h = try Self.converting()
        for _ in 0..<8 { h.press(.down) }
        #expect(h.converting?.selected == 8)
        let next = try #require(Self.panel(h.press(.down)))
        #expect(next.candidates.map(\.surface) == (1...9).map { "b\($0)" })
        #expect(next.selected == 0)
        #expect(h.converting?.pageIndex == 1)
        // 前のページの最後（広げた状態）に戻る
        let back = try #require(Self.panel(h.press(.up)))
        #expect(back.candidates.count == 9 && back.selected == 8)
    }

    @Test func numbersAreOneToThreeUntilExpandedThenOneToNine() throws {
        let h = try Self.converting()
        // 広げる前は 4〜9 は受け止めて何もしない
        #expect(h.press(.character("5")) == [.consume])
        #expect(h.converting?.selected == 0)
        for _ in 0..<3 { h.press(.down) }
        // 広げたあとは 1〜9
        let actions = h.press(.character("5"))
        #expect(actions.contains(.commit("a5")))
    }

    @Test func numberKeysWithinTheFirstThreeCommit() throws {
        let h = try Self.converting()
        #expect(h.press(.character("3")).contains(.commit("a3")))
    }

    @Test func aShortPageDoesNotReserveMoreRowsThanItHas() throws {
        let engine = Self.engine(pages: [Self.surfaces("a", 5)])
        let h = try Harness(engine: engine)
        h.type("hi")
        let model = try #require(Self.panel(h.press(.space)))
        #expect(model.candidates.count == 3)
        #expect(model.reservedRows == 5)
        h.press(.down)
        h.press(.down)
        let expanded = try #require(Self.panel(h.press(.down)))
        #expect(expanded.candidates.count == 5 && expanded.reservedRows == 5)
    }

    @Test func aPageOfThreeOrFewerNeverExpands() throws {
        let h = try Harness(engine: Self.engine(pages: [Self.surfaces("a", 3)]))
        h.type("hi")
        h.press(.space)
        h.press(.down)
        let last = try #require(Self.panel(h.press(.down)))
        #expect(last.candidates.count == 3 && last.selected == 2)
        // paging.wrap（既定 true）で先頭に戻る
        #expect(Self.panel(h.press(.down))?.selected == 0)
    }

    @Test func theSettingSetsHowManyAreShownFirst() throws {
        let h = try Harness(settings: Settings(pageSize: 5), engine: Self.engine())
        h.type("hi")
        let model = try #require(Self.panel(h.press(.space)))
        #expect(model.candidates.count == 5)
        for _ in 0..<4 { h.press(.down) }
        #expect(Self.panel(h.press(.down))?.candidates.count == 9)
    }

    @Test func aPageSizeOfNineShowsEverythingFromTheStart() throws {
        let h = try Harness(settings: Settings(pageSize: 9), engine: Self.engine())
        h.type("hi")
        let model = try #require(Self.panel(h.press(.space)))
        #expect(model.candidates.count == 9)
    }

    @Test func theDefaultIsThree() {
        #expect(Settings.default.pageSize == 3)
    }

    @Test func movingTheFocusToAnotherSegmentStartsCollapsedAgain() throws {
        let h = try SegmentFixture.converting()
        h.press(.right)
        let model = try #require(Self.panel(h.press(.left)))
        #expect(model.candidates.count <= Settings.defaultPageSize)
    }
}
