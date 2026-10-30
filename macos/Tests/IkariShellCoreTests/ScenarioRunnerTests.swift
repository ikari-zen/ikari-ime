// 共通のシナリオ（shell-core/scenarios/*.json。ADR 0041 §5）を、今の Swift の状態機械で流す。
// 移す前の振る舞いを JSON に固めるためのもの。Rust 版（shell-core/tests/scenarios.rs）と同じファイルを読む。
// 移行の間だけ使い、M5 で消す。
import Foundation
import Testing

@testable import IkariShellCore

@Suite struct ScenarioRunnerTests {
    @Test func everyScenarioMatchesTheSwiftStateMachine() throws {
        let files = try ScenarioFiles.load()
        #expect(!files.isEmpty)
        var count = 0
        for (name, file) in files {
            let defaults = file.defaults ?? ScenarioSetup()
            for scenario in file.scenarios {
                // 殻の判断（IkariIME にあり、このテストから呼べない）は Rust の実行器だけで流す
                if scenario.rustOnly != nil { continue }
                count += 1
                do {
                    try run(scenario, setup: defaults.resolve(scenario.setup))
                } catch {
                    Issue.record("\(name) / \(scenario.name): \(error)")
                }
            }
        }
        #expect(count > 0)
    }

    private func run(_ scenario: Scenario, setup: ResolvedScenarioSetup) throws {
        let parsed = Settings.parse(tsv: setup.settings.joined(separator: "\n"))
        guard parsed.warnings.isEmpty else { throw ScenarioFormatError("settings warnings: \(parsed.warnings)") }
        let engine = MockEngine()
        let script = ScriptCounter(setup.engine)
        let pages = setup.engine.pages
        engine.onConvert = { _ in
            if let error = script.next("convert") { throw error }
            guard let page = pages.first else { throw EngineClientError.engine(.badRequest) }
            return page.conversionPage
        }
        engine.onPage = { _, index in
            if let error = script.next("page") { throw error }
            guard pages.indices.contains(index) else { throw EngineClientError.engine(.badRequest) }
            return pages[index].conversionPage
        }
        let engineScript = setup.engine
        engine.onSegmentPage = { _, segment, index in
            if let error = script.next("segment_page") { throw error }
            guard let page = engineScript.segmentPage(segment: segment, index: index) else {
                throw EngineClientError.engine(.badRequest)
            }
            return page
        }
        let layoutRevs = LayoutRevs()
        engine.onResize = { _, segment, delta in
            if let error = script.next("resize_segment") { throw error }
            guard let layout = engineScript.resize(segment: segment, delta: delta, layoutRev: layoutRevs.last) else {
                throw EngineClientError.engine(.badRequest)
            }
            return layout
        }
        engine.onReconvert = { _ in
            if let error = script.next("reconvert") { throw error }
            guard let layout = engineScript.reconvert?.value else { throw EngineClientError.engine(.badRequest) }
            return layout
        }
        engine.onPostalLookup = { code in
            if let error = script.next("postal_lookup") { throw error }
            return engineScript.postal[code] ?? []
        }

        let table = ShortcodeTable(entries: setup.engine.emojiTable.map { ShortcodeEntry(name: $0.name, emoji: $0.emoji) })
        let h = try Harness(
            preset: parsed.settings.keymapPreset, settings: parsed.settings, secure: setup.client.secure,
            engine: engine, bundleID: setup.client.bundleID, shortcodeTable: table)
        h.surroundingText = setup.surrounding.machineValue

        for (index, step) in scenario.steps.enumerated() {
            let at = "step \(index + 1)"
            if let surrounding = step.surrounding { h.surroundingText = surrounding.machineValue }
            h.selection = step.selection?.machineValue
            let before = engine.calls.count
            var actions: [KeyAction] = []
            for key in try step.keys() {
                // `resize_segment` の台本の `layout_rev` は、要求の版で引く
                layoutRevs.last = h.converting?.layoutRev ?? 0
                actions += h.machine.handle(
                    key, surrounding: h.surroundingText, selection: h.selection, isRepeat: false)
            }
            switch step.host {
            case nil: break
            case .commitComposition: actions += h.machine.commitComposition()
            case .panelShown(let side): h.machine.setPanelSide(side)
            case .panelHidden: h.machine.setPanelSide(.below)
            case .unsupported(let type): throw ScenarioFormatError("\(at): the Swift runner cannot run host \(type)")
            }
            if let request = step.modeRequest, request != "none" {
                throw ScenarioFormatError("\(at): the Swift state machine never requests a mode")
            }
            if let expected = step.expect {
                let actual = actions.map(ScenarioAction.init)
                guard actual == expected else {
                    throw ScenarioFormatError("\(at): actions differ\n  expected: \(expected)\n  actual:   \(actual)")
                }
            }
            if let expected = step.engine {
                let sent = engine.calls.dropFirst(before).map(\.scenarioValue)
                let matches =
                    expected.count == sent.count
                    && zip(expected, sent).allSatisfy { JSONValue.object($0).matches($1) }
                guard matches else {
                    throw ScenarioFormatError(
                        "\(at): engine requests differ\n  expected: \(expected.map { JSONValue.object($0) })\n  actual:   \(sent)")
                }
            }
        }
    }
}

/// 区切り直しの要求の版（`MockEngine.onResize` は版を受け取らないので、キーの前の状態から写す）
private final class LayoutRevs: @unchecked Sendable {
    var last: UInt32 = 0
}

/// 台本の失敗を、要求の種類ごとの回数で引く
private final class ScriptCounter: @unchecked Sendable {
    private let lock = NSLock()
    private let script: ScenarioEngineScript
    private var counts: [String: Int] = [:]

    init(_ script: ScenarioEngineScript) { self.script = script }

    func next(_ request: String) -> EngineClientError? {
        lock.lock()
        defer { lock.unlock() }
        counts[request, default: 0] += 1
        return script.failure(for: request, nth: counts[request]!)
    }
}
