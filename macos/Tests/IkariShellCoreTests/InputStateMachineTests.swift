// 状態機械（ADR 0004 §6.3 の表）。入力、Space、ページ送り、数字、Esc / BackSpace、確定、Shift＋英字、z キー。
import Testing

@testable import IkariShellCore

@Suite struct ComposingTests {
    @Test func typingBuildsKanaAndPrefetchesOnlyWhenTheReadingChanges() throws {
        let h = try Harness()
        let actions = h.type("hinsi")
        #expect(h.machine.state == .composing(kana: "ひんし", pending: ""))
        #expect(actions == [.setMarkedText(MarkedText(text: "ひんし", highlightedLength: 3))])
        // 「n」が pending の間は読みが変わらないので、Prefetch は投げない
        #expect(h.engine.prefetchedReadings == ["ひ", "ひん", "ひんし"])
    }

    @Test func pendingRomajiIsShownButNotPartOfTheReading() throws {
        let h = try Harness()
        let actions = h.type("hik")
        #expect(h.machine.state == .composing(kana: "ひ", pending: "k"))
        // 強調はかなの部分だけ
        #expect(actions == [.setMarkedText(MarkedText(text: "ひk", highlightedLength: 1))])
        #expect(h.engine.prefetchedReadings == ["ひ"])
    }

    @Test func zKeyProducesSymbolThroughTheComposer() throws {
        let h = try Harness()
        h.type("zh")
        #expect(h.machine.state == .composing(kana: "←", pending: ""))
    }

    @Test func keysTheComposerIgnoresAreHandedBackToTheApp() throws {
        let h = try Harness()
        #expect(h.press(.character("!")) == [.passThrough])
        #expect(h.machine.state == .idle)
    }

    @Test func kanaInputUsesTheJisKeyNumbers() throws {
        let h = try Harness(settings: Settings(inputMethod: .kana))
        h.press(.jis(22))
        h.press(.jis(32), [.shift])
        #expect(h.machine.state == .composing(kana: "ちの", pending: ""))
        #expect(h.composer.pushedJis.map(\.shift) == [false, true])
        #expect(h.engine.prefetchedReadings == ["ち", "ちの"])
    }

    @Test func backspaceRemovesPendingFirstThenKanaThenReturnsToIdle() throws {
        let h = try Harness()
        h.type("hik")
        h.press(.backspace)
        #expect(h.machine.state == .composing(kana: "ひ", pending: ""))
        h.press(.backspace)
        #expect(h.machine.state == .idle)
        #expect(h.press(.backspace) == [.passThrough])
    }

    @Test func escapeCancelsTheComposition() throws {
        let h = try Harness()
        h.type("hi")
        let actions = h.press(.escape)
        #expect(actions == [.setMarkedText(MarkedText(text: ""))])
        #expect(h.machine.state == .idle)
        #expect(h.composer.kana.isEmpty)
    }

    @Test func enterCommitsTheKanaAsIsAndRecordsHistory() throws {
        let h = try Harness()
        h.type("hin")  // ひ + pending n
        let actions = h.press(.enter)
        #expect(actions == [.commit("ひん")])
        #expect(h.machine.state == .idle)
        #expect(h.engine.calls.last == .commitText(sessionID: Harness.sessionID, reading: "ひん", surface: "ひん"))
    }

    @Test func idleKeysThatAreNotTextGoToTheApp() throws {
        let h = try Harness()
        #expect(h.press(.space) == [.passThrough])
        // Enter と Esc は、取り残された変換中の文字列があれば確定するだけにする。なければコントローラがアプリへ渡す（ADR 0006 §16.3）
        #expect(h.press(.enter) == [.resolveStrayMarkedText])
        #expect(h.press(.escape) == [.resolveStrayMarkedText])
        #expect(h.press(.left) == [.passThrough])
        #expect(h.press(.right, [.shift]) == [.passThrough])
        #expect(h.press(.character("c"), [.command]) == [.passThrough])
        #expect(h.press(.function(5)) == [.passThrough])
        #expect(h.press(.other) == [.passThrough])
    }

    @Test func commandShortcutWhileComposingCommitsFirstThenPassesThrough() throws {
        let h = try Harness()
        h.type("hi")
        let actions = h.press(.character("c"), [.command])
        #expect(actions == [.commit("ひ"), .passThrough])
        #expect(h.machine.state == .idle)
    }
}

