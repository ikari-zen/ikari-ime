// 確定済みの文字列の再変換（ADR 0006 §9）と、Esc で元に戻す・取り残された変換中の文字列を守る（§16.3）。
import Foundation
import Testing

@testable import IkariShellCore

enum ReconvertFixture {
    static let conversionID: UInt64 = 21
    static let range = TextRange(location: 10, length: 2)
    static let selection = SelectedText(range: range, text: "品詞")
    /// Ctrl+Shift+R（Ctrl を伴うので、文字は押したキーの "r"）
    static let key = KeyInput.Key.character("r")
    static let modifiers: KeyModifiers = [.control, .shift]

    static let page0 = [CandidateItem(surface: "品詞"), CandidateItem(surface: "瀕死"), CandidateItem(surface: "ひんし", kind: .hiragana)]
    static let page1 = [CandidateItem(surface: "ヒンシ", kind: .katakana), CandidateItem(surface: "貧士")]

    static func layout(surface: String = "品詞", reading: String = "ひんし", rev: UInt32 = 0) -> SegmentLayoutInfo {
        SegmentLayoutInfo(
            conversionID: conversionID, layoutRev: rev, segments: [SegmentSummary(reading: reading, surface: surface)],
            focused: 0, focusedPage: SegmentPage(segment: 0, pageIndex: 0, candidates: page0, hasNext: true))
    }

    static func install(on engine: MockEngine) {
        engine.onReconvert = { _ in layout() }
        engine.onSegmentPage = { _, segment, index in
            guard index == 1 else { throw EngineClientError.engine(.badRequest) }
            return SegmentPage(segment: segment, pageIndex: 1, candidates: page1, hasNext: false)
        }
    }

    /// 再変換の変換中まで進める
    static func converting(secure: Bool = false) throws -> Harness {
        let engine = MockEngine()
        install(on: engine)
        let h = try Harness(secure: secure, engine: engine)
        h.selection = selection
        h.press(key, modifiers)
        return h
    }

    static var panel: CandidatePanelModel { SegmentFixture.model(page0) }
}

extension MockEngine {
    var reconvertRequests: [ReconvertRequest] {
        calls.compactMap { if case .reconvert(let request, _) = $0 { request } else { nil } }
    }
    var cancelledIDs: [UInt64] { calls.compactMap { if case .cancel(let id) = $0 { id } else { nil } } }
}

