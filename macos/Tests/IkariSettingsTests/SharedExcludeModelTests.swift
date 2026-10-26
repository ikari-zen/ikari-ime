// 「文脈を共有しないアプリ」の欄（ADR 0015 D6）のモデル。
import Foundation
import IkariShellCore
import Testing

@testable import IkariSettings

@Suite @MainActor struct SharedExcludeModelTests {
    @Test func startsWithTheDefaultList() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        let model = await makeSettingsModel(harness)
        #expect(model.values.contextSharedExclude == Settings.defaultContextSharedExclude)
    }

    @Test func addRemoveAndResetGoThroughTheShell() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        let model = await makeSettingsModel(harness)

        await model.addSharedExclude("  com.example.Secret ")
        #expect(model.errorMessage == nil)
        #expect(model.values.contextSharedExclude.last == "com.example.Secret")
        #expect(harness.shell.state.snapshot.settings["context.shared_exclude"]?.hasSuffix(",com.example.Secret") == true)

        await model.removeSharedExclude("com.apple.Terminal")
        #expect(!model.values.contextSharedExclude.contains("com.apple.Terminal"))

        await model.resetSharedExclude()
        #expect(model.values.contextSharedExclude == Settings.defaultContextSharedExclude)
    }

    @Test func removingEveryEntryStoresAnEmptyList() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        let model = await makeSettingsModel(harness)
        for id in Settings.defaultContextSharedExclude { await model.removeSharedExclude(id) }
        #expect(model.errorMessage == nil)
        #expect(model.values.contextSharedExclude.isEmpty)
        #expect(harness.shell.state.snapshot.settings["context.shared_exclude"] == "")
    }

    @Test func invalidOrDuplicateIDsAreNotAdded() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        let model = await makeSettingsModel(harness)
        await model.addSharedExclude("com.a b")
        #expect(model.errorMessage != nil)
        await model.addSharedExclude("com.a,com.b")
        #expect(model.values.contextSharedExclude == Settings.defaultContextSharedExclude)
        model.dismissError()
        await model.addSharedExclude("com.apple.Terminal")
        #expect(model.errorMessage == nil)
        #expect(model.values.contextSharedExclude == Settings.defaultContextSharedExclude)
    }
}

@Suite @MainActor struct SharedExcludeAppsTests {
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
        #expect(SharedExcludeApps.bundleID(ofAppAt: url) == "com.example.Secret")
    }

    @Test func appWithoutBundleIDOrNonAppGivesNil() throws {
        let url = try makeApp(named: "NoID", bundleID: nil)
        #expect(SharedExcludeApps.bundleID(ofAppAt: url) == nil)
        #expect(SharedExcludeApps.bundleID(ofAppAt: URL(fileURLWithPath: "/usr/bin")) == nil)
    }

    @Test func displayNameDropsTheAppSuffixAndFallsBackToNil() {
        func name(_ id: String, _ shown: String) -> String? {
            SharedExcludeApps.displayName(forBundleID: id, locate: located, displayNameAtPath: { _ in shown })
        }
        #expect(name("com.example.A", "A.app") == "A")
        #expect(name("com.example.A", "ターミナル") == "ターミナル")
        #expect(name("com.example.A", " ") == nil)
        #expect(name("com.example.Gone", "x") == nil)
    }

    @Test func rowsKeepOrderAndMarkUnknownApps() {
        let rows = SharedExcludeApps.rows(
            for: ["com.example.Gone", "com.example.A"], locate: located, displayNameAtPath: { _ in "A" })
        #expect(rows == [.init(bundleID: "com.example.Gone", name: nil), .init(bundleID: "com.example.A", name: "A")])
    }

    @Test func addingAnAppGoesThroughTheShell() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        let model = await makeSettingsModel(harness)
        await model.addSharedExclude(appAt: try makeApp(named: "Secret", bundleID: "com.example.Secret"))
        #expect(model.errorMessage == nil)
        #expect(model.values.contextSharedExclude.last == "com.example.Secret")
    }

    @Test func appWithoutBundleIDShowsAnError() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        let model = await makeSettingsModel(harness)
        await model.addSharedExclude(appAt: try makeApp(named: "NoID", bundleID: nil))
        #expect(model.errorMessage?.contains("NoID") == true)
        #expect(model.values.contextSharedExclude == Settings.defaultContextSharedExclude)
    }
}