@Suite struct ConversionTests {
    @Test func spaceConvertsTheKanaOnlyWithContextAndTheLmBudgetTimeout() throws {
        let h = try Harness()
        h.type("hinsi")
        let actions = h.press(.space)

        let request = try #require(h.engine.convertRequests.first)
        #expect(request.reading == "ひんし")
        #expect(request.sessionID == Harness.sessionID)
        #expect(request.pageSize == 9)
        #expect(request.context == SurroundingText(left: "ユーザー辞書に単語を登録するときは", available: true))
        guard case .convert(_, let timeout) = h.engine.calls.last(where: { if case .convert = $0 { true } else { false } })! else {
            Issue.record("no convert")
            return
        }
        #expect(timeout == EngineTimeouts.convertTimeout(readingCharacterCount: 3))

        let page = MockEngine.defaultPages[0]
        let model = CandidatePanelModel(candidates: page.candidates, selected: 0, annotation: "ひんし(品詞)", reservedRows: Settings.enginePageSize)
        #expect(actions == [.setMarkedText(MarkedText(text: "品詞", highlightedLength: 2)), .showCandidates(model)])
        #expect(h.converting?.reading == "ひんし")
    }

    @Test func pendingConsonantIsExcludedFromTheReading() throws {
        let h = try Harness()
        h.type("hik")
        h.press(.space)
        #expect(h.engine.convertRequests.first?.reading == "ひ")
    }

    @Test func spaceWithOnlyPendingDoesNothing() throws {
        let h = try Harness()
        h.type("k")
        #expect(h.press(.space) == [.consume])
        #expect(h.engine.convertRequests.isEmpty)
    }

    @Test func surroundingTextIsFetchedOnlyInTheEarlyKeystrokesAndBeforeConvertNotOnEveryKey() throws {
        let h = try Harness()
        h.type("hinsi")
        // 最初の contextRefreshKeystrokes 打鍵だけ読む（#1）
        #expect(h.surroundingFetches == InputStateMachine.contextRefreshKeystrokes)
        h.press(.space)
        #expect(h.surroundingFetches == InputStateMachine.contextRefreshKeystrokes + 1)
    }

    @Test func prefetchAndConvertShareTheContextRevisionWhenTheTextDidNotChange() throws {
        let h = try Harness()
        h.type("hi")
        h.press(.space)
        guard case .prefetch(let prefetch) = h.engine.calls.first(where: { if case .prefetch = $0 { true } else { false } })! else {
            Issue.record("no prefetch")
            return
        }
        #expect(prefetch.contextRev == h.engine.convertRequests[0].contextRev)
        #expect(prefetch.context == h.engine.convertRequests[0].context)
    }

    @Test func contextRevisionAdvancesWhenTheSurroundingTextChangedBeforeConvert() throws {
        let h = try Harness()
        h.type("hi")
        h.surroundingText = SurroundingText(left: "別の文", available: true)
        h.press(.space)
        guard case .prefetch(let prefetch) = h.engine.calls.first(where: { if case .prefetch = $0 { true } else { false } })! else {
            Issue.record("no prefetch")
            return
        }
        #expect(h.engine.convertRequests[0].contextRev != prefetch.contextRev)
        #expect(h.engine.convertRequests[0].context.left == "別の文")
    }

    @Test func surroundingTextIsCutToTheEngineLimit() throws {
        let h = try Harness()
        h.surroundingText = SurroundingText(
            left: String(repeating: "あ", count: 2500) + "末尾", right: String(repeating: "い", count: 2500), available: true)
        h.type("hi")
        h.press(.space)
        let context = try #require(h.engine.convertRequests.first?.context)
        #expect(context.left.count == 2000)
        #expect(context.left.hasSuffix("末尾"))
        #expect(context.right.count == 2000)
    }

    @Test func unavailableSurroundingTextIsPassedAsUnavailable() throws {
        let h = try Harness()
        h.surroundingText = .unavailable
        h.type("hi")
        h.press(.space)
        #expect(h.engine.convertRequests.first?.context.available == false)
    }

