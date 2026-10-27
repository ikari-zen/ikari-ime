// 英語のモードの郵便番号（ADR 0032 §9.1。issue #27）。状態機械と、`EnglishMode.handleText` を通した設定・環境の扱い。
import Testing

@testable import IkariShellCore

private func ch(_ text: String) -> KeyInput { KeyInput(key: .character(text)) }
private func chars(_ text: String) -> [KeyInput] { text.map { ch(String($0)) } }

private func feed(_ machine: inout PostalMachine, _ keys: [KeyInput]) -> PostalStep? {
    var last: PostalStep?
    for key in keys { last = machine.handle(key, canEnter: true) }
    return last
}

private func underlined(_ step: PostalStep?) -> Bool {
    for action in step?.actions ?? [] {
        if case .setMarkedText(let marked) = action { return marked.underlined }
    }
    return false
}

private func rows(_ machine: PostalMachine) -> [String]? {
    if case .choosing(let choosing) = machine.phase { return choosing.rows }
    return nil
}

private func showsCandidates(_ step: PostalStep?) -> Bool {
    step?.actions.contains { if case .showCandidates = $0 { true } else { false } } ?? false
}

private let code = "1000001"
private let chiyoda = "東京都千代田区千代田"

@Suite struct PostalMachineTests {
    @Test func plainDigitsAndOtherLettersAreNotTaken() {
        var machine = PostalMachine()
        #expect(machine.handle(ch("1"), canEnter: true) == nil)
        #expect(machine.handle(ch("u"), canEnter: true) == nil)
        #expect(!machine.isActive)
    }

    @Test func notWordStartDoesNotEnter() {
        var machine = PostalMachine()
        #expect(machine.handle(ch("t"), canEnter: false) == nil)
    }

    @Test func opensOnTheSeventhDigitWithoutUnderline() {
        var machine = PostalMachine()
        let before = feed(&machine, chars("te100000"))
        #expect(!underlined(before))
        #expect(rows(machine) == nil)
        #expect(!showsCandidates(before))
        let step = feed(&machine, [ch("1")])
        #expect(!underlined(step))
        #expect(showsCandidates(step))
        #expect(step?.lookup?.code == code)
        #expect(rows(machine) == ["te1000001", "〒100-0001", "〒1000001"])
    }

    @Test func downAndUpMoveAndEnterCommits() {
        var machine = PostalMachine()
        _ = feed(&machine, chars("te1000001"))
        _ = feed(&machine, [KeyInput(key: .down)])
        let step = feed(&machine, [KeyInput(key: .enter)])
        #expect(step?.committed == "〒100-0001")
        #expect(step?.passKey == false)
        #expect(!machine.isActive)
        _ = feed(&machine, chars("te1000001"))
        _ = feed(&machine, [KeyInput(key: .up)])
        #expect(feed(&machine, [KeyInput(key: .enter)])?.committed == "〒1000001")
    }

    @Test func enterOnZeroCommitsTheTypedText() {
        var machine = PostalMachine()
        _ = feed(&machine, chars("te1000001"))
        #expect(feed(&machine, [KeyInput(key: .enter)])?.committed == "te1000001")
    }

    @Test func spaceTabDigitAndLettersCommitAsTypedAndPassTheKey() {
        for key in [KeyInput(key: .space), ch("\t"), ch("2"), ch("a"), KeyInput(key: .left)] {
            var machine = PostalMachine()
            _ = feed(&machine, chars("te1000001"))
            _ = feed(&machine, [KeyInput(key: .down)])
            let step = feed(&machine, [key])
            #expect(step?.committed == "te1000001")
            #expect(step?.passKey == true)
            #expect(step?.actions.contains(.hideCandidates) == true)
            #expect(!machine.isActive)
        }
    }

    @Test func escapeCommitsTheTypedTextAndConsumesTheKey() {
        var machine = PostalMachine()
        _ = feed(&machine, chars("te1000001"))
        let step = feed(&machine, [KeyInput(key: .escape)])
        #expect(step?.committed == "te1000001")
        #expect(step?.passKey == false)
        #expect(!machine.isActive)
    }

