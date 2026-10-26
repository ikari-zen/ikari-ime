// `context.shared_exclude`（ADR 0015 D6）。既定、読み書き、除外の判定。
import Testing

@testable import IkariShellCore

@Suite struct SharedExcludeSettingsTests {
    @Test func defaultCoversPasswordManagersAndTerminals() {
        let settings = Settings.default
        #expect(settings.contextSharedExclude == Settings.defaultContextSharedExclude)
        for id in [
            "com.1password.1password", "com.bitwarden.desktop", "com.apple.keychainaccess",
            "com.apple.Terminal", "com.googlecode.iterm2", "com.github.wez.wezterm", "com.mitchellh.ghostty",
        ] {
            #expect(!settings.sharesContext(bundleID: id), "\(id)")
        }
        #expect(settings.sharesContext(bundleID: "com.apple.TextEdit"))
    }

    @Test func emptyBundleIDDoesNotShare() {
        #expect(!Settings.default.sharesContext(bundleID: ""))
    }

    @Test func parsesTheListAndKeepsOrder() {
        let result = Settings.parse(tsv: "context.shared_exclude\tcom.a.b, com.c.d ,com.a.b,,com.e-f_g\n")
        #expect(result.warnings.isEmpty)
        #expect(result.settings.contextSharedExclude == ["com.a.b", "com.c.d", "com.e-f_g"])
        #expect(!result.settings.sharesContext(bundleID: "com.c.d"))
        // 既定の一覧は置き換わる
        #expect(result.settings.sharesContext(bundleID: "com.apple.Terminal"))
    }

    @Test func emptyValueMeansNoExclusion() {
        let result = Settings.parse(tsv: "context.shared_exclude\t\n")
        #expect(result.warnings.isEmpty)
        #expect(result.settings.contextSharedExclude.isEmpty)
        #expect(result.settings.sharesContext(bundleID: "com.apple.Terminal"))
    }

    @Test func invalidCharactersWarnAndKeepTheDefault() {
        let result = Settings.parse(tsv: "context.shared_exclude\tcom.a b;rm\n")
        #expect(result.warnings.count == 1)
        #expect(!result.warnings[0].contains("rm"))
        #expect(result.settings.contextSharedExclude == Settings.defaultContextSharedExclude)
    }

    @Test func effectiveEntriesRoundTrip() {
        var settings = Settings.default
        settings.contextSharedExclude = ["com.x.y", "com.z.w"]
        let tsv = settings.effectiveEntries.map { "\($0.key)\t\($0.value)" }.joined(separator: "\n")
        let parsed = Settings.parse(tsv: tsv)
        #expect(parsed.warnings.isEmpty)
        #expect(parsed.settings.contextSharedExclude == ["com.x.y", "com.z.w"])

        settings.contextSharedExclude = []
        let emptyTSV = settings.effectiveEntries.map { "\($0.key)\t\($0.value)" }.joined(separator: "\n")
        #expect(Settings.parse(tsv: emptyTSV).settings.contextSharedExclude.isEmpty)
    }

    @Test func isAcceptableForTheKey() {
        #expect(Settings.isKnownKey("context.shared_exclude"))
        #expect(Settings.isAcceptable(key: "context.shared_exclude", value: "com.a.b,com.c.d"))
        #expect(Settings.isAcceptable(key: "context.shared_exclude", value: ""))
        #expect(!Settings.isAcceptable(key: "context.shared_exclude", value: "com.a b"))
    }

    @Test func fileUpdateReplacesOnlyThatLine() {
        let text = "input.method\tkana\ncontext.shared_exclude\tcom.a.b\n"
        let result = SettingsFile.update(
            text: text, set: [(key: "context.shared_exclude", value: "com.a.b,com.c.d")], reset: [])
        #expect(result == "input.method\tkana\ncontext.shared_exclude\tcom.a.b,com.c.d\n")
        let reset = SettingsFile.update(text: text, set: [], reset: ["context.shared_exclude"])
        #expect(reset == "input.method\tkana\n")
    }

    @Test func excludeChangeDoesNotRestartTheEngine() {
        var changed = Settings.default
        changed.contextSharedExclude = []
        #expect(!Settings.default.engineSettingsDiffer(from: changed))
    }
}

/// 設定の変更を接続中のセッションへ反映する（`InputStateMachine.setShareContext`）
@Suite struct SetShareContextTests {
    private func begins(_ h: Harness) -> [SessionInfo] {
        h.engine.calls.compactMap { call -> SessionInfo? in
            if case .beginSession(let s) = call { s } else { nil }
        }
    }

    @Test func excludingStopsSharingAndReincludingResumesIt() throws {
        let h = try Harness()
        h.machine.activate()  // 最初の BeginSession（既定は共有しない）
        h.machine.setShareContext(false)  // 変化なし
        h.machine.setShareContext(true)  // 除外を外した
        h.machine.setShareContext(true)  // 変化なし
        h.machine.setShareContext(false)  // 除外にした
        #expect(begins(h).map(\.shareContext) == [false, true, false])
        #expect(begins(h).allSatisfy { $0.sessionID == Harness.sessionID && $0.clientBundleID == "com.apple.TextEdit" })
    }

    @Test func secureStaysSecureWhenSharingChanges() throws {
        let h = try Harness(secure: true)
        h.machine.activate()
        h.machine.setShareContext(true)
        let last = try #require(begins(h).last)
        // 貸し借りしないのはエンジンの側（secure のセッションは入れない）。殻は secure を落とさずに送る
        #expect(last.secure)
        #expect(last.shareContext)
    }
}