    @Test func theEngineAlwaysGetsTheFullPageSizeAndTheSettingIsHowManyAreShownFirst() throws {
        let h = try Harness(settings: Settings(pageSize: 2))
        h.type("hi")
        let actions = h.press(.space)
        #expect(h.engine.convertRequests.first?.pageSize == Settings.enginePageSize)
        #expect(h.engine.count(where: { if case .prefetch(let r) = $0 { r.pageSize == Settings.enginePageSize } else { false } }) == 1)
        #expect(h.converting?.candidates.count == 3)  // エンジンの返事は 3 件
        guard case .showCandidates(let model)? = actions.last else { Issue.record("no panel"); return }
        #expect(model.candidates.count == 2)
    }

    @Test func spaceMovesThroughTheCandidatesOfThePage() throws {
        let h = try Harness()
        h.type("hinsi")
        h.press(.space)
        let actions = h.press(.space)
        #expect(h.converting?.selected == 1)
        let page = MockEngine.defaultPages[0]
        let model = CandidatePanelModel(candidates: page.candidates, selected: 1, annotation: "ひんし(瀕死)", reservedRows: Settings.enginePageSize)
        #expect(actions == [.setMarkedText(MarkedText(text: "瀕死", highlightedLength: 2)), .moveSelection(model)])
    }

