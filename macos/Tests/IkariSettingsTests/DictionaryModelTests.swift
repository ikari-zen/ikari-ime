import Foundation
import IkariProto
import Testing

@testable import IkariSettings

private func options() -> [PosOption] {
    FakeState.posOptions.map { option in
        var out = PosOption()
        out.name = option.name
        out.needsConjType = option.needsConjType
        return out
    }
}

@MainActor
private func loadedModel(_ harness: Harness) async -> DictionaryModel {
    let model = DictionaryModel(bridge: harness.client)
    await model.load()
    return model
}

@Suite struct DictionaryRulesTests {
    @Test func buildsAnEntryWithTrimmedFields() throws {
        let draft = DictionaryDraft(reading: " いかり ", surface: "錨 ", posName: "名詞", conjType: "", comment: " メモ")
        let entry = try DictionaryRules.entry(from: draft, options: options()).get()
        #expect(entry.reading == "いかり")
        #expect(entry.surface == "錨")
        #expect(entry.pos == "名詞")
        #expect(entry.comment == "メモ")
    }

    @Test func aVerbCarriesItsConjugationTypeInThePosField() throws {
        let draft = DictionaryDraft(reading: "いかる", surface: "怒る", posName: "動詞", conjType: "五段-ラ行")
        #expect(try DictionaryRules.entry(from: draft, options: options()).get().pos == "動詞:五段-ラ行")
    }

    @Test func aNonVerbIgnoresTheConjugationType() throws {
        let draft = DictionaryDraft(reading: "いかり", surface: "錨", posName: "名詞", conjType: "五段-ラ行")
        #expect(try DictionaryRules.entry(from: draft, options: options()).get().pos == "名詞")
    }

    @Test func reportsTheFirstMissingPart() {
        func problem(_ draft: DictionaryDraft) -> DraftProblem? {
            if case .failure(let problem) = DictionaryRules.entry(from: draft, options: options()) { return problem }
            return nil
        }
        #expect(problem(DictionaryDraft(reading: "", surface: "あ", posName: "名詞")) == .readingEmpty)
        #expect(problem(DictionaryDraft(reading: "あ", surface: "  ", posName: "名詞")) == .surfaceEmpty)
        #expect(problem(DictionaryDraft(reading: "あ", surface: "あ", posName: "")) == .posNotChosen)
        #expect(problem(DictionaryDraft(reading: "あ", surface: "あ", posName: "未知")) == .posNotChosen)
        #expect(problem(DictionaryDraft(reading: "あ", surface: "あ", posName: "動詞")) == .conjTypeNeeded)
        #expect(problem(DictionaryDraft(reading: "あ", surface: "あ\tい", posName: "名詞")) == .controlCharacters)
        #expect(problem(DictionaryDraft(reading: "あ", surface: "あ", posName: "名詞", comment: "a\nb")) == .controlCharacters)
    }

    @Test func showsTheConjugationTypeInParentheses() {
        #expect(DictionaryRules.posDisplay("名詞") == "名詞")
        #expect(DictionaryRules.posDisplay("動詞:五段-ラ行") == "動詞（五段-ラ行）")
    }

    @Test func searchesAllTermsAcrossFieldsIgnoringKanaCaseAndWidth() {
        let entries = [
            userEntry(reading: "いかり", surface: "怒り", comment: "ikari"),
            userEntry(reading: "いかり", surface: "錨", pos: "固有名詞"),
            userEntry(reading: "さんぷる", surface: "SAMPLE"),
        ]
        #expect(DictionaryRules.filter(entries, query: "").count == 3)
        #expect(DictionaryRules.filter(entries, query: "錨").map(\.surface) == ["錨"])
        #expect(DictionaryRules.filter(entries, query: "イカリ").count == 2)  // カタカナでも引ける
        #expect(DictionaryRules.filter(entries, query: "IKARI").map(\.surface) == ["怒り"])  // コメント、大文字小文字
        #expect(DictionaryRules.filter(entries, query: "いかり 固有").map(\.surface) == ["錨"])  // 品詞も対象、AND
        #expect(DictionaryRules.filter(entries, query: "ｓａｍｐｌｅ").map(\.surface) == ["SAMPLE"])  // 全角
        #expect(DictionaryRules.filter(entries, query: "なし").isEmpty)
    }

    @Test func sortsByReadingThenSurface() {
        let sorted = DictionaryRules.sorted([
            userEntry(reading: "いか", surface: "烏賊"), userEntry(reading: "あ", surface: "亜"),
            userEntry(reading: "いか", surface: "以下"),
        ])
        #expect(sorted.map(\.reading) == ["あ", "いか", "いか"])
        #expect(sorted[1].surface < sorted[2].surface)
    }
}

