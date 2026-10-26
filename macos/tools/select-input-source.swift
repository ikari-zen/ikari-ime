// ikari の入力ソースを TIS で扱う補助ツール（ADR 0004 §9.2）。install.sh が使う。
//
//   select-input-source reselect   ikari（日本語か英語のモード）が今の入力ソースなら、別の入力ソースを経由して選び直す
//                                  （新しい版を読み込ませる）。そうでなければ何も変えない
//   select-input-source register   ~/Library/Input Methods の ikari.app をシステムに登録する（候補に出るだけ。有効にはしない）
//   select-input-source enable     ikari（日本語と英語のモード）を有効にする（システム設定の入力ソースに追加される。明示したときだけ）
//   select-input-source prime-english  ikari の英語のモードを一度選んで、元の入力ソースに戻す。macOS は英数キーの行き先を
//                                  入力ソースの履歴から決めるので、インストールしたての履歴に英語のモードを入れておく（#303）
//   select-input-source status     有効か、選択中かを出す
//
// ユーザーの入力ソースの設定は、`enable` 以外では変えない。
import Carbon
import Foundation

/// 殻の Bundle ID。識別子の出どころは macos/scripts/lib.sh で、install.sh が環境変数 IKARI_PRODUCT_ID で渡す（ADR 0036 §3.1）
let productID: String = {
    guard let value = ProcessInfo.processInfo.environment["IKARI_PRODUCT_ID"], !value.isEmpty else {
        FileHandle.standardError.write(Data("error: IKARI_PRODUCT_ID が必要です（install.sh が lib.sh の BUNDLE_ID を渡す）\n".utf8))
        exit(1)
    }
    return value
}()
let inputSourceID = "\(productID).Japanese"
/// 英語のモード（ADR 0026。辞書のキーは com.apple.inputmethod.Roman だが、入力ソースの ID はこれ）
let englishInputSourceID = "\(productID).English"
let appPath = NSString(string: "~/Library/Input Methods/ikari.app").expandingTildeInPath

func property(_ source: TISInputSource, _ key: CFString) -> AnyObject? {
    guard let raw = TISGetInputSourceProperty(source, key) else { return nil }
    return Unmanaged<AnyObject>.fromOpaque(raw).takeUnretainedValue()
}

func flag(_ source: TISInputSource, _ key: CFString) -> Bool {
    (property(source, key) as? NSNumber)?.boolValue ?? false
}

func source(withID id: String) -> TISInputSource? {
    let filter = [kTISPropertyInputSourceID as String: id] as CFDictionary
    guard let list = TISCreateInputSourceList(filter, true)?.takeRetainedValue() as? [TISInputSource] else { return nil }
    return list.first
}

func ikariSource() -> TISInputSource? { source(withID: inputSourceID) }

func ikariEnglishSource() -> TISInputSource? { source(withID: englishInputSourceID) }

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data((message + "\n").utf8))
    exit(1)
}

switch CommandLine.arguments.dropFirst().first {
case "status":
    guard let source = ikariSource() else {
        print("not-registered")
        exit(0)
    }
    let enabled = flag(source, kTISPropertyInputSourceIsEnabled)
    let selected = flag(source, kTISPropertyInputSourceIsSelected)
    var line = "registered enabled=\(enabled) selected=\(selected)"
    if let english = ikariEnglishSource() {
        let englishEnabled = flag(english, kTISPropertyInputSourceIsEnabled)
        let englishSelected = flag(english, kTISPropertyInputSourceIsSelected)
        line += " english-enabled=\(englishEnabled) english-selected=\(englishSelected)"
    } else {
        line += " english=not-registered"
    }
    print(line)

case "register":
    let status = TISRegisterInputSource(URL(fileURLWithPath: appPath) as CFURL)
    guard status == noErr else { fail("TISRegisterInputSource failed: \(status)") }
    print("registered \(appPath)")

case "enable":
    guard let source = ikariSource() else { fail("ikari is not registered (run register, or log out and in)") }
    let status = TISEnableInputSource(source)
    guard status == noErr else { fail("TISEnableInputSource failed: \(status)") }
    if let english = ikariEnglishSource() {
        let englishStatus = TISEnableInputSource(english)
        guard englishStatus == noErr else { fail("TISEnableInputSource (English) failed: \(englishStatus)") }
    }
    print("enabled")

case "reselect":
    // 日本語と英語のモードのうち、今選ばれているほうを選び直す
    let candidates = [ikariSource(), ikariEnglishSource()].compactMap { $0 }
    guard candidates.contains(where: { flag($0, kTISPropertyInputSourceIsEnabled) }) else {
        print("ikari is not enabled; nothing to reselect")
        exit(0)
    }
    guard let source = candidates.first(where: { flag($0, kTISPropertyInputSourceIsSelected) }) else {
        print("ikari is not the current input source; nothing to reselect")
        exit(0)
    }
    // 別の入力ソース（ASCII 入力可能なもの）に切り替えて、すぐ戻す
    guard let other = TISCopyCurrentASCIICapableKeyboardInputSource()?.takeRetainedValue() else {
        fail("no ASCII capable input source")
    }
    guard TISSelectInputSource(other) == noErr else { fail("could not select the other input source") }
    Thread.sleep(forTimeInterval: 0.3)
    guard TISSelectInputSource(source) == noErr else { fail("could not reselect ikari") }
    print("reselected")

case "prime-english":
    guard let english = ikariEnglishSource(), flag(english, kTISPropertyInputSourceIsEnabled) else {
        print("ikari English is not enabled; nothing to prime")
        exit(0)
    }
    // 履歴にもう ikari の英語のモードがあれば、何もしない（入力ソースを不必要に動かさない）
    let history = CFPreferencesCopyAppValue("AppleInputSourceHistory" as CFString, "com.apple.HIToolbox" as CFString) as? [[String: Any]] ?? []
    if history.contains(where: { ($0["Bundle ID"] as? String) == productID && ($0["Input Mode"] as? String) == "com.apple.inputmethod.Roman" }) {
        print("ikari English is already in the input source history; nothing to prime")
        exit(0)
    }
    guard let original = TISCopyCurrentKeyboardInputSource()?.takeRetainedValue() else {
        fail("no current input source")
    }
    if flag(english, kTISPropertyInputSourceIsSelected) {
        print("ikari English is already selected; nothing to prime")
        exit(0)
    }
    guard TISSelectInputSource(english) == noErr else { fail("could not select ikari English") }
    Thread.sleep(forTimeInterval: 0.3)
    guard TISSelectInputSource(original) == noErr else { fail("could not restore the original input source") }
    print("primed")

default:
    FileHandle.standardError.write(Data("usage: select-input-source reselect|register|enable|prime-english|status\n".utf8))
    exit(2)
}