    @Test func spaceOnTheLastCandidateOfAPageFetchesTheNextPageAndSelectsItsFirst() throws {
        let h = try Harness()
        h.type("hinsi")
        h.press(.space)
        h.press(.space)
        h.press(.space)
        let actions = h.press(.space)  // ページの最後 → 次のページの先頭
        let state = try #require(h.converting)
        #expect(state.pageIndex == 1)
        #expect(state.selected == 0)
        #expect(state.candidateOffset == 3)
        #expect(state.candidates.first?.surface == "ヒンシ")
        #expect(
            h.engine.calls.last == .page(conversionID: 7, index: 1, EngineTimeouts.convertTimeout(readingCharacterCount: 3)))
        guard case .showCandidates = actions[1] else {
            Issue.record("a new page replaces the panel content")
            return
        }
    }

    /// 3 ページ目の先頭の候補まで進める
    private func advanceToLastPage(_ h: Harness) {
        h.type("hinsi")
        h.press(.space)  // page 0, selected 0
        for _ in 0..<5 { h.press(.space) }  // 0,1,2 → page 1 の 0,1,2
        h.press(.space)  // page 2 の先頭
    }

    @Test func spaceOnTheLastPageWrapsToTheFirstPageByDefault() throws {
        let h = try Harness()
        advanceToLastPage(h)
        #expect(h.converting?.pageIndex == 2)
        h.press(.space)  // 牝糸 → ﾋﾝｼ
        let pageCallsBefore = h.engine.count(where: { if case .page = $0 { true } else { false } })
        h.press(.space)  // 最後 → 先頭のページ
        let state = try #require(h.converting)
        #expect(state.pageIndex == 0)
        #expect(state.selected == 0)
        #expect(state.candidateOffset == 0)
        // 生成済みのページはエンジンへ問い合わせ直さない
        #expect(h.engine.count(where: { if case .page = $0 { true } else { false } }) == pageCallsBefore)
    }

    @Test func spaceOnTheLastPageStopsWhenWrapIsOff() throws {
        let h = try Harness(settings: Settings(pagingWrap: false))
        advanceToLastPage(h)
        h.press(.space)
        let before = h.converting
        #expect(h.press(.space) == [.consume])
        #expect(h.converting == before)
        #expect(h.converting?.pageIndex == 2)
        #expect(h.converting?.selected == 1)
    }

    @Test func shiftSpaceGoesBackAndCrossesToTheLastCandidateOfThePreviousPage() throws {
        let h = try Harness()
        advanceToLastPage(h)  // page 2 の先頭
        h.press(.space, [.shift])
        let state = try #require(h.converting)
        #expect(state.pageIndex == 1)
        #expect(state.selected == 2)
        #expect(state.candidateOffset == 3)
        h.press(.space, [.shift])
        #expect(h.converting?.selected == 1)
    }

    @Test func shiftSpaceOnTheVeryFirstCandidateStays() throws {
        let h = try Harness()
        h.type("hi")
        h.press(.space)
        #expect(h.press(.space, [.shift]) == [.consume])
        #expect(h.converting?.selected == 0)
        #expect(h.converting?.pageIndex == 0)
    }

    @Test func failedPageFetchKeepsTheCurrentSelection() throws {
        let h = try Harness()
        h.engine.onPage = { _, _ in throw EngineClientError.timeout }
        h.type("hi")
        h.press(.space)
        h.press(.space)
        h.press(.space)
        #expect(h.press(.space) == [.consume])
        #expect(h.converting?.pageIndex == 0)
        #expect(h.converting?.selected == 2)
    }

    @Test func numberKeySelectsAndCommitsThatCandidate() throws {
        let h = try Harness()
        h.type("hinsi")
        h.press(.space)
        let actions = h.press(.character("2"))
        #expect(actions == [.commit("瀕死"), .hideCandidates])
        #expect(h.engine.calls.last == .commit(conversionID: 7, candidateIndex: 1))
        #expect(h.machine.state == .idle)
    }

    @Test func numberKeyOnLaterPageUsesTheCumulativeIndex() throws {
        let h = try Harness()
        h.type("hinsi")
        h.press(.space)
        for _ in 0..<3 { h.press(.space) }  // page 1 の先頭
        let actions = h.press(.character("3"))
        #expect(actions == [.commit("品指"), .hideCandidates])
        #expect(h.engine.calls.last == .commit(conversionID: 7, candidateIndex: 3 + 2))
    }

    @Test func numberBeyondTheCandidatesIsIgnored() throws {
        let h = try Harness()
        h.type("hi")
        h.press(.space)
        #expect(h.press(.character("9")) == [.consume])
        #expect(h.converting != nil)
    }

    @Test func jisDigitKeysAlsoSelectInKanaMode() throws {
        let h = try Harness(settings: Settings(inputMethod: .kana))
        h.press(.jis(22))
        h.press(.space)
        let actions = h.press(.jis(1))  // IKARI_JIS_DIGIT2
        #expect(actions == [.commit("瀕死"), .hideCandidates])
    }

    @Test func enterCommitsTheSelectedCandidate() throws {
        let h = try Harness()
        h.type("hinsi")
        h.press(.space)
        h.press(.space)
        let actions = h.press(.enter)
        #expect(actions == [.commit("瀕死"), .hideCandidates])
        #expect(h.engine.calls.last == .commit(conversionID: 7, candidateIndex: 1))
        #expect(h.composer.kana.isEmpty)
    }

    @Test func escapeAndBackspaceGoBackToTheReadingWithoutDeleting() throws {
        for key in [KeyInput.Key.escape, .backspace] {
            let h = try Harness()
            h.type("hinsi")
            h.press(.space)
            let actions = h.press(key)
            #expect(h.machine.state == .composing(kana: "ひんし", pending: ""))
            #expect(actions == [.hideCandidates, .setMarkedText(MarkedText(text: "ひんし", highlightedLength: 3))])
            #expect(h.engine.calls.contains(.cancel(7)))
            // 読みに戻ったあと、もう一度 Space で変換できる
            h.press(.space)
            #expect(h.converting?.reading == "ひんし")
        }
    }

    @Test func typingACharacterWhileConvertingCommitsAndStartsANewComposition() throws {
        let h = try Harness()
        h.type("hinsi")
        h.press(.space)
        let actions = h.press(.character("k"))
        #expect(
            actions == [
                .commit("品詞"), .hideCandidates, .setMarkedText(MarkedText(text: "k", highlightedLength: 0)),
            ])
        #expect(h.machine.state == .composing(kana: "", pending: "k"))
        #expect(h.engine.calls.contains(.commit(conversionID: 7, candidateIndex: 0)))
    }

    @Test func commandShortcutWhileConvertingCommitsFirstThenPassesThrough() throws {
        let h = try Harness()
        h.type("hi")
        h.press(.space)
        #expect(h.press(.character("v"), [.command]) == [.commit("品詞"), .hideCandidates, .passThrough])
    }

    @Test func nonScriptFunctionKeysAreSwallowedWhileComposing() throws {
        let h = try Harness()
        h.type("hi")
        #expect(h.press(.function(1)) == [.consume])
        #expect(h.machine.state == .composing(kana: "ひ", pending: ""))
    }
}