@Suite @MainActor struct DictionaryModelTests {
    @Test func loadsEntriesAndTheChoices() async throws {
        var state = FakeState()
        state.userDict = [userEntry(reading: "いかり", surface: "錨")]
        let harness = try await Harness.make(state: state)
        defer { harness.close() }
        let model = await loadedModel(harness)
        #expect(model.isLoaded)
        #expect(model.entries.count == 1)
        #expect(model.posOptions.count == FakeState.posOptions.count)
        #expect(model.draft.posName == "名詞")  // 先頭を既定にする
        #expect(!model.needsConjType)
    }

    @Test func addsAnEntryAndClearsTheFieldsButKeepsThePos() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        let model = await loadedModel(harness)
        model.selectPos("人名")
        model.draft = DictionaryDraft(reading: "いかり", surface: "碇", posName: "人名", comment: "名字")
        #expect(model.canAdd)
        #expect(await model.add())
        #expect(model.entries.map(\.surface) == ["碇"])
        #expect(model.entries.first?.pos == "人名")
        #expect(model.draft.reading.isEmpty && model.draft.surface.isEmpty && model.draft.comment.isEmpty)
        #expect(model.draft.posName == "人名")
        #expect(model.confirmation == "「碇」を登録しました")
        #expect(model.message == nil)
        #expect(harness.shell.state.snapshot.userDict.count == 1)
    }

    @Test func addingAVerbSendsTheConjugationType() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        let model = await loadedModel(harness)
        model.selectPos("動詞")
        #expect(model.needsConjType)
        #expect(model.draft.conjType == "五段-ラ行")  // 既定は先頭
        model.draft.reading = "いかる"
        model.draft.surface = "怒る"
        #expect(await model.add())
        #expect(harness.shell.state.snapshot.userDict.first?.pos == "動詞:五段-ラ行")
        model.selectPos("名詞")
        #expect(model.draft.conjType.isEmpty)
    }

    @Test func theEngineRefusalIsShownAndNothingIsAdded() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        let model = await loadedModel(harness)
        model.draft.reading = "ikari"  // ひらがなではない
        model.draft.surface = "錨"
        #expect(!(await model.add()))
        #expect(model.message == "読みはひらがな（とー）にしてください")
        #expect(model.entries.isEmpty)
        #expect(model.draft.reading == "ikari")  // 直せるように残す
        #expect(model.confirmation == nil)
    }

    @Test func anIncompleteDraftIsRefusedWithoutAskingTheEngine() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        let model = await loadedModel(harness)
        let before = harness.shell.state.snapshot.requestLog.count
        model.draft.reading = "いかり"
        #expect(!model.canAdd)
        #expect(!(await model.add()))
        #expect(model.message == DraftProblem.surfaceEmpty.message)
        #expect(harness.shell.state.snapshot.requestLog.count == before)
    }

    @Test func removesAnEntry() async throws {
        var state = FakeState()
        state.userDict = [userEntry(reading: "いかり", surface: "錨"), userEntry(reading: "いかり", surface: "怒り")]
        let harness = try await Harness.make(state: state)
        defer { harness.close() }
        let model = await loadedModel(harness)
        let target = try #require(model.entries.first { $0.surface == "錨" })
        await model.remove(target)
        #expect(model.entries.map(\.surface) == ["怒り"])
        #expect(model.message == nil)
    }

    @Test func removingAMissingEntryShowsTheReason() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        let model = await loadedModel(harness)
        await model.remove(userEntry(reading: "ない", surface: "無い"))
        #expect(model.message == "該当する項目がありません")
    }

    @Test func searchNarrowsTheVisibleList() async throws {
        var state = FakeState()
        state.userDict = [userEntry(reading: "いかり", surface: "錨"), userEntry(reading: "さかな", surface: "魚")]
        let harness = try await Harness.make(state: state)
        defer { harness.close() }
        let model = await loadedModel(harness)
        model.query = "魚"
        #expect(model.visibleEntries.map(\.surface) == ["魚"])
        model.query = ""
        #expect(model.visibleEntries.count == 2)
    }

    @Test func aTransportFailureIsShownAsAMessage() async throws {
        let harness = try await Harness.make()
        let model = await loadedModel(harness)
        harness.shell.stop()
        model.draft.reading = "いかり"
        model.draft.surface = "錨"
        #expect(!(await model.add()))
        #expect(model.message?.contains("接続が切れました") == true)
    }
}
