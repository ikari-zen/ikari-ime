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
        engine.onSegmentPage = { _, _, _ in
            throw script.next("segment_page") ?? EngineClientError.engine(.badRequest)
        }
        engine.onResize = { _, _, _ in
            throw script.next("resize_segment") ?? EngineClientError.engine(.badRequest)
        }
        engine.onReconvert = { _ in
            throw script.next("reconvert") ?? EngineClientError.engine(.badRequest)
        }
        engine.onPostalLookup = { _ in
            if let error = script.next("postal_lookup") { throw error }
            return []
        }

        let h = try Harness(
            preset: parsed.settings.keymapPreset, settings: parsed.settings, secure: setup.client.secure,
            engine: engine, bundleID: setup.client.bundleID)
        h.surroundingText = setup.surrounding.machineValue

        for (index, step) in scenario.steps.enumerated() {
            let at = "step \(index + 1)"
            if let surrounding = step.surrounding { h.surroundingText = surrounding.machineValue }
            h.selection = step.selection?.machineValue
            let before = engine.calls.count
            var actions: [KeyAction] = []
            for key in try step.keys() {
                actions += h.machine.handle(
                    key, surrounding: h.surroundingText, selection: h.selection, isRepeat: false)
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