@Suite struct ScriptConversionKeyTests {
    @Test(arguments: [
        (6, "ひんし"), (7, "ヒンシ"), (8, "ﾋﾝｼ"),
    ])
    func functionKeysConvertTheReadingAndEnterASingleCandidateConversion(number: Int, expected: String) throws {
        let h = try Harness()
        h.type("hinsi")
        let actions = h.press(.function(number))
        #expect(actions == [.hideCandidates, .setMarkedText(MarkedText(text: expected, highlightedLength: expected.utf16.count))])
        let state = try #require(h.converting)
        #expect(state.candidates.map(\.surface) == [expected])
        #expect(state.isFallback)
        #expect(h.engine.convertRequests.isEmpty)
    }

    @Test func f9AndF10UseTheAlnumConversions() throws {
        let h = try Harness()
        h.type("hi")
        h.press(.function(9))
        #expect(h.converting?.candidates.count == 1)
        h.press(.function(10))
        #expect(h.converting?.candidates.count == 1)
        #expect(h.converting?.reading == "ひ")
    }

    @Test(arguments: [
        ("j", "ひんし"), ("k", "ヒンシ"), (";", "ﾋﾝｼ"),
    ])
    func macosPresetBindsControlKeys(key: String, expected: String) throws {
        let h = try Harness(preset: .macos)
        h.type("hinsi")
        h.press(.character(key), [.control])
        #expect(h.converting?.candidates.first?.surface == expected)
    }

    @Test(arguments: [
        ("u", "ひんし"), ("i", "ヒンシ"), ("o", "ﾋﾝｼ"),
    ])
    func msImePresetBindsItsControlKeys(key: String, expected: String) throws {
        let h = try Harness(preset: .msIme)
        h.type("hinsi")
        h.press(.character(key), [.control])
        #expect(h.converting?.candidates.first?.surface == expected)
    }

    private func atokConverts(key: String, modifiers: KeyModifiers) throws {
        let h = try Harness(preset: .atok)
        h.type("hinsi")
        h.press(.character(key), modifiers)
        // 半角英数もひらがなも、変換中になる（替え玉の字種変換は読みのまま返す）
        #expect(h.converting?.candidates.first?.surface == "ひんし")
    }

    @Test func atokPresetBindsCtrlAtForJisAndCtrlShiftAtForUs() throws {
        try atokConverts(key: "@", modifiers: [.control])
        try atokConverts(key: "@", modifiers: [.control, .shift])
    }

    @Test func atokPresetBindsCtrlU() throws {
        try atokConverts(key: "u", modifiers: [.control])
    }

    @Test func presetKeysOfTheOtherPresetAreNotBound() throws {
        let h = try Harness(preset: .macos)
        h.type("hinsi")
        // ctrl+i は MS-IME 風のキー。macOS 風では割り当てがないので、確定してからアプリへ渡す
        #expect(h.press(.character("i"), [.control]) == [.commit("ひんし"), .passThrough])
    }

    @Test func overridingAKeyReplacesThePresetCharacterKeyButKeepsFunctionKeys() throws {
        let settings = Settings(keymapOverrides: [.toFullwidthKatakana: "ctrl+g"])
        let h = try Harness(settings: settings)
        h.type("hinsi")
        h.press(.character("k"), [.control])  // 外れた
        #expect(h.converting == nil)
    }

    @Test func overriddenKeyWorksAndFunctionKeyStillWorks() throws {
        let settings = Settings(keymapOverrides: [.toFullwidthKatakana: "ctrl+g"])
        let h = try Harness(settings: settings)
        h.type("hinsi")
        h.press(.character("g"), [.control])
        #expect(h.converting?.candidates.first?.surface == "ヒンシ")
        h.press(.escape)
        h.press(.function(7))
        #expect(h.converting?.candidates.first?.surface == "ヒンシ")
    }

    @Test func scriptKeysDoNothingWhenIdle() throws {
        let h = try Harness()
        #expect(h.press(.function(7)) == [.passThrough])
    }

    @Test func scriptKeyWhileConvertingReplacesTheConversionAndCancelsTheEngineOne() throws {
        let h = try Harness()
        h.type("hinsi")
        h.press(.space)
        h.press(.function(7))
        #expect(h.engine.calls.contains(.cancel(7)))
        #expect(h.converting?.candidates.map(\.surface) == ["ヒンシ"])
        // 字種変換の結果を確定しても、履歴は CommitText で送る（Commit ではない）
        h.press(.enter)
        #expect(h.engine.calls.last == .commitText(sessionID: Harness.sessionID, reading: "ひんし", surface: "ヒンシ"))
    }