@Suite struct ReconversionStartTests {
    @Test func theKeyStartsAConversionThatShowsTheOriginalAndReplacesTheSelection() throws {
        let engine = MockEngine()
        ReconvertFixture.install(on: engine)
        let h = try Harness(engine: engine)
        h.selection = ReconvertFixture.selection
        let actions = h.press(ReconvertFixture.key, ReconvertFixture.modifiers)
        #expect(
            actions == [
                .setMarkedText(MarkedText(text: "品詞", highlightedLength: 2, replacementRange: ReconvertFixture.range)),
                .showCandidates(ReconvertFixture.panel),
            ])
        let state = try #require(h.converting)
        #expect(state.origin == .reconversion(original: "品詞", replacing: ReconvertFixture.range))
        #expect(state.reading == "ひんし")
        #expect(state.conversionID == ReconvertFixture.conversionID)
        #expect(state.segments.count == 1)
        let request = try #require(h.engine.reconvertRequests.first)
        #expect(request.sessionID == Harness.sessionID)
        #expect(request.surface == "品詞")
        #expect(request.context == SurroundingText(left: "ユーザー辞書に単語を登録するときは", available: true))
        #expect(request.pageSize == 9)
        guard case .reconvert(_, let timeout)? = h.engine.calls.last else {
            Issue.record("no reconvert call")
            return
        }
        #expect(timeout == EngineTimeouts.convertTimeout(readingCharacterCount: 2))
    }

    @Test func theSelectionIsOnlyReadWhenTheReconvertKeyIsPressed() throws {
        let h = try Harness()
        h.selection = ReconvertFixture.selection
        h.type("hinsi")
        h.press(.space)
        h.press(.enter)
        h.press(.function(7))
        h.press(.character("c"), [.command])
        #expect(h.selectionFetches == 0)
    }

    @Test(arguments: [
        ("no selection", nil as SelectedText?),
        ("empty range", SelectedText(range: TextRange(location: 3, length: 0), text: "")),
        ("over 64 characters", SelectedText(range: TextRange(location: 0, length: 65), text: String(repeating: "あ", count: 65))),
        ("newline", SelectedText(range: TextRange(location: 0, length: 5), text: "品詞\n瀕死")),
        ("carriage return", SelectedText(range: TextRange(location: 0, length: 3), text: "品\r詞")),
        ("control characters only", SelectedText(range: TextRange(location: 0, length: 2), text: "\u{07}\u{1B}")),
        ("not found location", SelectedText(range: TextRange(location: -1, length: 2), text: "品詞")),
    ])
    func refusesTheSelectionsThatSection92Excludes(name: String, selection: SelectedText?) throws {
        let h = try Harness()
        ReconvertFixture.install(on: h.engine)
        h.selection = selection
        #expect(h.press(ReconvertFixture.key, ReconvertFixture.modifiers) == [.consume], "\(name)")
        #expect(h.machine.state == .idle)
        #expect(h.engine.reconvertRequests.isEmpty)
    }

    @Test func exactlySixtyFourCharactersAreAccepted() throws {
        let text = String(repeating: "品", count: 64)
        let engine = MockEngine()
        engine.onReconvert = { _ in ReconvertFixture.layout(surface: text) }
        let h = try Harness(engine: engine)
        h.selection = SelectedText(range: TextRange(location: 0, length: 64), text: text)
        #expect(h.press(ReconvertFixture.key, ReconvertFixture.modifiers) != [.consume])
        #expect(h.converting != nil)
    }

    @Test func refusesWhenTheLeftSurroundingTextCannotBeRead() throws {
        let h = try Harness()
        ReconvertFixture.install(on: h.engine)
        h.selection = ReconvertFixture.selection
        h.surroundingText = .unavailable
        #expect(h.press(ReconvertFixture.key, ReconvertFixture.modifiers) == [.consume])
        #expect(h.engine.reconvertRequests.isEmpty)
        #expect(h.machine.state == .idle)
    }

    @Test func refusesInASecureSessionWithoutReadingTheSelection() throws {
        let h = try Harness(secure: true)
        ReconvertFixture.install(on: h.engine)
        h.selection = ReconvertFixture.selection
        // secure は日本語のモードでも素通し（ADR 0026 D5）。再変換もしない
        #expect(h.press(ReconvertFixture.key, ReconvertFixture.modifiers) == [.passThrough])
        #expect(h.selectionFetches == 0)
        #expect(h.surroundingFetches == 0)
        #expect(h.engine.reconvertRequests.isEmpty)
    }

    @Test func doesNothingWhileComposingOrConverting() throws {
        let h = try Harness()
        ReconvertFixture.install(on: h.engine)
        h.selection = ReconvertFixture.selection
        h.type("hi")
        #expect(h.press(ReconvertFixture.key, ReconvertFixture.modifiers) == [.consume])
        #expect(h.machine.state == .composing(kana: "ひ", pending: ""))
        h.press(.space)
        let before = h.converting
        #expect(h.press(ReconvertFixture.key, ReconvertFixture.modifiers) == [.consume])
        #expect(h.converting == before)
        #expect(h.engine.reconvertRequests.isEmpty)
    }

    @Test func anEngineFailureLeavesTheDocumentAlone() throws {
        let h = try Harness()
        h.selection = ReconvertFixture.selection
        for error in [EngineClientError.timeout, .unavailable, .engine(.badRequest)] {
            h.engine.onReconvert = { _ in throw error }
            #expect(h.press(ReconvertFixture.key, ReconvertFixture.modifiers) == [.consume])
            #expect(h.machine.state == .idle)
        }
    }

    @Test func aReplyWhoseSurfaceDiffersFromTheSelectionIsNeverShown() throws {
        let h = try Harness()
        h.selection = ReconvertFixture.selection
        h.engine.onReconvert = { _ in ReconvertFixture.layout(surface: "瀕死") }
        #expect(h.press(ReconvertFixture.key, ReconvertFixture.modifiers) == [.consume])
        #expect(h.machine.state == .idle)
        #expect(h.engine.cancelledIDs == [ReconvertFixture.conversionID])
    }

    @Test func theKeyCanBeRebound() throws {
        let engine = MockEngine()
        ReconvertFixture.install(on: engine)
        let h = try Harness(settings: Settings(keymapOverrides: [.reconvert: "ctrl+option+j"]), engine: engine)
        h.selection = ReconvertFixture.selection
        // 既定の割り当ては外れた: 確定済みの idle なので、アプリへ渡す
        #expect(h.press(ReconvertFixture.key, ReconvertFixture.modifiers) == [.passThrough])
        h.press(.character("j"), [.control, .option])
        #expect(h.converting?.origin == .reconversion(original: "品詞", replacing: ReconvertFixture.range))
    }
}

