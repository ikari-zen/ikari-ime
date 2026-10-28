// 文節の移動、区切り直し、文節ごとの選択と確定（ADR 0006 §8.2 の表、§16.1-3、§16.1-5）。
import Foundation
import Testing

@testable import IkariShellCore

/// 「はじめてしった」→ 初めて｜知った の 2 文節を返すエンジン
enum SegmentFixture {
    static let conversionID: UInt64 = 11
    /// FakeComposer で打てるローマ字。かなは はじめてしった
    static let typedReading = "hajimetesitta"

    static let head0 = [CandidateItem(surface: "初めて"), CandidateItem(surface: "始めて"), CandidateItem(surface: "はじめて", kind: .hiragana)]
    static let head1 = [CandidateItem(surface: "ハジメテ", kind: .katakana), CandidateItem(surface: "肇て")]
    static let tail0 = [CandidateItem(surface: "知った"), CandidateItem(surface: "しった", kind: .hiragana), CandidateItem(surface: "絞った")]
    static let tail1 = [CandidateItem(surface: "シッタ", kind: .katakana), CandidateItem(surface: "汁た")]

    static func page(_ segment: Int, _ index: Int) throws -> SegmentPage {
        let table: [[[CandidateItem]]] = [[head0, head1], [tail0, tail1]]
        guard table.indices.contains(segment), table[segment].indices.contains(index) else {
            throw EngineClientError.engine(.badRequest)
        }
        return SegmentPage(
            segment: segment, pageIndex: index, candidates: table[segment][index], hasNext: index + 1 < table[segment].count)
    }

    static func layout(rev: UInt32 = 0) throws -> SegmentLayoutInfo {
        SegmentLayoutInfo(
            conversionID: conversionID, layoutRev: rev,
            segments: [SegmentSummary(reading: "はじめて", surface: "初めて"), SegmentSummary(reading: "しった", surface: "知った")],
            focused: 0, focusedPage: try page(0, 0))
    }

    /// 読み全体の候補（最初の Space で出る。ADR 0006 §16.6）。0 番は最初の区切りの表記、2 番は文節の候補の組み合わせでは作れない
    static let wholeCandidates = [
        CandidateItem(surface: "初めて知った"), CandidateItem(surface: "始めてしった"), CandidateItem(surface: "はじめて仕った"),
        CandidateItem(surface: "初めてシッタ"),
    ]

    static func convertReply(layout: SegmentLayoutInfo?) -> ConversionPage {
        ConversionPage(
            conversionID: conversionID, pageIndex: 0, candidates: wholeCandidates, hasNext: false, layout: layout)
    }

    static func install(on engine: MockEngine) {
        engine.onConvert = { _ in convertReply(layout: try layout()) }
        engine.onSegmentPage = { _, segment, index in try page(segment, index) }
    }

    /// 状態機械が作る候補ウィンドウの内容。次のページがあれば、位置を決める高さはページの上限（9）で見込む
    static func model(
        _ candidates: [CandidateItem], selected: Int = 0, hasNext: Bool = true, pageCount: Int? = nil
    ) -> CandidatePanelModel {
        CandidatePanelModel(
            candidates: candidates, selected: selected, annotation: candidates[selected].annotation,
            reservedRows: hasNext ? Settings.enginePageSize : pageCount)
    }

    /// 最初の Space のあと（読み全体の候補を出している）まで進める
    static func wholeCandidates(_ settings: Settings = .default, secure: Bool = false) throws -> Harness {
        let engine = MockEngine()
        install(on: engine)
        let h = try Harness(settings: settings, secure: secure, engine: engine)
        h.type(typedReading)
        h.press(.space)
        return h
    }

    /// 文節のモード（2 文節、フォーカスは先頭）まで進める。Space のあと ← で入る（表示は変わらない）
    static func converting(_ settings: Settings = .default, secure: Bool = false) throws -> Harness {
        let h = try wholeCandidates(settings, secure: secure)
        h.press(.left)
        return h
    }
}

extension MockEngine {
    var segmentPageCalls: [Call] { calls.filter { if case .segmentPage = $0 { true } else { false } } }
    var resizeCalls: [Call] { calls.filter { if case .resize = $0 { true } else { false } } }
    var commitSegmentsCalls: [Call] { calls.filter { if case .commitSegments = $0 { true } else { false } } }
    var plainCommitCalls: [Call] { calls.filter { if case .commit = $0 { true } else { false } } }
}

private func marked(_ text: String, _ lengths: [Int], focused: Int) -> MarkedText {
    MarkedText(text: text, segments: lengths, focused: focused)
}

@Suite struct SegmentDisplayTests {
    @Test func aTwoSegmentConversionIsShownWithTheFocusOnTheFirstSegment() throws {
        let h = try SegmentFixture.converting()
        let state = try #require(h.converting)
        #expect(state.segments.map(\.reading) == ["はじめて", "しった"])
        #expect(state.focused == 0)
        #expect(state.conversionID == SegmentFixture.conversionID)
        #expect(state.candidates == SegmentFixture.head0)
        // 文節 1 は、フォーカスするまでエンジンに問い合わせない
        #expect(h.engine.segmentPageCalls.isEmpty)
        #expect(h.engine.convertRequests.count == 1)
    }