    @Test func escapeFromAScriptConversionReturnsToTheReading() throws {
        let h = try Harness()
        h.type("hinsi")
        h.press(.function(7))
        h.press(.escape)
        #expect(h.machine.state == .composing(kana: "ひんし", pending: ""))
    }
}

@Suite struct AsciiRunTests {
    @Test func shiftLettersStartATemporaryAlphanumericRun() throws {
        let h = try Harness()
        h.press(.character("A"), [.shift])
        h.press(.character("P"), [.shift])
        let actions = h.press(.character("I"), [.shift])
        #expect(h.machine.state == .asciiRun(text: "API"))
        #expect(actions == [.setMarkedText(MarkedText(text: "API", highlightedLength: 3))])
        // 英字は ASCII のまま。ローマ字の合成器には渡さない
        #expect(h.composer.kana.isEmpty)
        #expect(h.press(.enter) == [.commit("API")])
        #expect(h.machine.state == .idle)
    }

    @Test func followingLowercaseLettersStayAscii() throws {
        let h = try Harness()
        h.press(.character("K"), [.shift])
        h.type("ubernetes")
        #expect(h.machine.state == .asciiRun(text: "Kubernetes"))
    }

    @Test func shiftLetterAfterKanaKeepsTheKanaAsAPrefix() throws {
        let h = try Harness()
        h.type("hin")
        h.press(.character("A"), [.shift])
        #expect(h.machine.state == .asciiRun(text: "ひんA"))  // pending の n は ん として確定する
    }

    @Test func escapeLeavesTheRunBackToTheKanaOrToIdle() throws {
        let h = try Harness()
        h.type("hi")
        h.press(.character("A"), [.shift])
        h.press(.escape)
        #expect(h.machine.state == .composing(kana: "ひ", pending: ""))

        let h2 = try Harness()
        h2.press(.character("A"), [.shift])
        h2.press(.escape)
        #expect(h2.machine.state == .idle)
    }

    @Test func backspaceRemovesOneAsciiCharacterAndLeavesWhenNoneIsLeft() throws {
        let h = try Harness()
        h.press(.character("A"), [.shift])
        h.type("pi")
        h.press(.backspace)
        #expect(h.machine.state == .asciiRun(text: "Ap"))
        h.press(.backspace)
        h.press(.backspace)
        #expect(h.machine.state == .idle)
    }

    @Test func spaceConvertsTheRunAsTheReadingAndEscapeReturnsToTheRun() throws {
        let h = try Harness()
        h.press(.character("A"), [.shift])
        h.press(.character("P"), [.shift])
        h.press(.space)
        #expect(h.engine.convertRequests.first?.reading == "AP")
        h.press(.escape)
        #expect(h.machine.state == .asciiRun(text: "AP"))
    }

    @Test func shiftLetterWhileConvertingCommitsThenStartsARun() throws {
        let h = try Harness()
        h.type("hi")
        h.press(.space)
        let actions = h.press(.character("A"), [.shift])
        #expect(actions.first == .commit("品詞"))
        #expect(h.machine.state == .asciiRun(text: "A"))
    }

    @Test func shiftedNonLettersAreNotARun() throws {
        let h = try Harness()
        // Shift+1 → "!"。合成器が受け取らないのでアプリに渡る
        #expect(h.press(.character("!"), [.shift]) == [.passThrough])
        #expect(h.machine.state == .idle)
    }
}

@Suite struct FallbackTests {
    @Test(arguments: [
        EngineClientError.unavailable, .timeout, .invalidReply, .engine(.engineNotReady),
    ])
    func engineFailureYieldsShellOnlyCandidatesWithoutBlockingInput(error: EngineClientError) throws {
        let h = try Harness()
        h.engine.onConvert = { _ in throw error }
        h.type("hinsi")
        let actions = h.press(.space)
        let state = try #require(h.converting)
        #expect(state.candidates.map(\.surface) == ["ひんし", "ヒンシ", "ﾋﾝｼ"])
        #expect(state.isFallback)
        #expect(state.conversionID == 0)
        #expect(actions.contains { if case .showCandidates = $0 { true } else { false } })
    }

