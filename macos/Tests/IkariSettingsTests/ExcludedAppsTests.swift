// 設定の処理は shell-core（ikari_settings_*）を通すので、XCFramework のあるビルドだけで流す（ADR 0041 §2.3）
#if IKARI_HAS_SHELL_FFI
// 「ショートコードを使わないアプリ」の欄（ADR 0027 §8.1）で、アプリ（.app）と Bundle ID を行き来する部分。
import Foundation
import Testing

@testable import IkariSettings

@Suite @MainActor struct ExcludedAppsTests {
    private func makeApp(named name: String, bundleID: String?) throws -> URL {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let contents = root.appendingPathComponent("\(name).app/Contents")
        try FileManager.default.createDirectory(at: contents, withIntermediateDirectories: true)
        var plist: [String: Any] = ["CFBundleName": name]
        if let bundleID { plist["CFBundleIdentifier"] = bundleID }
        try (plist as NSDictionary).write(to: contents.appendingPathComponent("Info.plist"))
        return root.appendingPathComponent("\(name).app")
    }

    private let located: (String) -> URL? = {
        $0 == "com.example.A" ? URL(fileURLWithPath: "/Applications/A.app") : nil
    }

    @Test func readsTheBundleIDOfAnApp() throws {
        let url = try makeApp(named: "Secret", bundleID: "com.example.Secret")
        #expect(ExcludedApps.bundleID(ofAppAt: url) == "com.example.Secret")
    }

    @Test func appWithoutBundleIDOrNonAppGivesNil() throws {
        let url = try makeApp(named: "NoID", bundleID: nil)
        #expect(ExcludedApps.bundleID(ofAppAt: url) == nil)
        #expect(ExcludedApps.bundleID(ofAppAt: URL(fileURLWithPath: "/usr/bin")) == nil)
    }

    @Test func displayNameDropsTheAppSuffixAndFallsBackToNil() {
        func name(_ id: String, _ shown: String) -> String? {
            ExcludedApps.displayName(forBundleID: id, locate: located, displayNameAtPath: { _ in shown })
        }
        #expect(name("com.example.A", "A.app") == "A")
        #expect(name("com.example.A", "ターミナル") == "ターミナル")
        #expect(name("com.example.A", " ") == nil)
        #expect(name("com.example.Gone", "x") == nil)
    }

    @Test func rowsKeepOrderAndMarkUnknownApps() {
        let rows = ExcludedApps.rows(
            for: ["com.example.Gone", "com.example.A"], locate: located, displayNameAtPath: { _ in "A" })
        #expect(rows == [.init(bundleID: "com.example.Gone", name: nil), .init(bundleID: "com.example.A", name: "A")])
    }

    @Test func addingAnAppGoesThroughTheShell() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        let model = await makeSettingsModel(harness)
        await model.addEmojiShortcodeExclude(appAt: try makeApp(named: "Secret", bundleID: "com.example.Secret"))
        #expect(model.errorMessage == nil)
        #expect(model.values.emojiShortcodeExclude.last == "com.example.Secret")
    }

    @Test func appWithoutBundleIDShowsAnError() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        let model = await makeSettingsModel(harness)
        await model.addEmojiShortcodeExclude(appAt: try makeApp(named: "NoID", bundleID: nil))
        #expect(model.errorMessage?.contains("NoID") == true)
        #expect(model.values.emojiShortcodeExclude == SettingsValues.defaults.emojiShortcodeExclude)
    }
}

#endif