    @Test func theFirstDisplayIsTheWholeCandidatesWithClauseMarksAndNoFocus() throws {
        let engine = MockEngine()
        SegmentFixture.install(on: engine)
        let h = try Harness(engine: engine)
        h.type(SegmentFixture.typedReading)
        let actions = h.press(.space)
        #expect(
            actions == [
                .setMarkedText(MarkedText(text: "初めて知った", segments: [3, 3], focused: nil)),
                .showCandidates(
                    SegmentFixture.model(
                        Array(SegmentFixture.wholeCandidates.prefix(Settings.defaultPageSize)), hasNext: false,
                        pageCount: SegmentFixture.wholeCandidates.count)),
            ])
        let state = try #require(h.converting)
        #expect(state.segments.count == 1)
        #expect(state.wholeLayout?.segments.count == 2)
        // 文節の候補は問い合わせない
        #expect(h.engine.segmentPageCalls.isEmpty)
    }

    @Test func segmentLengthsAreUtf16UnitsNotCharacters() throws {
        let engine = MockEngine()
        engine.onConvert = { _ in
            let layout = SegmentLayoutInfo(
                conversionID: 5, layoutRev: 0,
                segments: [SegmentSummary(reading: "はじめて", surface: "😀"), SegmentSummary(reading: "しった", surface: "知った")],
                focused: 0, focusedPage: SegmentPage(segment: 0, pageIndex: 0, candidates: [CandidateItem(surface: "😀")], hasNext: false))
            return ConversionPage(conversionID: 5, pageIndex: 0, candidates: [CandidateItem(surface: "😀知った")], hasNext: false, layout: layout)
        }
        let h = try Harness(engine: engine)
        h.type(SegmentFixture.typedReading)
        let actions = h.press(.space)
        guard case .setMarkedText(let text) = actions.first else {
            Issue.record("expected marked text")
            return
        }
        #expect(text.segments == [2, 3])
        #expect(text.segments.reduce(0, +) == text.text.utf16.count)
    }

    @Test func aLayoutWhoseReadingsDoNotAddUpToTheTypedReadingIsIgnored() throws {
        let engine = MockEngine()
        engine.onConvert = { _ in
            var layout = try SegmentFixture.layout()
            layout.segments[1].reading = "ちがう"
            return SegmentFixture.convertReply(layout: layout)
        }
        let h = try Harness(engine: engine)
        h.type(SegmentFixture.typedReading)
        let actions = h.press(.space)
        // 古いエンジンと同じ 1 文節（全体の第一候補）
        #expect(actions.first == .setMarkedText(MarkedText(text: "初めて知った", highlightedLength: 6)))
        #expect(h.converting?.segments.count == 1)
        #expect(h.press(.right) == [.consume])
    }
}

/// 最初の Space は読み全体の候補を出す（ADR 0006 §16.6）。文節の操作のキーで文節のモードに入る
@Suite struct WholeCandidatesFirstTests {
    @Test func aNumberKeyCommitsTheWholeCandidateEvenWhenThereAreTwoSegments() throws {
        let h = try SegmentFixture.wholeCandidates()
        let actions = h.press(.character("2"))
        #expect(actions == [.commit("始めてしった"), .hideCandidates])
        #expect(h.engine.plainCommitCalls == [.commit(conversionID: 11, candidateIndex: 1)])
        #expect(h.engine.commitSegmentsCalls.isEmpty)
        #expect(h.machine.state == .idle)
    }

    @Test func enterCommitsTheSelectedWholeCandidateWithTheLegacyCommit() throws {
        let h = try SegmentFixture.wholeCandidates()
        h.press(.space)
        #expect(h.press(.enter) == [.commit("始めてしった"), .hideCandidates])
        #expect(h.engine.plainCommitCalls == [.commit(conversionID: 11, candidateIndex: 1)])
        #expect(h.engine.commitSegmentsCalls.isEmpty)
    }

    @Test func spaceStepsThroughTheWholeCandidatesAndTheClauseMarksFollowTheFirstSurface() throws {
        let h = try SegmentFixture.wholeCandidates()
        let second = h.press(.space)
        // 区切りは最初の表記のものなので、別の表記には付けない
        #expect(second.first == .setMarkedText(MarkedText(text: "始めてしった", highlightedLength: 6)))
        h.press(.space, [.shift])
        #expect(h.converting?.selected == 0)
        #expect(h.engine.segmentPageCalls.isEmpty)
    }