    @Test func emptyEngineReplyIsTreatedAsFailure() throws {
        let h = try Harness()
        h.engine.onConvert = { _ in ConversionPage(conversionID: 1, pageIndex: 0, candidates: [], hasNext: false) }
        h.type("hi")
        h.press(.space)
        #expect(h.converting?.isFallback == true)
    }

    @Test func fallbackCommitGoesThroughCommitTextAndNeverTouchesTheEngineConversion() throws {
        let h = try Harness()
        h.engine.onConvert = { _ in throw EngineClientError.timeout }
        h.type("hinsi")
        h.press(.space)
        h.press(.space)  // ヒンシ
        h.press(.enter)
        #expect(h.engine.calls.last == .commitText(sessionID: Harness.sessionID, reading: "ひんし", surface: "ヒンシ"))
        #expect(h.engine.count(where: { if case .commit = $0 { true } else { false } }) == 0)
        // フォールバックの変換はエンジンに Cancel を送らない
        h.type("hinsi")
        h.press(.space)
        h.press(.escape)
        #expect(h.engine.count(where: { if case .cancel = $0 { true } else { false } }) == 0)
    }

    @Test func fallbackWrapsWithinItsSinglePage() throws {
        let h = try Harness()
        h.engine.onConvert = { _ in throw EngineClientError.unavailable }
        h.type("hinsi")
        h.press(.space)
        h.press(.space)
        h.press(.space)
        h.press(.space)  // 最後 → 先頭
        #expect(h.converting?.selected == 0)
    }

    @Test func readingLongerThanTheEngineLimitSkipsTheEngine() throws {
        let h = try Harness()
        h.type(String(repeating: "a", count: InputStateMachine.maxReadingCharacters + 1))
        h.press(.space)
        #expect(h.engine.convertRequests.isEmpty)
        #expect(h.converting?.isFallback == true)
    }

    @Test func pageSizedReplyFromTheEngineIsShownAsIs() throws {
        let h = try Harness()
        h.type("hi")
        h.press(.space)
        #expect(h.converting?.isFallback == false)
        #expect(h.converting?.conversionID == 7)
    }
}

@Suite struct SecureAndHistoryTests {
    // ADR 0026 D5: secure の間は日本語のモードでも機能として OFF（打鍵は素通し。変換も文脈も学習もない）。
    // 以前は「secure でも変換する」だった（文脈と学習だけ止める）ので、このスイートの期待を D5 に合わせて直した
    @Test func secureSessionPassesEveryKeyThroughAndNeverTouchesTheEngine() throws {
        let h = try Harness(secure: true)
        for character in "hinsi" { #expect(h.press(.character(String(character))) == [.passThrough]) }
        #expect(h.press(.space) == [.passThrough])
        #expect(h.press(.enter) == [.passThrough])
        #expect(h.surroundingFetches == 0)
        #expect(h.engine.prefetchedReadings.isEmpty)
        #expect(h.engine.convertRequests.isEmpty)
        #expect(h.engine.count(where: { if case .commit = $0 { true } else { false } }) == 0)
        #expect(h.engine.count(where: { if case .commitText = $0 { true } else { false } }) == 0)
        #expect(h.machine.state == .idle)
    }

    @Test func secureFlagCanTurnOnMidSessionAndTellsTheEngineOnlyOnChange() throws {
        let h = try Harness()
        h.machine.activate()
        h.machine.setSecure(false)  // 変化なし
        h.machine.setSecure(true)
        h.machine.setSecure(true)
        let begins = h.engine.calls.compactMap { call -> SessionInfo? in
            if case .beginSession(let s) = call { s } else { nil }
        }
        #expect(begins.map(\.secure) == [false, true])
        #expect(begins.allSatisfy { $0.sessionID == Harness.sessionID && $0.clientBundleID == "com.apple.TextEdit" })

        #expect(h.press(.character("h")) == [.passThrough])
        #expect(h.surroundingFetches == 0)
        #expect(h.machine.state == .idle)
    }

    @Test func leavingSecureRestoresTheNormalBehavior() throws {
        let h = try Harness(secure: true)
        #expect(h.press(.character("h")) == [.passThrough])
        h.machine.setSecure(false)
        h.type("hi")
        #expect(h.markedText == "ひ")
        h.press(.space)
        #expect(h.engine.convertRequests.count == 1)
        #expect(h.surroundingFetches > 0)
    }

    @Test func becomingSecureWhileComposingCommitsTheDisplayAsIsWithoutSendingAndPassesTheKey() throws {
        let h = try Harness()
        h.type("hi")
        h.machine.setSecure(true)
        let before = h.engine.calls.count
        #expect(h.press(.character("x")) == [.commit("ひ"), .passThrough])
        #expect(h.machine.state == .idle)
        // 確定履歴にも学習にも送らない
        #expect(h.engine.calls.count == before)
    }

    @Test func becomingSecureWhileConvertingCommitsTheSelectedCandidateWithoutSending() throws {
        let h = try Harness()
        h.type("hinsi")
        h.press(.space)
        h.machine.setSecure(true)
        let before = h.engine.calls.count
        #expect(h.press(.enter) == [.commit("品詞"), .hideCandidates, .passThrough])
        #expect(h.machine.state == .idle)
        #expect(h.engine.calls.count == before)
    }

    @Test func committedTextIsReportedToTheEngineSoItBecomesTheLeftContext() throws {
        let h = try Harness()
        h.surroundingText = .unavailable  // ターミナル。履歴がないと文脈が無くなる場面
        h.machine.activate()
        h.type("hinsi")
        h.press(.space)
        h.press(.enter)  // Commit
        h.type("hi")
        h.press(.enter)  // CommitText
        let relevant = h.engine.calls.filter {
            switch $0 {
            case .commit, .commitText: true
            default: false
            }
        }
        #expect(
            relevant == [
                .commit(conversionID: 7, candidateIndex: 0),
                .commitText(sessionID: Harness.sessionID, reading: "ひ", surface: "ひ"),
            ])
        // 文脈が取れないことは available=false で伝え、履歴の合成はエンジンに任せる
        #expect(h.engine.convertRequests.first?.context.available == false)
    }