@Suite struct ReconversionKeysTests {
    @Test func escapeWritesTheOriginalBackWithOneRestoreAndCancelsTheConversion() throws {
        let h = try ReconvertFixture.converting()
        let actions = h.press(.escape)
        #expect(actions == [.hideCandidates, .restore("品詞", replacing: TextRange(location: 10, length: 2))])
        #expect(h.engine.cancelledIDs == [ReconvertFixture.conversionID])
        #expect(h.machine.state == .idle)
        // 変換中の状態を作り直す操作（空の setMarkedText など）を挟まない
        #expect(!actions.contains { if case .setMarkedText = $0 { true } else { false } })
    }

    @Test func escapeRestoresTheOriginalEvenAfterTheChoiceChanged() throws {
        let h = try ReconvertFixture.converting()
        h.press(.space)
        #expect(h.press(.escape) == [.hideCandidates, .restore("品詞", replacing: TextRange(location: 10, length: 2))])
    }

    @Test func escapeAfterAScriptConversionStillRestores() throws {
        let h = try ReconvertFixture.converting()
        h.press(.function(7))
        #expect(h.press(.escape) == [.hideCandidates, .restore("品詞", replacing: TextRange(location: 10, length: 3))])
    }

    @Test func enterWithoutAnyChangeCancelsAndLearnsNothing() throws {
        let h = try ReconvertFixture.converting()
        #expect(h.press(.enter) == [.commit("品詞"), .hideCandidates])
        #expect(h.engine.cancelledIDs == [ReconvertFixture.conversionID])
        #expect(h.engine.commitSegmentsCalls.isEmpty && h.engine.plainCommitCalls.isEmpty)
        #expect(h.engine.count(where: { if case .commitText = $0 { true } else { false } }) == 0)
    }

    @Test func movingThroughCandidatesAndBackToTheOriginalIsStillUnchanged() throws {
        let h = try ReconvertFixture.converting()
        h.press(.space)
        h.press(.space, [.shift])
        h.press(.enter)
        #expect(h.engine.commitSegmentsCalls.isEmpty)
        #expect(h.engine.cancelledIDs == [ReconvertFixture.conversionID])
    }