    @Test func rightEntersTheClauseModeAndMovesTheFocusToTheSecondSegment() throws {
        let h = try SegmentFixture.wholeCandidates()
        let actions = h.press(.right)
        #expect(
            actions == [
                .setMarkedText(marked("初めて知った", [3, 3], focused: 1)),
                .showCandidates(SegmentFixture.model(SegmentFixture.tail0)),
            ])
        let state = try #require(h.converting)
        #expect(state.wholeLayout == nil)
        #expect(state.focused == 1)
        // 文節のモードの数字キーは選ぶだけ（§16.1-3）
        h.press(.character("2"))
        #expect(h.machine.state != .idle)
        #expect(h.engine.plainCommitCalls.isEmpty)
        h.press(.enter)
        #expect(
            h.engine.commitSegmentsCalls == [
                .commitSegments(conversionID: 11, layoutRev: 0, choices: [.candidate(0), .candidate(1)])
            ])
    }

    @Test func leftEntersTheClauseModeOnTheFirstSegmentWithoutChangingTheDisplay() throws {
        let h = try SegmentFixture.wholeCandidates()
        let actions = h.press(.left)
        #expect(
            actions == [
                .setMarkedText(marked("初めて知った", [3, 3], focused: 0)),
                .showCandidates(SegmentFixture.model(SegmentFixture.head0)),
            ])
        #expect(h.engine.segmentPageCalls.isEmpty)
    }

    @Test func aWholeSelectionThatTheSegmentsCanFormIsCarriedIntoTheClauseMode() throws {
        let h = try SegmentFixture.wholeCandidates()
        h.press(.space)  // 始めてしった = 始めて(1) + しった(1)
        let actions = h.press(.left)
        #expect(
            actions == [
                .setMarkedText(marked("始めてしった", [3, 3], focused: 0)),
                .showCandidates(SegmentFixture.model(SegmentFixture.head0, selected: 1)),
            ])
        let state = try #require(h.converting)
        #expect(state.segments.map(\.candidateIndex) == [1, 1])
        // 文節の候補は組み合わせを探すために取る
        #expect(h.engine.segmentPageCalls.count == 1)
        h.press(.enter)
        #expect(
            h.engine.commitSegmentsCalls == [
                .commitSegments(conversionID: 11, layoutRev: 0, choices: [.candidate(1), .candidate(1)])
            ])
    }

    /// 実機の形: 全体の候補の 0 番（LM で並べ替えたもの）が、区切りの表記をつないだものと違う
    private static func wholeCandidatesDiffering() throws -> Harness {
        let engine = MockEngine()
        SegmentFixture.install(on: engine)
        engine.onConvert = { _ in
            ConversionPage(
                conversionID: SegmentFixture.conversionID, pageIndex: 0,
                candidates: [CandidateItem(surface: "始めて知った"), CandidateItem(surface: "初めて知った")], hasNext: false,
                layout: try SegmentFixture.layout())
        }
        let h = try Harness(settings: .default, secure: false, engine: engine)
        h.type(SegmentFixture.typedReading)
        h.press(.space)
        return h
    }

    @Test func rightEntersTheClauseModeEvenWhenTheShownWholeCandidateIsNotMadeOfTheSegments() throws {
        let h = try Self.wholeCandidatesDiffering()
        let actions = h.press(.right)
        #expect(actions != [.consume])
        let state = try #require(h.converting)
        #expect(state.wholeLayout == nil)
        #expect(state.focused == 1)
        #expect(h.engine.segmentPageCalls.count >= 1)
        #expect(actions.contains(.showCandidates(SegmentFixture.model(SegmentFixture.tail0))))
    }

    @Test func leftEntersTheClauseModeEvenWhenTheShownWholeCandidateIsNotMadeOfTheSegments() throws {
        let h = try Self.wholeCandidatesDiffering()
        let actions = h.press(.left)
        #expect(actions != [.consume])
        let state = try #require(h.converting)
        #expect(state.wholeLayout == nil)
        #expect(state.focused == 0)
        #expect(actions.contains(.showCandidates(SegmentFixture.model(SegmentFixture.head0, selected: 1))))
    }

    /// 実機と同じ形の入力（キーコード 123/124、フラグ 0xa00000）を、変換する前から殻の入口まで通す
    private static func deviceArrow(_ keyCode: UInt16, flags: UInt = 0xa0_0000) -> KeyInput {
        KeyCodes.keyInput(
            keyCode: keyCode, characters: "\u{F702}", modifiers: KeyCodes.modifiers(fromFlagBits: flags), inputMethod: .romaji)
    }

    @Test func deviceShapedArrowsEnterTheClauseModeFromTheWholeCandidates() throws {
        let right = try SegmentFixture.wholeCandidates()
        let rightActions = right.machine.handle(Self.deviceArrow(KeyCodes.rightArrow), surrounding: .unavailable)
        #expect(rightActions.contains(.setMarkedText(marked("初めて知った", [3, 3], focused: 1))))
        #expect(right.engine.segmentPageCalls.count == 1)

        let left = try SegmentFixture.wholeCandidates()
        let leftActions = left.machine.handle(Self.deviceArrow(KeyCodes.leftArrow), surrounding: .unavailable)
        #expect(leftActions.contains(.setMarkedText(marked("初めて知った", [3, 3], focused: 0))))
        #expect(left.converting?.focused == 0 && left.converting?.wholeLayout == nil)
    }

    @Test func aWholeSelectionThatTheSegmentsCannotFormStillEntersTheClauseModeWithTheFirstCandidates() throws {
        let h = try SegmentFixture.wholeCandidates()
        h.press(.space)
        h.press(.space)  // はじめて仕った: 文節の候補では作れない
        let actions = h.press(.right)
        #expect(
            actions == [
                .setMarkedText(marked("初めて知った", [3, 3], focused: 1)),
                .showCandidates(SegmentFixture.model(SegmentFixture.tail0)),
            ])
        let state = try #require(h.converting)
        #expect(state.wholeLayout == nil)
        #expect(state.segments.map(\.candidateIndex) == [0, 0])
    }

    @Test func leftEntersOnTheFirstSegmentWhenTheShownWholeCandidateCannotBeFormed() throws {
        let h = try SegmentFixture.wholeCandidates()
        h.press(.space)
        h.press(.space)
        let actions = h.press(.left)
        #expect(
            actions == [
                .setMarkedText(marked("初めて知った", [3, 3], focused: 0)),
                .showCandidates(SegmentFixture.model(SegmentFixture.head0)),
            ])
        #expect(h.converting?.wholeLayout == nil)
    }

    @Test func aFailedFetchWhileCarryingTheSelectionKeepsTheWholeCandidates() throws {
        let h = try SegmentFixture.wholeCandidates()
        h.press(.space)
        h.engine.onSegmentPage = { _, _, _ in throw EngineClientError.timeout }
        let before = h.converting
        #expect(h.press(.right) == [.consume])
        #expect(h.converting == before)
        // 全体の候補のままなので、確定は全体の確定
        h.press(.enter)
        #expect(h.engine.plainCommitCalls.count == 1)
    }

    @Test func shiftArrowsResegmentFromTheFirstSegmentAndDropTheWholeSelection() throws {
        let h = try SegmentFixture.wholeCandidates()
        h.press(.space)
        h.engine.onResize = { _, _, _ in
            SegmentLayoutInfo(
                conversionID: 11, layoutRev: 1,
                segments: [SegmentSummary(reading: "はじめ", surface: "始め"), SegmentSummary(reading: "てしった", surface: "手しった")],
                focused: 0,
                focusedPage: SegmentPage(
                    segment: 0, pageIndex: 0, candidates: [CandidateItem(surface: "始め"), CandidateItem(surface: "初め")], hasNext: false))
        }
        let actions = h.press(.left, [.shift])
        #expect(actions.first == .setMarkedText(marked("始め手しった", [2, 4], focused: 0)))
        #expect(h.engine.resizeCalls.count == 1)
        let state = try #require(h.converting)
        #expect(state.wholeLayout == nil)
        #expect(state.layoutRev == 1)
        h.press(.enter)
        #expect(
            h.engine.commitSegmentsCalls == [
                .commitSegments(conversionID: 11, layoutRev: 1, choices: [.candidate(0), .candidate(0)])
            ])
    }

    @Test func aFailedResizeKeepsTheWholeCandidatesAndTheirSelection() throws {
        let h = try SegmentFixture.wholeCandidates()
        h.press(.space)
        h.engine.onResize = { _, _, _ in throw EngineClientError.timeout }
        let before = h.converting
        #expect(h.press(.right, [.shift]) == [.consume])
        #expect(h.converting == before)
        #expect(h.converting?.selected == 1)
    }

    @Test func escapeFromTheWholeCandidatesReturnsToTheReading() throws {
        let h = try SegmentFixture.wholeCandidates()
        h.press(.escape)
        #expect(h.machine.state == .composing(kana: "はじめてしった", pending: ""))
        #expect(h.engine.calls.contains(.cancel(11)))
    }

    @Test func aScriptKeyConvertsTheWholeReadingWhileTheWholeCandidatesAreShown() throws {
        let h = try SegmentFixture.wholeCandidates()
        h.press(.function(7))
        #expect(h.converting?.candidates.map(\.surface) == ["ハジメテシッタ"])
        #expect(h.converting?.wholeLayout == nil)
    }
}