    @Test func nextCompositionFetchesTheSurroundingTextAgainAfterACommit() throws {
        let h = try Harness()
        h.type("hi")
        h.press(.enter)
        h.surroundingText = SurroundingText(left: "ひ", available: true)
        h.type("a")
        guard case .prefetch(let request) = h.engine.calls.last! else {
            Issue.record("expected prefetch")
            return
        }
        #expect(request.context.left == "ひ")
    }

    @Test func sessionStartAndEndAreReported() throws {
        let h = try Harness()
        h.machine.activate()
        h.machine.deactivate()
        #expect(
            h.engine.calls == [
                .beginSession(SessionInfo(sessionID: Harness.sessionID, clientBundleID: "com.apple.TextEdit", secure: false)),
                .endSession(Harness.sessionID),
            ])
    }

    @Test func activateIfNeededBeginsTheSessionOnlyWhenItIsNotBegun() throws {
        // #315: activateServer を経ずに打鍵が来る（殻のプロセスが起動し直した直後など）と、
        // エンジンはセッションを知らず、UNKNOWN_SESSION が続く。打鍵の前に BeginSession を送る
        let h = try Harness()
        let begin = MockEngine.Call.beginSession(
            SessionInfo(sessionID: Harness.sessionID, clientBundleID: "com.apple.TextEdit", secure: false))
        h.machine.activateIfNeeded()  // 一度も始めていない
        h.machine.activateIfNeeded()  // 始めたあとは送らない
        #expect(h.engine.calls == [begin])

        h.machine.deactivate()
        h.machine.activateIfNeeded()  // 終えたあと（deactivateServer のあとに打鍵が来た）
        #expect(h.engine.calls == [begin, .endSession(Harness.sessionID), begin])

        h.machine.activate()
        h.machine.activateIfNeeded()
        #expect(h.engine.calls.count == 4)
    }

    @Test func aSecureChangeAlsoBeginsTheSessionSoActivateIfNeededDoesNotRepeatIt() throws {
        let h = try Harness()
        h.machine.setSecure(true)
        h.machine.activateIfNeeded()
        #expect(h.engine.count(where: { if case .beginSession = $0 { true } else { false } }) == 1)
    }

    @Test func commitCompositionCommitsWhatIsShown() throws {
        let h = try Harness()
        #expect(h.machine.commitComposition().isEmpty)
        h.type("hi")
        #expect(h.machine.commitComposition() == [.commit("ひ")])
        #expect(h.machine.state == .idle)
    }
}