    @Test func aChangedChoiceIsCommittedBySegmentsReplacingTheSelection() throws {
        let h = try ReconvertFixture.converting()
        h.press(.space)
        #expect(h.press(.enter) == [.commit("瀕死"), .hideCandidates])
        #expect(
            h.engine.commitSegmentsCalls == [
                .commitSegments(conversionID: ReconvertFixture.conversionID, layoutRev: 0, choices: [.candidate(1)])
            ])
        #expect(h.engine.cancelledIDs.isEmpty)
        #expect(h.machine.state == .idle)
    }

    @Test func aNumberKeyOnASingleSegmentCommitsTheChoiceBySegments() throws {
        let h = try ReconvertFixture.converting()
        #expect(h.press(.character("2")) == [.commit("瀕死"), .hideCandidates])
        #expect(h.engine.commitSegmentsCalls.count == 1)
    }

    @Test func nextPagesComeFromTheSegmentRequest() throws {
        let h = try ReconvertFixture.converting()
        for _ in 0..<3 { h.press(.space) }
        #expect(h.converting?.candidates == ReconvertFixture.page1)
        #expect(h.engine.segmentPageCalls.count == 1)
        #expect(h.engine.count(where: { if case .page = $0 { true } else { false } }) == 0)
    }

    @Test func aResizeCountsAsAChangeEvenIfTheSurfaceIsTheSame() throws {
        let h = try ReconvertFixture.converting()
        h.engine.onResize = { _, _, _ in
            SegmentLayoutInfo(
                conversionID: ReconvertFixture.conversionID, layoutRev: 1,
                segments: [SegmentSummary(reading: "ひん", surface: "品"), SegmentSummary(reading: "し", surface: "詞")], focused: 0,
                focusedPage: SegmentPage(segment: 0, pageIndex: 0, candidates: [CandidateItem(surface: "品")], hasNext: false))
        }
        h.press(.left, [.shift])
        h.press(.enter)
        #expect(h.engine.commitSegmentsCalls.count == 1)
        #expect(h.engine.cancelledIDs.isEmpty)
    }

    @Test func theImeCommitCompositionCommitsWhatIsShownAndLearnsNothingIfUnchanged() throws {
        let h = try ReconvertFixture.converting()
        #expect(h.machine.commitComposition() == [.commit("品詞"), .hideCandidates])
        #expect(h.engine.cancelledIDs == [ReconvertFixture.conversionID])
        #expect(h.engine.commitSegmentsCalls.isEmpty)
    }

    @Test func typingACharacterCommitsAndStartsAComposition() throws {
        let h = try ReconvertFixture.converting()
        let actions = h.press(.character("k"))
        #expect(actions.first == .commit("品詞"))
        #expect(h.machine.state == .composing(kana: "", pending: "k"))
    }
}

@Suite struct ReconversionBackToReadingTests {
    @Test func backspaceReturnsToTheEstimatedReadingAsAComposition() throws {
        let h = try ReconvertFixture.converting()
        let actions = h.press(.backspace)
        #expect(actions == [.hideCandidates, .setMarkedText(MarkedText(text: "ひんし", highlightedLength: 3))])
        #expect(h.machine.state == .composing(kana: "ひんし", pending: ""))
        #expect(h.engine.cancelledIDs == [ReconvertFixture.conversionID])
        // 直した読みで、通常どおり変換し直せる
        h.type("a")
        #expect(h.machine.state == .composing(kana: "ひんしあ", pending: ""))
    }

    @Test func escapeInThatCompositionWritesTheOriginalBack() throws {
        let h = try ReconvertFixture.converting()
        h.press(.backspace)
        #expect(h.press(.escape) == [.restore("品詞", replacing: TextRange(location: 10, length: 3))])
        #expect(h.machine.state == .idle)
        #expect(h.composer.kana.isEmpty)
        // 終わったあとの Esc は、通常の idle
        #expect(h.press(.escape) == [.resolveStrayMarkedText])
    }

