// InputStateMachine.reconfigure（ADR 0005 §7.2）。idle のときだけ反映し、表示中の文字列と確定履歴を変えない。
import Testing

@testable import IkariShellCore

@Suite struct ReconfigureTests {
    private func keymap(_ preset: KeymapPreset, settings: Settings) throws -> Keymap {
        try Keymap(presetTSV: Harness.presetTSV(preset), settings: settings)
    }

    @Test func appliesTheNewKeymapWhenIdle() throws {
        let h = try Harness(preset: .macos)
        let settings = Settings(keymapPreset: .msIme)
        #expect(h.machine.reconfigure(settings: settings, keymap: try keymap(.msIme, settings: settings), composer: nil))

        h.type("hinsi")
        // ctrl+i は MS-IME 風では全角カタカナ。macOS 風のままなら確定してアプリへ渡される
        h.press(.character("i"), [.control])
        #expect(h.converting?.candidates.first?.surface == "ヒンシ")
    }

    @Test func dropsTheOldKeymapBindings() throws {
        let h = try Harness(preset: .macos)
        let settings = Settings(keymapPreset: .msIme)
        h.machine.reconfigure(settings: settings, keymap: try keymap(.msIme, settings: settings), composer: nil)
        h.type("hinsi")
        // ctrl+k は macOS 風だけの割り当て
        #expect(h.press(.character("k"), [.control]) == [.commit("ひんし"), .passThrough])
    }

    @Test func appliesPagingAndPageSizeToTheNextConversion() throws {
        let h = try Harness()
        let settings = Settings(pagingWrap: false, pageSize: 4)
        h.machine.reconfigure(settings: settings, keymap: try keymap(.macos, settings: settings), composer: nil)
        h.type("hinsi")
        let actions = h.press(.space)
        // エンジンにはいつも上限の件数を頼む。設定の件数は、最初に見せる数
        #expect(h.engine.convertRequests.last?.pageSize == Settings.enginePageSize)
        guard case .showCandidates(let model)? = actions.last else { Issue.record("no panel"); return }
        #expect(model.candidates.count == 3)
    }

    @Test func isIgnoredWhileComposingAndReportsFalse() throws {
        let h = try Harness(preset: .macos)
        h.type("hinsi")
        let settings = Settings(keymapPreset: .msIme, pageSize: 3)
        let applied = h.machine.reconfigure(settings: settings, keymap: try keymap(.msIme, settings: settings), composer: nil)
        #expect(!applied)
        #expect(h.machine.state == .composing(kana: "ひんし", pending: ""))

        // 古い設定のまま: macOS 風の ctrl+k が効き、ページの件数も変わらない
        h.press(.character("k"), [.control])
        #expect(h.converting?.candidates.first?.surface == "ヒンシ")
    }

    @Test func isIgnoredWhileConverting() throws {
        let h = try Harness()
        h.type("hinsi")
        h.press(.space)
        let before = h.machine.state
        let settings = Settings(pageSize: 2)
        #expect(!h.machine.reconfigure(settings: settings, keymap: try keymap(.macos, settings: settings), composer: nil))
        #expect(h.machine.state == before)
    }

    @Test func isIgnoredWhileInAnAsciiRun() throws {
        let h = try Harness()
        h.press(.character("A"), [.shift])
        guard case .asciiRun = h.machine.state else {
            Issue.record("expected asciiRun, got \(h.machine.state)")
            return
        }
        #expect(!h.machine.reconfigure(settings: .default, keymap: try keymap(.macos, settings: .default), composer: nil))
    }

    @Test func succeedsAgainOnceTheCompositionIsCommitted() throws {
        let h = try Harness()
        h.type("hinsi")
        let settings = Settings(keymapPreset: .msIme)
        let msIme = try keymap(.msIme, settings: settings)
        #expect(!h.machine.reconfigure(settings: settings, keymap: msIme, composer: nil))
        _ = h.press(.enter)
        #expect(h.machine.state == .idle)
        #expect(h.machine.reconfigure(settings: settings, keymap: msIme, composer: nil))
    }

    @Test func swapsTheComposerOnlyWhenOneIsGiven() throws {
        let h = try Harness()
        let replacement = FakeComposer()
        let settings = Settings(inputMethod: .kana)
        #expect(h.machine.reconfigure(settings: settings, keymap: try keymap(.macos, settings: settings), composer: replacement))
        h.type("ka")
        #expect(replacement.kana == "か")
        #expect(h.composer.kana.isEmpty && h.composer.pending.isEmpty)

        // composer が nil なら、今の合成器のまま
        _ = h.press(.escape)
        #expect(h.machine.reconfigure(settings: .default, keymap: try keymap(.macos, settings: .default), composer: nil))
        h.type("ki")
        #expect(replacement.kana == "き")
    }

    @Test func doesNotTouchTheSessionOrSendBeginSession() throws {
        let h = try Harness()
        h.machine.activate()
        let before = h.engine.count { if case .beginSession = $0 { true } else { false } }
        #expect(h.machine.reconfigure(settings: Settings(pageSize: 2), keymap: try keymap(.macos, settings: .default), composer: nil))
        #expect(h.engine.count { if case .beginSession = $0 { true } else { false } } == before)
        h.type("ka")
        h.press(.space)
        #expect(h.engine.convertRequests.last?.sessionID == Harness.sessionID)
    }
}