    @Test func commandModifiersRetype() {
        var machine = PostalMachine()
        _ = feed(&machine, chars("te10"))
        let step = machine.handle(KeyInput(key: .character("v"), modifiers: [.command]), canEnter: true)
        #expect(step?.committed == "te10")
        #expect(step?.passKey == true)
    }

    @Test func backspaceShortensAndClosesTheCandidates() {
        var machine = PostalMachine()
        _ = feed(&machine, chars("te1000001"))
        let step = feed(&machine, [KeyInput(key: .backspace)])
        #expect(step?.actions.contains(.hideCandidates) == true)
        #expect(step?.passKey == false)
        #expect(machine.phase == .typing(.te, "te100000"))
        // もう一度 7 桁目を打てば開き直す
        #expect(feed(&machine, [ch("1")])?.lookup?.code == code)
    }

    @Test func backspaceEmptiesAndCancels() {
        var machine = PostalMachine()
        let step = feed(&machine, [ch("t"), KeyInput(key: .backspace)])
        #expect(step?.passKey == false)
        #expect(!machine.isActive)
    }

    @Test func nonMatchingKeyRetypes() {
        for text in ["to", "tex", "te1a"] {
            var machine = PostalMachine()
            let step = feed(&machine, chars(text))
            #expect(step?.committed == String(text.dropLast()))
            #expect(step?.passKey == true)
            #expect(!machine.isActive)
        }
    }

    @Test func lookupAddsAddressesWhenTheStateIsTheSame() throws {
        var machine = PostalMachine()
        let request = try #require(feed(&machine, chars("te1000001"))?.lookup)
        let step = machine.lookupArrived(request, addresses: [chiyoda])
        #expect(rows(machine)?.last == chiyoda)
        #expect(showsCandidates(step))
        // 同じ返事がもう一度来ても足さない
        #expect(machine.lookupArrived(request, addresses: [chiyoda]) == nil)
        #expect(rows(machine)?.count == 4)
    }

    @Test func emptyAddressesAddNothing() throws {
        var machine = PostalMachine()
        let request = try #require(feed(&machine, chars("te1000001"))?.lookup)
        #expect(machine.lookupArrived(request, addresses: []) == nil)
        #expect(rows(machine)?.count == 3)
    }

    @Test func staleLookupIsDropped() throws {
        var machine = PostalMachine()
        let first = try #require(feed(&machine, chars("te1000001"))?.lookup)
        _ = feed(&machine, [KeyInput(key: .escape)])
        #expect(machine.lookupArrived(first, addresses: ["x"]) == nil)
        // 同じ番号を打ち直しても、前の問い合わせの返事は捨てる
        let second = try #require(feed(&machine, chars("te1000001"))?.lookup)
        #expect(second.code == first.code && second.serial != first.serial)
        #expect(machine.lookupArrived(first, addresses: ["x"]) == nil)
        #expect(rows(machine)?.count == 3)
        #expect(machine.lookupArrived(second, addresses: ["y"]) != nil)
    }

    @Test func addressesCanBeSelectedAndCommitted() throws {
        var machine = PostalMachine()
        let request = try #require(feed(&machine, chars("te1000001"))?.lookup)
        _ = machine.lookupArrived(request, addresses: [chiyoda])
        _ = feed(&machine, [KeyInput(key: .up)])
        #expect(feed(&machine, [KeyInput(key: .enter)])?.committed == chiyoda)
    }

    @Test func flushCommitsTheTypedTextWithoutPassingTheKey() {
        var machine = PostalMachine()
        #expect(machine.flush() == nil)
        _ = feed(&machine, chars("te10"))
        let typing = machine.flush()
        #expect(typing?.committed == "te10")
        #expect(typing?.passKey == false)
        _ = feed(&machine, chars("te1000001"))
        let choosing = machine.flush()
        #expect(choosing?.committed == "te1000001")
        #expect(choosing?.actions.contains(.hideCandidates) == true)
    }
}