@Suite struct SegmentFocusTests {
    @Test func rightMovesTheFocusAndFetchesTheFirstPageOfThatSegmentOnce() throws {
        let h = try SegmentFixture.converting()
        let actions = h.press(.right)
        #expect(
            actions == [
                .setMarkedText(marked("初めて知った", [3, 3], focused: 1)),
                .showCandidates(SegmentFixture.model(SegmentFixture.tail0)),
            ])
        #expect(h.converting?.focused == 1)
        #expect(
            h.engine.segmentPageCalls == [
                .segmentPage(
                    conversionID: 11, layoutRev: 0, segment: 1, index: 0, left: [.candidate(0)],
                    EngineTimeouts.convertTimeout(readingCharacterCount: 3))
            ])
        // 戻って、また行っても、取得済みのページを使う
        h.press(.left)
        h.press(.right)
        #expect(h.engine.segmentPageCalls.count == 1)
    }

    @Test func theFocusWrapsAroundAtBothEnds() throws {
        let h = try SegmentFixture.converting()
        h.press(.right)  // 文節のモードに入って、最後の文節（1）へ
        #expect(h.converting?.focused == 1)
        // 最後で → は、先頭の文節へ
        let toFirst = h.press(.right)
        #expect(toFirst.first == .setMarkedText(marked("初めて知った", [3, 3], focused: 0)))
        #expect(h.converting?.focused == 0)
        // 先頭で ← は、最後の文節へ
        let toLast = h.press(.left)
        #expect(toLast.first == .setMarkedText(marked("初めて知った", [3, 3], focused: 1)))
        #expect(h.converting?.focused == 1)
    }

    @Test func movingTheFocusDoesNotChangeWhatIsDisplayed() throws {
        let h = try SegmentFixture.converting()
        h.press(.space)  // 文節 0 を 始めて に
        let actions = h.press(.right)
        #expect(actions.first == .setMarkedText(marked("始めて知った", [3, 3], focused: 1)))
        let back = h.press(.left)
        // 戻ると、選んでいた候補が選ばれたまま
        #expect(back == [.setMarkedText(marked("始めて知った", [3, 3], focused: 0)), .showCandidates(SegmentFixture.model(SegmentFixture.head0, selected: 1))])
    }

    @Test func aFailedFetchKeepsTheFocus() throws {
        let h = try SegmentFixture.converting()
        h.engine.onSegmentPage = { _, _, _ in throw EngineClientError.timeout }
        #expect(h.press(.right) == [.consume])
        #expect(h.converting?.focused == 0)
    }

    @Test func leftChoicesCarryTheCandidatesChosenOnTheLeft() throws {
        let h = try SegmentFixture.converting()
        h.press(.space)  // 文節 0: 始めて（通し番号 1）
        h.press(.right)
        guard case .segmentPage(_, _, _, _, let left, _)? = h.engine.segmentPageCalls.last else {
            Issue.record("no segment page")
            return
        }
        #expect(left == [.candidate(1)])
    }

    @Test func spaceStepsThroughTheFocusedSegmentOnly() throws {
        let h = try SegmentFixture.converting()
        h.press(.right)
        let actions = h.press(.space)
        #expect(
            actions == [
                .setMarkedText(marked("初めてしった", [3, 3], focused: 1)),
                .moveSelection(SegmentFixture.model(SegmentFixture.tail0, selected: 1)),
            ])
        #expect(h.press(.space, [.shift]).first == .setMarkedText(marked("初めて知った", [3, 3], focused: 1)))
        // 文節 0 は動かない
        #expect(h.converting?.segments[0].selected == 0)
    }

    @Test func spacePastThePageFetchesTheNextPageOfThatSegmentWithTheSegmentRequest() throws {
        let h = try SegmentFixture.converting()
        h.press(.right)
        for _ in 0..<3 { h.press(.space) }  // 知った → しった → 絞った → 次のページ
        let state = try #require(h.converting)
        #expect(state.pageIndex == 1)
        #expect(state.candidates == SegmentFixture.tail1)
        #expect(state.candidateOffset == 3)
        #expect(
            h.engine.segmentPageCalls.last
                == .segmentPage(
                    conversionID: 11, layoutRev: 0, segment: 1, index: 1, left: [.candidate(0)],
                    EngineTimeouts.convertTimeout(readingCharacterCount: 3)))
        #expect(h.engine.count(where: { if case .page = $0 { true } else { false } }) == 0)
    }

    @Test func theLastPageWrapsToTheFirstPageOfTheSegment() throws {
        let h = try SegmentFixture.converting()
        for _ in 0..<5 { h.press(.space) }  // 初めて → 始めて → はじめて → ハジメテ → 肇て → (折り返し)
        #expect(h.converting?.pageIndex == 0)
        #expect(h.converting?.selected == 0)
        #expect(h.converting?.segments[0].candidateIndex == 0)
    }

    @Test func aSingleSegmentLayoutKeepsTheLegacyPagingAndCommit() throws {
        let engine = MockEngine()
        engine.onConvert = { _ in
            let layout = SegmentLayoutInfo(
                conversionID: 7, layoutRev: 0, segments: [SegmentSummary(reading: "ひんし", surface: "品詞")], focused: 0,
                focusedPage: nil)
            var page = MockEngine.defaultPages[0]
            page.layout = layout
            return page
        }
        let h = try Harness(engine: engine)
        h.type("hinsi")
        let actions = h.press(.space)
        #expect(actions.first == .setMarkedText(MarkedText(text: "品詞", highlightedLength: 2)))
        for _ in 0..<3 { h.press(.space) }
        #expect(h.engine.calls.contains(.page(conversionID: 7, index: 1, EngineTimeouts.convertTimeout(readingCharacterCount: 3))))
        #expect(h.engine.segmentPageCalls.isEmpty)
        h.press(.enter)
        #expect(h.engine.plainCommitCalls == [.commit(conversionID: 7, candidateIndex: 3)])
        #expect(h.engine.commitSegmentsCalls.isEmpty)
    }
}

