// 確信が高い変換では、最初の Space で 1 位だけを出し、一覧は次の Space / ↓ で出す（`candidate.hide_when_confident`。#69）
import Testing

@testable import IkariShellCore

@Suite struct ConfidentConversionTests {
    static let onSettings = Settings(candidateHideWhenConfident: true)
    static let offSettings = Settings(candidateHideWhenConfident: false)

    static func page(confident: Bool, layout: SegmentLayoutInfo? = nil) -> ConversionPage {
        var page = MockEngine.defaultPages[0]
        page.confident = confident
        page.layout = layout
        return page
    }

    static func harness(
        settings: Settings = onSettings, secure: Bool = false, confident: Bool = true, layout: SegmentLayoutInfo? = nil
    ) throws -> Harness {
        let h = try Harness(settings: settings, secure: secure)
        let reply = page(confident: confident, layout: layout)
        h.engine.onConvert = { _ in reply }
        return h
    }

    static func model(selected: Int = 0, annotation: String = "ひんし(品詞)") -> CandidatePanelModel {
        CandidatePanelModel(
            candidates: MockEngine.defaultPages[0].candidates, selected: selected, annotation: annotation,
            reservedRows: Settings.enginePageSize)
    }

    static let marked = KeyAction.setMarkedText(MarkedText(text: "品詞", highlightedLength: 2))

    @Test func defaultIsOnAndOffKeepsTheListEvenWhenConfident() throws {
        #expect(Settings.default.candidateHideWhenConfident)
        let h = try Self.harness(settings: Self.offSettings)
        h.type("hinsi")
        #expect(h.press(.space) == [Self.marked, .showCandidates(Self.model())])
        #expect(h.converting?.candidatesHidden == false)
    }

    @Test func firstSpaceShowsOnlyTheTopCandidateUnderlined() throws {
        let h = try Self.harness()
        h.type("hinsi")
        #expect(h.press(.space) == [Self.marked])
        #expect(h.converting?.candidatesHidden == true)
        #expect(h.converting?.selected == 0)
    }

    @Test func secondSpaceShowsTheListWithoutMovingTheSelection() throws {
        let h = try Self.harness()
        h.type("hinsi")
        h.press(.space)
        #expect(h.press(.space) == [.showCandidates(Self.model())])
        #expect(h.converting?.candidatesHidden == false)
        #expect(h.converting?.selected == 0)
        // 次の Space から、今までどおり進む
        #expect(h.press(.space) == [.setMarkedText(MarkedText(text: "瀕死", highlightedLength: 2)), .moveSelection(Self.model(selected: 1, annotation: "ひんし(瀕死)"))])
    }

    @Test func downAndUpShowTheList() throws {
        for key in [KeyInput.Key.down, .up] {
            let h = try Self.harness()
            h.type("hinsi")
            h.press(.space)
            #expect(h.press(key) == [.showCandidates(Self.model())])
            #expect(h.converting?.selected == 0)
        }
    }

    @Test func aDigitShowsTheListInsteadOfPicking() throws {
        let h = try Self.harness()
        h.type("hinsi")
        h.press(.space)
        #expect(h.press(.character("2")) == [.showCandidates(Self.model())])
        #expect(h.converting != nil)
        // 一覧が出たあとは、今までどおり番号で選んで確定する
        #expect(h.press(.character("2")) == [.commit("瀕死"), .hideCandidates])
    }

    @Test func enterCommitsTheTopCandidateWhileHidden() throws {
        let h = try Self.harness()
        h.type("hinsi")
        h.press(.space)
        #expect(h.press(.enter) == [.commit("品詞"), .hideCandidates])
        #expect(h.machine.state == .idle)
        #expect(h.engine.calls.last == .commit(conversionID: 7, candidateIndex: 0))
    }

    @Test func escapeReturnsToTheReadingWhileHidden() throws {
        let h = try Self.harness()
        h.type("hinsi")
        h.press(.space)
        let actions = h.press(.escape)
        #expect(actions == [.hideCandidates, .setMarkedText(MarkedText(text: "ひんし", highlightedLength: 3))])
        #expect(h.machine.state == .composing(kana: "ひんし", pending: ""))
    }

    @Test func notConfidentKeepsTheList() throws {
        let h = try Self.harness(confident: false)
        h.type("hinsi")
        #expect(h.press(.space) == [Self.marked, .showCandidates(Self.model())])
    }

    @Test func fallbackNeverHidesTheList() throws {
        let h = try Self.harness()
        h.engine.onConvert = { _ in throw EngineClientError.timeout }
        h.type("hinsi")
        let actions = h.press(.space)
        #expect(actions.contains { if case .showCandidates = $0 { true } else { false } })
    }

    @Test func severalSegmentsKeepTheList() throws {
        let layout = SegmentLayoutInfo(
            conversionID: 7, layoutRev: 0,
            segments: [SegmentSummary(reading: "ひん", surface: "品"), SegmentSummary(reading: "し", surface: "詞")],
            focused: 0,
            focusedPage: SegmentPage(
                segment: 0, pageIndex: 0, candidates: [CandidateItem(surface: "品")], hasNext: false))
        let h = try Self.harness(layout: layout)
        h.type("hinsi")
        let actions = h.press(.space)
        #expect(actions.contains { if case .showCandidates = $0 { true } else { false } })
        #expect(h.converting?.candidatesHidden == false)
    }
}
