// 画面に出す設定の一覧（shell-core の `settings::all_specs`。ADR 0040 §2.5）の全部のキーが、設定アプリにあることを確かめる。
// 一覧は Rust が持つ。ここは一覧を読み、画面の Model（`SettingsModel`）から各キーを書けることを確かめるだけで、
// 何が一覧に入るかの判断は持たない。一覧の書き出し（`macos/Resources/settings-specs.tsv`）が今の一覧と同じことは、
// shell-core のテスト（`golden_file_for_the_screens_is_current`）が守る。
#if IKARI_HAS_SHELL_FFI
import Foundation
import Testing

@testable import IkariSettings

/// 一覧の書き出しの 1 行（列は `specs_tsv` の doc）
private struct SpecRow {
    var key: String
    var kind: String
    var section: String
    var screen: String
    var defaultText: String
    var reflect: String
}

private func registryRows() throws -> [SpecRow] {
    let url = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        .appendingPathComponent("Resources/settings-specs.tsv")
    let text = try String(contentsOf: url, encoding: .utf8)
    return text.split(separator: "\n", omittingEmptySubsequences: true).map { line in
        let cols = line.split(separator: "\t", omittingEmptySubsequences: false).map(String.init)
        return SpecRow(
            key: cols[0], kind: cols[1], section: cols[2], screen: cols[3], defaultText: cols[4], reflect: cols[5])
    }
}

private let keymapPrefix = "keymap."

@Suite @MainActor struct SettingsRegistryTests {
    /// 画面が持つ操作。キーごとに、一覧の既定値と違う値を書く（既定のままだと行を書かないため）
    private func write(_ row: SpecRow, with model: SettingsModel) async throws {
        let flipped = row.defaultText == "on" || row.defaultText == "true" ? false : true
        switch row.key {
        case "input.method": await model.setInputMethod(.kana)
        case "keymap.preset": await model.setKeymapPreset(.atok)
        case "paging.wrap": await model.setPagingWrap(flipped)
        case "paging.size": await model.setPageSize(try otherInt(row))
        case "candidate.hide_when_confident": await model.setCandidateHideWhenConfident(flipped)
        case "clause.min_chars": await model.setClauseMinChars(try otherInt(row))
        case "engine.lm": await model.setLMEnabled(flipped)
        case "engine.idle_unload_secs": await model.setIdleUnloadSecs(try otherInt(row))
        case "engine.lm_deadline_ms": await model.setLMDeadlineMs(try otherInt(row))
        case "engine.convert_wait_ms": await model.setConvertWaitMs(try otherInt(row))
        case "emoji.shortcode": await model.setEmojiShortcode(flipped)
        case "emoji.small_letters": await model.setEmojiSmallLetters(flipped)
        case "emoji.shortcode_exclude": await model.addEmojiShortcodeExclude("com.example.registry")
        case "postal.japanese": await model.setPostalJapanese(flipped)
        case "postal.te_prefix": await model.setPostalTePrefix(flipped)
        case "postal.english": await model.setPostalEnglish(flipped)
        case "unicode.notation": await model.setUnicodeNotation(flipped)
        case "date.weekday_dates": await model.setDateWeekdayDates(flipped)
        default:
            guard row.kind == "key_chord", row.key.hasPrefix(keymapPrefix),
                let action = KeymapAction(rawValue: String(row.key.dropFirst(keymapPrefix.count)))
            else {
                Issue.record("画面にないキー: \(row.key)")
                return
            }
            await model.setKeymap(action, to: .chord("f13"))
        }
    }

    /// 一覧の範囲の中で、既定値と違う整数（範囲の両端のどちらか）
    private func otherInt(_ row: SpecRow) throws -> Int {
        let bounds = row.kind.dropFirst("int:".count).components(separatedBy: "..").compactMap { Int($0) }
        try #require(bounds.count == 2, "int の範囲を読めない: \(row.kind)")
        return Int(row.defaultText) == bounds[0] ? bounds[1] : bounds[0]
    }

    @Test func registryIsReadable() throws {
        let rows = try registryRows()
        #expect(rows.count > 18)
        #expect(Set(rows.map(\.key)).count == rows.count)
    }

    @Test func everyKeyInTheRegistryCanBeWrittenFromTheScreen() async throws {
        let rows = try registryRows()
        let harness = try await Harness.make()
        defer { harness.close() }
        let model = await makeSettingsModel(harness)
        for row in rows {
            try await write(row, with: model)
            #expect(model.errorMessage == nil, "\(row.key): \(model.errorMessage ?? "")")
            #expect(harness.shell.state.snapshot.settings[row.key] != nil, "画面から書けない: \(row.key)")
        }
    }

    /// 画面のキー割り当ての行は、一覧のキー割り当てと同じ（操作を足したら、一覧（Rust）にも足す）
    @Test func keyBindingRowsMatchTheRegistry() async throws {
        let fromRegistry = Set(
            try registryRows().filter { $0.kind == "key_chord" }.map { String($0.key.dropFirst(keymapPrefix.count)) })
        #expect(fromRegistry == Set(KeymapAction.allCases.map(\.rawValue)))
        let harness = try await Harness.make()
        defer { harness.close() }
        let model = await makeSettingsModel(harness)
        #expect(Set(model.keymapRows.map(\.action.rawValue)) == fromRegistry)
    }

    /// 一覧の既定値と範囲は、画面が持つもの（shell-core の `constants`）と同じ
    @Test func registryDefaultsAndRangesMatchTheScreenValues() throws {
        let rows = Dictionary(uniqueKeysWithValues: try registryRows().map { ($0.key, $0) })
        func range(_ key: String) -> String? {
            rows[key].map { String($0.kind.dropFirst("int:".count)) }
        }
        func text(_ range: ClosedRange<Int>) -> String { "\(range.lowerBound)..\(range.upperBound)" }
        #expect(range("paging.size") == text(SettingsValues.pageSizeRange))
        #expect(range("clause.min_chars") == text(SettingsValues.clauseMinCharsRange))
        #expect(range("engine.idle_unload_secs") == text(SettingsValues.idleUnloadRange))
        #expect(range("engine.lm_deadline_ms") == text(SettingsValues.lmDeadlineRange))
        #expect(range("engine.convert_wait_ms") == text(SettingsValues.convertWaitRange))
        let defaults = SettingsValues.defaults
        #expect(rows["paging.size"]?.defaultText == String(defaults.pageSize))
        #expect(rows["clause.min_chars"]?.defaultText == String(defaults.clauseMinChars))
        #expect(rows["engine.lm_deadline_ms"]?.defaultText == String(defaults.lmDeadlineMs))
        #expect(rows["input.method"]?.defaultText == defaults.inputMethod.rawValue)
    }
}
#endif