@Suite struct SegmentSelectionTests {
    @Test func aNumberKeyOnTwoSegmentsOnlyChoosesAndMovesToTheNextSegment() throws {
        let h = try SegmentFixture.converting()
        let actions = h.press(.character("2"))
        #expect(
            actions == [
                .setMarkedText(marked("始めて知った", [3, 3], focused: 1)),
                .showCandidates(SegmentFixture.model(SegmentFixture.tail0)),
            ])
        #expect(h.converting?.segments[0].selected == 1)
        #expect(h.converting?.focused == 1)
        // 確定しない
        #expect(h.engine.plainCommitCalls.isEmpty && h.engine.commitSegmentsCalls.isEmpty)
        #expect(h.engine.segmentPageCalls.count == 1)
    }

    @Test func aNumberKeyOnTheLastSegmentChoosesWithoutMoving() throws {
        let h = try SegmentFixture.converting()
        h.press(.character("1"))
        let actions = h.press(.character("3"))
        #expect(
            actions == [
                .setMarkedText(marked("初めて絞った", [3, 3], focused: 1)),
                .moveSelection(SegmentFixture.model(SegmentFixture.tail0, selected: 2)),
            ])
        #expect(h.converting?.focused == 1)
        #expect(h.machine.state != .idle)
        #expect(h.engine.commitSegmentsCalls.isEmpty)
    }

    @Test func jisDigitKeysBehaveTheSame() throws {
        let engine = MockEngine()
        engine.onConvert = { _ in
            // かな入力で打てる「ちの」（FakeComposer の jisTable）を 地｜野 の 2 文節にする
            let layout = SegmentLayoutInfo(
                conversionID: 31, layoutRev: 0,
                segments: [SegmentSummary(reading: "ち", surface: "地"), SegmentSummary(reading: "の", surface: "野")], focused: 0,
                focusedPage: SegmentPage(
                    segment: 0, pageIndex: 0, candidates: [CandidateItem(surface: "地"), CandidateItem(surface: "血")],
                    hasNext: false))
            return ConversionPage(conversionID: 31, pageIndex: 0, candidates: [CandidateItem(surface: "地野")], hasNext: false, layout: layout)
        }
        engine.onSegmentPage = { _, segment, index in
            SegmentPage(segment: segment, pageIndex: index, candidates: [CandidateItem(surface: "野"), CandidateItem(surface: "之")], hasNext: false)
        }
        let h = try Harness(settings: Settings(inputMethod: .kana), engine: engine)
        h.press(.jis(22))
        h.press(.jis(32))
        h.press(.space)
        h.press(.left)  // 文節のモードに入る（§16.6）
        #expect(h.converting?.segments.count == 2)
        let actions = h.press(.jis(1))  // 数字の 2
        #expect(actions.first == .setMarkedText(marked("血野", [1, 1], focused: 1)))
        #expect(h.converting?.segments[0].selected == 1)
        #expect(h.converting?.focused == 1)
        #expect(h.engine.commitSegmentsCalls.isEmpty)
    }

    @Test func aNumberBeyondTheCandidatesOfThePageIsIgnored() throws {
        let h = try SegmentFixture.converting()
        let before = h.converting
        #expect(h.press(.character("9")) == [.consume])
        #expect(h.converting == before)
    }

    @Test func aNumberKeyStillChoosesWhenMovingToTheNextSegmentFails() throws {
        let h = try SegmentFixture.converting()
        h.engine.onSegmentPage = { _, _, _ in throw EngineClientError.timeout }
        let actions = h.press(.character("2"))
        // 選択は残り、フォーカスは動かない
        #expect(actions.first == .setMarkedText(marked("始めて知った", [3, 3], focused: 0)))
        #expect(h.converting?.segments[0].selected == 1)
        #expect(h.converting?.focused == 0)
    }

    @Test func enterCommitsEverySegmentWithTheChoicesAndReportsThemToTheEngine() throws {
        let h = try SegmentFixture.converting()
        h.press(.character("2"))  // 始めて、文節 1 へ
        h.press(.space)  // しった
        let actions = h.press(.enter)
        #expect(actions == [.commit("始めてしった"), .hideCandidates])
        #expect(
            h.engine.commitSegmentsCalls == [
                .commitSegments(conversionID: 11, layoutRev: 0, choices: [.candidate(1), .candidate(1)])
            ])
        #expect(h.engine.plainCommitCalls.isEmpty)
        #expect(h.machine.state == .idle)
    }

    @Test func enterWithoutVisitingASegmentSendsItsFirstCandidate() throws {
        let h = try SegmentFixture.converting()
        h.press(.space)
        #expect(h.press(.enter) == [.commit("始めて知った"), .hideCandidates])
        #expect(
            h.engine.commitSegmentsCalls == [
                .commitSegments(conversionID: 11, layoutRev: 0, choices: [.candidate(1), .candidate(0)])
            ])
    }

    @Test func choicesAcrossPagesUseTheCumulativeIndex() throws {
        let h = try SegmentFixture.converting()
        for _ in 0..<3 { h.press(.space) }  // 文節 0 の 2 ページ目の先頭（ハジメテ）
        h.press(.enter)
        #expect(
            h.engine.commitSegmentsCalls == [
                .commitSegments(conversionID: 11, layoutRev: 0, choices: [.candidate(3), .candidate(0)])
            ])
    }

    @Test func aSecureSessionCommitsWithoutTellingTheEngine() throws {
        // 変換中に secure になると、次のキーで今の表示のまま確定してから素通し（ADR 0026 D5、§5.4）
        let h = try SegmentFixture.converting()
        h.press(.right)
        h.machine.setSecure(true)
        #expect(h.press(.enter) == [.commit("初めて知った"), .hideCandidates, .passThrough])
        #expect(h.engine.commitSegmentsCalls.isEmpty && h.engine.plainCommitCalls.isEmpty)
    }

    @Test func escapeAndBackspaceReturnToTheWholeReadingAndDropTheSelections() throws {
        for key in [KeyInput.Key.escape, .backspace] {
            let h = try SegmentFixture.converting()
            h.press(.character("2"))
            let actions = h.press(key)
            #expect(h.machine.state == .composing(kana: "はじめてしった", pending: ""))
            #expect(actions == [.hideCandidates, .setMarkedText(MarkedText(text: "はじめてしった", highlightedLength: 7))])
            #expect(h.engine.calls.contains(.cancel(11)))
            // もう一度 Space で、最初から変換できる
            h.press(.space)
            #expect(h.converting?.focused == 0)
            #expect(h.converting?.segments[0].selected == 0)
        }
    }

    @Test func typingACharacterCommitsAllSegmentsAndStartsAComposition() throws {
        let h = try SegmentFixture.converting()
        h.press(.right)
        let actions = h.press(.character("k"))
        #expect(
            actions == [
                .commit("初めて知った"), .hideCandidates, .setMarkedText(MarkedText(text: "k", highlightedLength: 0)),
            ])
        #expect(h.engine.commitSegmentsCalls.count == 1)
    }

    @Test func aCommandShortcutCommitsFirstThenPassesThrough() throws {
        let h = try SegmentFixture.converting()
        #expect(h.press(.character("v"), [.command]) == [.commit("初めて知った"), .hideCandidates, .passThrough])
    }
}

