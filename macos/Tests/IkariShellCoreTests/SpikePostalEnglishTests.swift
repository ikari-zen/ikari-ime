// スパイク（#27、捨てるコード）の状態機械の確認。
import Testing

@testable import IkariShellCore

private func ch(_ text: String) -> KeyInput { KeyInput(key: .character(text)) }

private func feed(_ machine: inout SpikePostalMachine, _ keys: [KeyInput]) -> SpikeStep? {
    var last: SpikeStep?
    for key in keys { last = machine.handle(key, canEnter: true, now: 0) }
    return last
}

@Suite struct SpikePostalEnglishTests {
    @Test func digitsOpenChoicesAndRequestLookup() {
        var machine = SpikePostalMachine()
        let step = feed(&machine, "1000001".map { ch(String($0)) } + [KeyInput(key: .down)])
        #expect(step?.trace.result == "choices")
        #expect(step?.lookup == "1000001")
        let arrived = machine.lookupArrived(code: "1000001", addresses: SpikePostalTable.lookup("1000001"))
        #expect(arrived.trace.result == "lookup_arrived")
    }

    @Test func staleLookupIsIgnored() {
        var machine = SpikePostalMachine()
        _ = feed(&machine, "1000001".map { ch(String($0)) } + [ch("\t")])
        _ = machine.handle(KeyInput(key: .escape), canEnter: true, now: 0)
        #expect(machine.lookupArrived(code: "1000001", addresses: ["x"]).trace.result == "lookup_stale")
    }

    @Test func teOffersPostalMarkFirst() {
        var machine = SpikePostalMachine()
        let step = feed(&machine, "te1000001".map { ch(String($0)) } + [ch("\t")])
        #expect(step?.trace.result == "choices")
        guard case .choosing(let choosing) = machine.phase else { Issue.record("not choosing"); return }
        #expect(choosing.rows == ["te1000001", "〒100-0001", "〒1000001"])
    }

    @Test func unicodeHex() {
        var machine = SpikePostalMachine()
        _ = feed(&machine, [ch("u"), ch("+"), ch("3"), ch("0"), ch("4"), ch("2"), ch("\t")])
        guard case .choosing(let choosing) = machine.phase else { Issue.record("not choosing"); return }
        #expect(choosing.rows.last == "あ")
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
        #expect(machine.handle(ch("1"), canEnter: false, now: 0) == nil)
    }

    @Test func backspaceEmptiesAndCancels() {
        var machine = SpikePostalMachine()
        let step = feed(&machine, [ch("1"), KeyInput(key: .backspace)])
        #expect(step?.trace.result == "cancel")
        #expect(!machine.isActive)
    }
}
