import Foundation
import Security
import Testing

@testable import IkariXPC

@Suite struct BundlePinningTests {
    @Test func settingsURLIsInsideTheShellBundle() {
        let shell = URL(fileURLWithPath: "/Applications/ikari.app")
        #expect(BundlePinning.settingsURL(shellBundle: shell).path == "/Applications/ikari.app/Contents/Helpers/IkariSettings.app")
    }

    @Test func shellURLIsFoundOnlyFromTheExpectedLayout() {
        let ok = URL(fileURLWithPath: "/Applications/ikari.app/Contents/Helpers/IkariSettings.app")
        #expect(BundlePinning.shellURL(settingsBundle: ok)?.path == "/Applications/ikari.app")
        #expect(BundlePinning.shellURL(settingsBundle: URL(fileURLWithPath: "/Applications/IkariSettings.app")) == nil)
        #expect(BundlePinning.shellURL(settingsBundle: URL(fileURLWithPath: "/tmp/x/Contents/Other/IkariSettings.app")) == nil)
        #expect(BundlePinning.shellURL(settingsBundle: URL(fileURLWithPath: "/tmp/x/Contents/Helpers/IkariSettings.app")) == nil)
    }

    @Test func missingBundleHasNoCodeHash() {
        #expect(BundlePinning.cdhash(ofBundleAt: URL(fileURLWithPath: "/nonexistent-ikari-test/x.app")) == nil)
    }

    @Test func missingBundlesMakeNoAdhocExpectation() {
        let shell = URL(fileURLWithPath: "/nonexistent-ikari-test/ikari.app")
        #expect(PeerExpectation.settingsApp(mode: .adhoc, shellBundle: shell) == nil)
        #expect(PeerExpectation.settingsApp(mode: .team, shellBundle: shell)?.expectedPath == nil)
        let settings = URL(fileURLWithPath: "/nonexistent-ikari-test/ikari.app/Contents/Helpers/IkariSettings.app")
        #expect(PeerExpectation.bridgeShell(mode: .adhoc, settingsBundle: settings) == nil)
        // 構造が想定と違う設定アプリ
        #expect(PeerExpectation.bridgeShell(mode: .adhoc, settingsBundle: URL(fileURLWithPath: "/tmp/IkariSettings.app")) == nil)
    }

    @Test func pathMatchesResolvesSymlinks() throws {
        let directory = try makeTemporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let real = directory.appendingPathComponent("real")
        try FileManager.default.createDirectory(at: real, withIntermediateDirectories: true)
        let link = directory.appendingPathComponent("link")
        try FileManager.default.createSymbolicLink(at: link, withDestinationURL: real)
        #expect(BundlePinning.pathMatches(link, expected: real))
        #expect(!BundlePinning.pathMatches(directory, expected: real))
    }

    @Test func pathMatchesIgnoresTrailingSlashButNotPrefixesOrDotDot() {
        let expected = URL(fileURLWithPath: "/Applications/ikari.app/Contents/Helpers/IkariSettings.app")
        #expect(BundlePinning.pathMatches(URL(string: "file:///Applications/ikari.app/Contents/Helpers/IkariSettings.app/")!, expected: expected))
        #expect(BundlePinning.pathMatches(URL(fileURLWithPath: "/Applications/ikari.app/Contents/Helpers/IkariSettings.app/", isDirectory: true), expected: expected))
        #expect(BundlePinning.pathMatches(URL(fileURLWithPath: "/Applications/x/../ikari.app/Contents/Helpers/IkariSettings.app"), expected: expected))
        #expect(!BundlePinning.pathMatches(URL(fileURLWithPath: "/Applications/ikari.app/Contents/Helpers/IkariSettings.app-evil/"), expected: expected))
        #expect(!BundlePinning.pathMatches(URL(fileURLWithPath: "/Applications/ikari.app-evil/Contents/Helpers/IkariSettings.app"), expected: expected))
    }

    /// 本物と同じバイナリは通り、同じ識別子で ad-hoc 署名した別のバイナリは弾かれる
    @Test func cdhashRequirementRejectsAnotherBinaryWithTheSameIdentifier() throws {
        let directory = try makeTemporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let identifier = IkariIdentifiers.settings
        let genuine = directory.appendingPathComponent("genuine")
        let other = directory.appendingPathComponent("other")
        try FileManager.default.copyItem(atPath: "/usr/bin/true", toPath: genuine.path)
        try FileManager.default.copyItem(atPath: "/usr/bin/false", toPath: other.path)
        try adhocSign(genuine, identifier: identifier)
        try adhocSign(other, identifier: identifier)

        let hash = try #require(BundlePinning.cdhash(ofBundleAt: genuine))
        let pinned = try #require(SigningRequirements.settingsApp(mode: .adhoc, cdhash: hash))
        #expect(satisfies(genuine, requirement: pinned))
        #expect(!satisfies(other, requirement: pinned))
        // 識別子だけの要件では、別のバイナリも通る（cdhash を足す理由）
        #expect(satisfies(other, requirement: "identifier \"\(identifier)\""))
    }

    // MARK: 補助

    private func makeTemporaryDirectory() throws -> URL {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("ikari-pin-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        return url
    }

    private func adhocSign(_ url: URL, identifier: String) throws {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/codesign")
        process.arguments = ["--force", "-s", "-", "-i", identifier, url.path]
        process.standardError = FileHandle.nullDevice
        try process.run()
        process.waitUntilExit()
        #expect(process.terminationStatus == 0)
    }

    private func satisfies(_ url: URL, requirement text: String) -> Bool {
        var staticCode: SecStaticCode?
        var requirement: SecRequirement?
        guard SecStaticCodeCreateWithPath(url as CFURL, SecCSFlags(), &staticCode) == errSecSuccess, let staticCode,
            SecRequirementCreateWithString(text as CFString, SecCSFlags(), &requirement) == errSecSuccess, let requirement
        else { return false }
        return SecStaticCodeCheckValidity(staticCode, SecCSFlags(), requirement) == errSecSuccess
    }
}