@Suite struct SegmentScriptConversionTests {
    @Test func aScriptKeyConvertsOnlyTheFocusedSegmentAndKeepsTheConversion() throws {
        let h = try SegmentFixture.converting()
        let actions = h.press(.function(7))
        #expect(actions == [.hideCandidates, .setMarkedText(marked("ハジメテ知った", [4, 3], focused: 0))])
        #expect(!h.engine.calls.contains(.cancel(11)))
        #expect(h.converting?.focused == 0)
        #expect(h.converting?.segments.count == 2)
    }

    @Test func theLiteralIsCommittedAsALiteralChoiceAndTheOtherSegmentsKeepTheirs() throws {
        let h = try SegmentFixture.converting()
        h.press(.right)
        h.press(.function(8))  // 半角カタカナ
        #expect(h.converting?.segments[1].literal == "ｼｯﾀ")
        h.press(.enter)
        #expect(
            h.engine.commitSegmentsCalls == [
                .commitSegments(conversionID: 11, layoutRev: 0, choices: [.candidate(0), .literal("ｼｯﾀ")])
            ])
    }

    @Test func movingAwayAndBackKeepsTheLiteralAndReopensThePanel() throws {
        let h = try SegmentFixture.converting()
        h.press(.function(7))
        h.press(.right)
        let back = h.press(.left)
        #expect(back == [.setMarkedText(marked("ハジメテ知った", [4, 3], focused: 0)), .showCandidates(SegmentFixture.model(SegmentFixture.head0))])
    }

    @Test func spaceAfterAScriptConversionChoosesTheNextCandidateAndReopensThePanel() throws {
        let h = try SegmentFixture.converting()
        h.press(.function(7))
        let actions = h.press(.space)
        #expect(
            actions == [
                .setMarkedText(marked("始めて知った", [3, 3], focused: 0)),
                .showCandidates(SegmentFixture.model(SegmentFixture.head0, selected: 1)),
            ])
        #expect(h.converting?.segments[0].literal == nil)
    }

    @Test func aScriptKeyOnASingleSegmentConversionStillReplacesTheWholeReading() throws {
        let h = try Harness()
        h.type("hinsi")
        h.press(.space)
        h.press(.function(7))
        #expect(h.engine.calls.contains(.cancel(7)))
        #expect(h.converting?.candidates.map(\.surface) == ["ヒンシ"])
    }

    @Test func scriptKeysWorkWhenAnArrowKeyChordIsBoundToThem() throws {
        // 矢印は割り当てから引く: ctrl+left を字種変換に割り当てても動く
        let settings = Settings(keymapOverrides: [.toFullwidthKatakana: "ctrl+left"])
        let engine = MockEngine()
        SegmentFixture.install(on: engine)
        let h = try Harness(settings: settings, engine: engine)
        h.type(SegmentFixture.typedReading)
        h.press(.space)
        h.press(.left)  // 文節のモードに入る（§16.6）
        h.press(.left, [.control])
        #expect(h.converting?.segments[0].literal == "ハジメテ")
    }
}