@Suite struct EnglishPostalModeTests {
    private func environment(
        postal: Bool = true, bundleID: String = "com.apple.TextEdit", secure: Bool = false, isRepeat: Bool = false
    ) -> EnglishShortcodeEnvironment {
        var settings = Settings.default
        settings.postalEnglish = postal
        return EnglishShortcodeEnvironment(settings: settings, clientBundleID: bundleID, secure: secure, isRepeat: isRepeat)
    }

    private func type(_ text: String, into mode: inout EnglishMode, environment: EnglishShortcodeEnvironment) -> EnglishTextOutcome? {
        var last: EnglishTextOutcome?
        for key in chars(text) { last = mode.handleText(key, environment: environment) }
        return last
    }

    @Test func settingDefaultsToOffAndOffChangesNothing() {
        #expect(Settings.default.postalEnglish == false)
        var mode = EnglishMode()
        let outcome = type("te1000001", into: &mode, environment: environment(postal: false))
        #expect(outcome?.passKey == true)
        #expect(!mode.isPostalActive)
        #expect(outcome?.lookup == nil)
    }

    @Test func opensWhenOn() {
        var mode = EnglishMode()
        let outcome = type("te1000001", into: &mode, environment: environment())
        #expect(outcome?.passKey == false)
        #expect(outcome?.lookup?.code == code)
        #expect(mode.isPostalActive)
    }

    @Test func excludedAppSecureAndRepeatDoNotEnter() {
        var settings = Settings.default
        settings.postalEnglish = true
        settings.emojiShortcodeExclude = ["com.example.excluded"]
        let cases: [EnglishShortcodeEnvironment] = [
            EnglishShortcodeEnvironment(
                settings: settings, clientBundleID: "com.example.excluded", secure: false, isRepeat: false),
            environment(secure: true),
            environment(isRepeat: true),
        ]
        for env in cases {
            var mode = EnglishMode()
            let outcome = mode.handleText(ch("t"), environment: env)
            #expect(outcome.passKey)
            #expect(!mode.isPostalActive)
        }
    }

    @Test func onlyAtWordStart() {
        var mode = EnglishMode()
        _ = type("abc", into: &mode, environment: environment())
        let outcome = type("te1000001", into: &mode, environment: environment())
        #expect(outcome?.passKey == true)
        #expect(!mode.isPostalActive)
        // 空白のあとは働く
        _ = mode.handleText(KeyInput(key: .space), environment: environment())
        _ = type("te1000001", into: &mode, environment: environment())
        #expect(mode.isPostalActive)
    }

    @Test func committedTextGoesToTheContextPool() {
        var mode = EnglishMode()
        _ = type("te1000001", into: &mode, environment: environment())
        let escape = mode.handleText(KeyInput(key: .escape), environment: environment())
        #expect(escape.passKey == false)
        #expect(mode.pool.lastCharacter == "1")
    }

    @Test func turningTheSettingOffWhileTypingCommitsAndPassesTheKey() {
        var mode = EnglishMode()
        _ = type("te10", into: &mode, environment: environment())
        let outcome = mode.handleText(ch("0"), environment: environment(postal: false))
        #expect(outcome.passKey)
        #expect(!mode.isPostalActive)
    }

    @Test func lookupArrivedThroughTheModeUpdatesTheCandidates() throws {
        var mode = EnglishMode()
        let request = try #require(type("te1000001", into: &mode, environment: environment())?.lookup)
        let outcome = mode.postalLookupArrived(request, addresses: [chiyoda])
        #expect(outcome?.actions.isEmpty == false)
        _ = mode.flushPostal()
        #expect(mode.postalLookupArrived(request, addresses: [chiyoda]) == nil)
    }

    @Test func fetchUsesTheEngineOnceAndTreatsFailureAsEmpty() {
        let engine = MockEngine()
        engine.onPostalLookup = { _ in [chiyoda] }
        #expect(fetchPostalAddresses(engine: engine, code: code) == [chiyoda])
        #expect(engine.calls == [.postalLookup(code: code, postalLookupTimeout)])
        engine.onPostalLookup = { _ in throw EngineClientError.engine(.badRequest) }
        #expect(fetchPostalAddresses(engine: engine, code: code).isEmpty)
    }
}
