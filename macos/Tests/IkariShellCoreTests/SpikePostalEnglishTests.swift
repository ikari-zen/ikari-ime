// スパイク（#27、捨てるコード）の状態機械の確認。
import Testing

@testable import IkariShellCore

private func ch(_ text: String) -> KeyInput { KeyInput(key: .character(text)) }
private func chars(_ text: String) -> [KeyInput] { text.map { ch(String($0)) } }

private func feed(_ machine: inout SpikePostalMachine, _ keys: [KeyInput]) -> SpikeStep? {
    var last: SpikeStep?
    for key in keys { last = machine.handle(key, canEnter: true, now: 0) }
    return last
}

private func underlined(_ step: SpikeStep?) -> Bool {
    for action in step?.actions ?? [] {
        if case .setMarkedText(let marked) = action { return marked.underlined }
    }
    return false
}

@Suite struct SpikePostalEnglishTests {
    @Test func plainDigitsAreNotTaken() {
        var machine = SpikePostalMachine()
        #expect(machine.handle(ch("1"), canEnter: true, now: 0) == nil)
    }

    @Test func teOpensChoicesOnTheLastDigitWithoutUnderline() {
        var machine = SpikePostalMachine()
        let before = feed(&machine, chars("te100000"))
        #expect(before?.trace.result == "typing")
        #expect(!underlined(before))
        let step = feed(&machine, [ch("1")])
        #expect(step?.trace.result == "choices")
        #expect(step?.lookup == "1000001")
        #expect(!underlined(step))
        guard case .choosing(let choosing) = machine.phase else { Issue.record("not choosing"); return }
        #expect(choosing.rows == ["te1000001", "〒100-0001", "〒1000001"])
    }

    @Test func staleLookupIsIgnored() {
        var machine = SpikePostalMachine()
        _ = feed(&machine, chars("te1000001"))
        _ = machine.handle(KeyInput(key: .escape), canEnter: true, now: 0)
        #expect(machine.lookupArrived(code: "1000001", addresses: ["x"]).trace.result == "lookup_stale")
    }

    @Test func lookupAddsAddressesWhileChoosing() {
        var machine = SpikePostalMachine()
        _ = feed(&machine, chars("te1000001"))
        let arrived = machine.lookupArrived(code: "1000001", addresses: SpikePostalTable.lookup("1000001"))
        #expect(arrived.trace.result == "lookup_arrived")
    }

    @Test func downAndUpMoveAndEnterCommits() {
        var machine = SpikePostalMachine()
        _ = feed(&machine, chars("te1000001"))
        _ = feed(&machine, [KeyInput(key: .down)])
        let step = feed(&machine, [KeyInput(key: .enter)])
        #expect(step?.trace.result == "commit")
        #expect(step?.committed == "〒100-0001")
        _ = feed(&machine, chars("te1000001"))
        let up = feed(&machine, [KeyInput(key: .up)])
        #expect(up?.trace.result == "move")
    }

    @Test func spaceAndTabDoNotSelect() {
        for key in [KeyInput(key: .space), ch("\t")] {
            var machine = SpikePostalMachine()
            _ = feed(&machine, chars("te1000001"))
            _ = feed(&machine, [KeyInput(key: .down)])
            let step = feed(&machine, [key])
            #expect(step?.trace.result == "retype")
            #expect(step?.committed == "te1000001")
            #expect(step?.passKey == true)
        }
    }

    @Test func digitKeysDoNotSelect() {
        var machine = SpikePostalMachine()
        _ = feed(&machine, chars("te1000001"))
        let step = feed(&machine, [ch("2")])
        #expect(step?.trace.result == "retype")
        #expect(step?.committed == "te1000001")
    }

    @Test func escapeKeepsTheTypedTextAndConsumesTheKey() {
        var machine = SpikePostalMachine()
        _ = feed(&machine, chars("te1000001"))
        let step = feed(&machine, [KeyInput(key: .escape)])
        #expect(step?.committed == "te1000001")
        #expect(step?.passKey == false)
        #expect(!machine.isActive)
    }

    @Test func unicodeOpensAtFourDigitsAndUpdatesUpToSix() {
        var machine = SpikePostalMachine()
        let three = feed(&machine, chars("u+304"))
        #expect(three?.trace.result == "typing")
        let four = feed(&machine, [ch("2")])
        #expect(four?.trace.result == "choices")
        guard case .choosing(let choosing) = machine.phase else { Issue.record("not choosing"); return }
        #expect(choosing.rows.last == "あ")
        _ = feed(&machine, chars("u"))
        _ = feed(&machine, [KeyInput(key: .escape)])
        _ = feed(&machine, chars("u+01F6"))
        #expect(feed(&machine, [ch("0")])?.trace.result == "choices")
        let six = feed(&machine, [ch("0")])
        #expect(six?.trace.result == "choices")
        guard case .choosing(let updated) = machine.phase else { Issue.record("not choosing"); return }
        #expect(updated.rows.last == "😀")
        #expect(feed(&machine, [ch("0")])?.trace.result == "retype")
    }

    @Test func backspaceInChoicesShortensAndClosesWhenTooShort() {
        var machine = SpikePostalMachine()
        _ = feed(&machine, chars("u+30420"))
        #expect(feed(&machine, [KeyInput(key: .backspace)])?.trace.result == "choices")
        let step = feed(&machine, [KeyInput(key: .backspace)])
        #expect(step?.trace.result == "typing")
        #expect(step?.actions.contains(.hideCandidates) == true)
    }

    @Test func nonMatchingKeyRetypes() {
        var machine = SpikePostalMachine()
        let step = feed(&machine, [ch("t"), ch("o")])
        #expect(step?.trace.result == "retype")
        #expect(step?.committed == "t")
        #expect(step?.passKey == true)
    }

    @Test func notWordStartDoesNotEnter() {
        var machine = SpikePostalMachine()
        #expect(machine.handle(ch("t"), canEnter: false, now: 0) == nil)
    }

    @Test func backspaceEmptiesAndCancels() {
        var machine = SpikePostalMachine()
        let step = feed(&machine, [ch("t"), KeyInput(key: .backspace)])
        #expect(step?.trace.result == "cancel")
        #expect(!machine.isActive)
    }
}