@Suite struct SegmentResizeTests {
    private static let shrunk = SegmentLayoutInfo(
        conversionID: 11, layoutRev: 1,
        segments: [SegmentSummary(reading: "はじめ", surface: "始め"), SegmentSummary(reading: "てしった", surface: "手しった")],
        focused: 0,
        focusedPage: SegmentPage(
            segment: 0, pageIndex: 0, candidates: [CandidateItem(surface: "始め"), CandidateItem(surface: "初め")], hasNext: false))

    @Test func shiftLeftShrinksTheFocusedSegmentAndAdoptsTheNewLayout() throws {
        let h = try SegmentFixture.converting()
        h.engine.onResize = { _, _, _ in Self.shrunk }
        let actions = h.press(.left, [.shift])
        #expect(
            h.engine.resizeCalls == [
                .resize(
                    conversionID: 11, layoutRev: 0, segment: 0, delta: -1, left: [],
                    EngineTimeouts.convertTimeout(readingCharacterCount: 4))
            ])
        let state = try #require(h.converting)
        #expect(state.layoutRev == 1)
        #expect(state.segments.map(\.reading) == ["はじめ", "てしった"])
        #expect(state.focused == 0)
        #expect(
            actions == [
                .setMarkedText(marked("始め手しった", [2, 4], focused: 0)),
                .showCandidates(SegmentFixture.model(state.candidates, hasNext: state.hasNext)),
            ])
    }

    @Test func afterAResizeEveryRequestCarriesTheNewLayoutRevision() throws {
        let h = try SegmentFixture.converting()
        h.engine.onResize = { _, _, _ in Self.shrunk }
        h.engine.onSegmentPage = { _, segment, index in
            SegmentPage(segment: segment, pageIndex: index, candidates: [CandidateItem(surface: "手しった"), CandidateItem(surface: "てしった")], hasNext: false)
        }
        h.press(.left, [.shift])
        h.press(.right)
        guard case .segmentPage(_, let rev, _, _, _, _)? = h.engine.segmentPageCalls.last else {
            Issue.record("no segment page")
            return
        }
        #expect(rev == 1)
        h.press(.enter)
        #expect(
            h.engine.commitSegmentsCalls == [
                .commitSegments(conversionID: 11, layoutRev: 1, choices: [.candidate(0), .candidate(0)])
            ])
    }

    @Test func aResizedSingleSegmentUsesTheSegmentRequestsAndCommitsBySegments() throws {
        let engine = MockEngine()
        engine.onConvert = { _ in
            let layout = SegmentLayoutInfo(
                conversionID: 7, layoutRev: 0, segments: [SegmentSummary(reading: "ひんし", surface: "品詞")], focused: 0,
                focusedPage: nil)
            var page = MockEngine.defaultPages[0]
            page.layout = layout
            return page
        }
        engine.onResize = { _, _, _ in
            SegmentLayoutInfo(
                conversionID: 7, layoutRev: 1,
                segments: [SegmentSummary(reading: "ひん", surface: "品"), SegmentSummary(reading: "し", surface: "詞")], focused: 0,
                focusedPage: SegmentPage(segment: 0, pageIndex: 0, candidates: [CandidateItem(surface: "品")], hasNext: false))
        }
        let h = try Harness(engine: engine)
        h.type("hinsi")
        h.press(.space)
        h.press(.left, [.shift])
        #expect(h.converting?.segments.count == 2)
        h.press(.enter)
        #expect(h.engine.plainCommitCalls.isEmpty)
        #expect(h.engine.commitSegmentsCalls.count == 1)
    }

    @Test func shiftRightExpandsAndPassesTheLeftSegmentsChoices() throws {
        let h = try SegmentFixture.converting()
        h.press(.space)  // 文節 0 を 始めて に
        h.press(.right)
        h.engine.onResize = { _, _, _ in
            SegmentLayoutInfo(
                conversionID: 11, layoutRev: 1,
                segments: [SegmentSummary(reading: "はじめて", surface: "始めて"), SegmentSummary(reading: "しった", surface: "知った")],
                focused: 1, focusedPage: try SegmentFixture.page(1, 0))
        }
        // 最後の文節は伸ばせない: エンジンを呼ばない
        #expect(h.press(.right, [.shift]) == [.consume])
        #expect(h.engine.resizeCalls.isEmpty)
        h.press(.left)
        h.press(.right, [.shift])
        guard case .resize(_, _, let segment, let delta, let left, _)? = h.engine.resizeCalls.last else {
            Issue.record("no resize")
            return
        }
        #expect(segment == 0 && delta == 1 && left == [])
    }

    @Test func aOneCharacterSegmentCannotBeShrunk() throws {
        let engine = MockEngine()
        engine.onConvert = { _ in
            let layout = SegmentLayoutInfo(
                conversionID: 7, layoutRev: 0, segments: [SegmentSummary(reading: "ひ", surface: "日")], focused: 0,
                focusedPage: nil)
            var page = MockEngine.defaultPages[0]
            page.layout = layout
            return page
        }
        let h = try Harness(engine: engine)
        h.type("hi")
        h.press(.space)
        #expect(h.press(.left, [.shift]) == [.consume])
        #expect(h.press(.right, [.shift]) == [.consume])
        #expect(h.engine.resizeCalls.isEmpty)
    }

    @Test func theLeftSegmentsAreNotTouchedAndTheirChoicesAreSentWithTheRequest() throws {
        let h = try SegmentFixture.converting()
        h.press(.character("2"))  // 文節 0: 始めて、文節 1 へ
        h.engine.onResize = { _, _, _ in
            SegmentLayoutInfo(
                conversionID: 11, layoutRev: 1,
                segments: [
                    SegmentSummary(reading: "はじめて", surface: "始めて"), SegmentSummary(reading: "し", surface: "市"),
                    SegmentSummary(reading: "った", surface: "った"),
                ],
                focused: 1,
                focusedPage: SegmentPage(segment: 1, pageIndex: 0, candidates: [CandidateItem(surface: "市")], hasNext: false))
        }
        h.press(.left, [.shift])
        guard case .resize(_, _, let segment, let delta, let left, _)? = h.engine.resizeCalls.last else {
            Issue.record("no resize")
            return
        }
        #expect(segment == 1 && delta == -1 && left == [.candidate(1)])
        let state = try #require(h.converting)
        #expect(state.segments.map(\.reading) == ["はじめて", "し", "った"])
        #expect(state.segments[0].selected == 1)  // 左の選択は残る
        #expect(state.focused == 1)
        #expect(state.segments[1].candidateIndex == 0)
    }

    @Test(arguments: [
        EngineClientError.timeout, .unavailable, .invalidReply, .engine(.staleLayout), .engine(.badRequest),
    ])
    func aFailedResizeKeepsTheDisplayAsItIs(error: EngineClientError) throws {
        let h = try SegmentFixture.converting()
        h.engine.onResize = { _, _, _ in throw error }
        let before = h.converting
        #expect(h.press(.left, [.shift]) == [.consume])
        #expect(h.converting == before)
    }

    @Test func aResizeReplyThatChangesTheLeftSegmentsOrTheReadingIsNotTrusted() throws {
        let h = try SegmentFixture.converting()
        h.press(.right)
        let before = h.converting
        h.engine.onResize = { _, _, _ in
            SegmentLayoutInfo(  // 左の文節の読みが変わっている
                conversionID: 11, layoutRev: 1,
                segments: [SegmentSummary(reading: "はじ", surface: "はじ"), SegmentSummary(reading: "めてしった", surface: "")],
                focused: 1, focusedPage: try SegmentFixture.page(1, 0))
        }
        #expect(h.press(.left, [.shift]) == [.consume])
        #expect(h.converting == before)
    }

    @Test func anOldEngineWithoutALayoutIgnoresArrowsAndNeverSendsSegmentRequests() throws {
        let h = try Harness()
        h.type("hinsi")
        h.press(.space)
        for (key, modifiers) in [(KeyInput.Key.left, KeyModifiers()), (.right, []), (.left, [.shift]), (.right, [.shift])] {
            #expect(h.press(key, modifiers) == [.consume])
        }
        #expect(h.engine.resizeCalls.isEmpty && h.engine.segmentPageCalls.isEmpty)
        #expect(h.converting?.isFallback == false)
    }

    @Test func theShellFallbackIgnoresSegmentKeys() throws {
        let engine = MockEngine()
        engine.onConvert = { _ in throw EngineClientError.timeout }
        let h = try Harness(engine: engine)
        h.type("hinsi")
        h.press(.space)
        #expect(h.press(.left, [.shift]) == [.consume])
        #expect(h.press(.right) == [.consume])
        #expect(h.engine.resizeCalls.isEmpty)
    }
}