    @Test func enterInThatCompositionCommitsTheKanaAsItIs() throws {
        let h = try ReconvertFixture.converting()
        h.press(.backspace)
        #expect(h.press(.enter) == [.commit("ひんし")])
        #expect(h.engine.calls.last == .commitText(sessionID: Harness.sessionID, reading: "ひんし", surface: "ひんし"))
        // 元の文字列に戻す対象は消える: 次の合成の Esc は通常どおり
        h.type("hi")
        #expect(h.press(.escape) == [.setMarkedText(MarkedText(text: ""))])
    }

    @Test func convertingAgainFromThatCompositionStillRestoresOnTheSecondEscape() throws {
        let h = try ReconvertFixture.converting()
        h.press(.backspace)
        h.engine.onConvert = { _ in MockEngine.defaultPages[0] }
        h.press(.space)
        #expect(h.converting?.origin == .composing)
        #expect(h.engine.convertRequests.first?.reading == "ひんし")
        // 変換中の Esc は読みに戻る（通常どおり）
        #expect(h.press(.escape).contains(.hideCandidates))
        #expect(h.machine.state == .composing(kana: "ひんし", pending: ""))
        // その合成の Esc で、元の文字列に戻る
        #expect(h.press(.escape) == [.restore("品詞", replacing: TextRange(location: 10, length: 3))])
    }

    @Test func kanaInputCannotRebuildTheReadingSoBackspaceDoesNothing() throws {
        // かな入力の合成器は、かなの文字を受け取れない
        final class KeysOnlyComposer: Composing {
            var kana: String { "" }
            var pending: String { "" }
            func pushCharacter(_ scalar: Unicode.Scalar) {}
            func pushJis(key: UInt8, shift: Bool) {}
            func backspace() -> Bool { false }
            func flush() {}
            func clear() {}
        }
        let engine = MockEngine()
        ReconvertFixture.install(on: engine)
        let settings = Settings.default
        let machine = InputStateMachine(
            engine: engine, composer: KeysOnlyComposer(), scripts: FakeScripts(),
            keymap: try Keymap(presetTSV: Harness.presetTSV(.macos), settings: settings), settings: settings,
            session: SessionInfo(sessionID: 1, clientBundleID: "com.apple.TextEdit", secure: false))
        _ = machine.handle(
            KeyInput(key: ReconvertFixture.key, modifiers: ReconvertFixture.modifiers),
            surrounding: SurroundingText(left: "左", available: true), selection: ReconvertFixture.selection)
        guard case .converting = machine.state else {
            Issue.record("did not start")
            return
        }
        let actions = machine.handle(KeyInput(key: .backspace), surrounding: .unavailable)
        #expect(actions == [.consume])
        guard case .converting = machine.state else {
            Issue.record("must stay in the conversion")
            return
        }
        #expect(engine.cancelledIDs.isEmpty)
        // Esc は元に戻せる
        #expect(machine.handle(KeyInput(key: .escape), surrounding: .unavailable) == [.hideCandidates, .restore("品詞", replacing: TextRange(location: 10, length: 2))])
    }
}

@Suite struct StrayMarkedTextTests {
    @Test func enterAndEscapeWhileNothingIsComposedAskTheControllerToResolveAStrayMark() throws {
        let h = try Harness()
        #expect(h.press(.enter) == [.resolveStrayMarkedText])
        #expect(h.press(.escape) == [.resolveStrayMarkedText])
        #expect(h.machine.state == .idle)
    }

    @Test func otherIdleKeysAreNotAffected() throws {
        let h = try Harness()
        #expect(h.press(.space) == [.passThrough])
        #expect(h.press(.backspace) == [.passThrough])
        #expect(h.press(.enter, [.command]) == [.passThrough])
    }

    @Test func enterAndEscapeInsideAnActiveCompositionAreNotStray() throws {
        let h = try Harness()
        h.type("hi")
        #expect(h.press(.enter) == [.commit("ひ")])
        h.type("hi")
        #expect(h.press(.escape) == [.setMarkedText(MarkedText(text: ""))])
    }
}