@Suite struct SegmentKeymapTests {
    @Test(arguments: [KeymapPreset.macos, .msIme])
    func bothPresetsUseTheArrowKeys(preset: KeymapPreset) throws {
        let engine = MockEngine()
        SegmentFixture.install(on: engine)
        let h = try Harness(preset: preset, engine: engine)
        h.type(SegmentFixture.typedReading)
        h.press(.space)
        h.press(.right)
        #expect(h.converting?.focused == 1)
        h.press(.left)
        #expect(h.converting?.focused == 0)
    }

    @Test func aSegmentActionCanBeBoundToAnotherKey() throws {
        let settings = Settings(keymapOverrides: [.segmentFocusRight: "ctrl+l", .segmentFocusLeft: "ctrl+h"])
        let engine = MockEngine()
        SegmentFixture.install(on: engine)
        let h = try Harness(settings: settings, engine: engine)
        h.type(SegmentFixture.typedReading)
        h.press(.space)
        h.press(.character("l"), [.control])
        #expect(h.converting?.focused == 1)
        h.press(.character("h"), [.control])
        #expect(h.converting?.focused == 0)
        // 矢印の割り当ては外れた（上書きでプリセットの矢印は消える）。受け止めて何もしない
        #expect(h.press(.right) == [.consume])
        #expect(h.converting?.focused == 0)
    }

    @Test func noneUnbindsAnActionAndTheArrowIsJustSwallowed() throws {
        let settings = Settings(keymapOverrides: [.segmentShrink: Settings.noneValue])
        let engine = MockEngine()
        SegmentFixture.install(on: engine)
        let h = try Harness(settings: settings, engine: engine)
        h.type(SegmentFixture.typedReading)
        h.press(.space)
        #expect(h.press(.left, [.shift]) == [.consume])
        #expect(h.engine.resizeCalls.isEmpty)
    }

    @Test func aSegmentKeyBoundToACharacterChordDoesNothingOutsideConverting() throws {
        let settings = Settings(keymapOverrides: [.segmentFocusLeft: "ctrl+h"])
        let h = try Harness(settings: settings)
        // idle: 通常どおりアプリへ
        #expect(h.press(.character("h"), [.control]) == [.passThrough])
        // 合成中: 確定してからアプリへ（今のコマンドキーの扱い）
        h.type("hi")
        #expect(h.press(.character("h"), [.control]) == [.commit("ひ"), .passThrough])
    }

    @Test func arrowsAreHandedToTheAppWhenIdleAndSwallowedWhileComposing() throws {
        let h = try Harness()
        #expect(h.press(.left) == [.passThrough])
        #expect(h.press(.right, [.shift]) == [.passThrough])
        h.type("hi")
        let before = h.machine.state
        #expect(h.press(.left) == [.consume])
        #expect(h.press(.right, [.shift]) == [.consume])
        #expect(h.machine.state == before)
        // 英字の一時入力中も同じ
        let ascii = try Harness()
        ascii.press(.character("A"), [.shift])
        #expect(ascii.press(.left) == [.consume])
    }

    @Test func anArrowWithACommandModifierWhileConvertingCommitsAndPassesThrough() throws {
        let h = try SegmentFixture.converting()
        #expect(h.press(.left, [.command]) == [.commit("初めて知った"), .hideCandidates, .passThrough])
    }
}
