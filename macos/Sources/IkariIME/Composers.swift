// 合成器と字種変換。Rust の `ikari-input`（shell-ffi の XCFramework がある `IKARI_HAS_SHELL_FFI` のとき）か、
// FFI がないビルドの代替。代替の合成器はキーを受け取らない（すべてアプリに渡す）。
import Foundation
import IkariShellCore

#if IKARI_HAS_SHELL_FFI
import IkariShellFFI

/// `IkariComposer` の薄い包み。借用の文字列は、次にその composer を呼ぶまでに Swift の String へコピーする。
final class RustComposer: Composing {
    private var handle: OpaquePointer?

    init?(method: InputMethodKind, romajiTable: Data?) {
        switch method {
        case .romaji:
            var error = IkariShellBuf(ptr: nil, len: 0)
            let created: OpaquePointer? = (romajiTable ?? Data()).withUnsafeBytes { raw in
                let base = raw.bindMemory(to: UInt8.self).baseAddress
                return ikari_composer_new_romaji(base, raw.count, &error)
            }
            ikari_shell_buf_free(error)
            handle = created
        case .kana:
            handle = ikari_composer_new_jis()
        }
        guard handle != nil else { return nil }
    }

    deinit { ikari_composer_free(handle) }

    var kana: String { Self.string(ikari_composer_kana(handle)) }
    var pending: String { Self.string(ikari_composer_pending(handle)) }

    func pushCharacter(_ scalar: Unicode.Scalar) { ikari_composer_push_char(handle, scalar.value) }
    func pushJis(key: UInt8, shift: Bool) { ikari_composer_push_jis(handle, key, shift) }
    @discardableResult func backspace() -> Bool { ikari_composer_backspace(handle) }
    func flush() { ikari_composer_flush(handle) }
    func clear() { ikari_composer_clear(handle) }

    private static func string(_ borrowed: IkariStr) -> String {
        guard let pointer = borrowed.ptr, borrowed.len > 0 else { return "" }
        return String(decoding: UnsafeBufferPointer(start: pointer, count: borrowed.len), as: UTF8.self)
    }
}

/// `ikari_convert_script`（IkariScript の値と一致させる）
struct RustScripts: ScriptConverting {
    func convert(_ text: String, to script: KeymapAction) -> String {
        let bytes = Array(text.utf8)
        let result: IkariShellBuf = bytes.withUnsafeBufferPointer { buffer in
            ikari_convert_script(buffer.baseAddress, buffer.count, Self.scriptNumber(script))
        }
        defer { ikari_shell_buf_free(result) }
        guard let pointer = result.ptr, result.len > 0 else { return text }
        return String(decoding: UnsafeBufferPointer(start: pointer, count: result.len), as: UTF8.self)
    }

    private static func scriptNumber(_ script: KeymapAction) -> UInt32 {
        switch script {
        case .toHiragana: return UInt32(IKARI_SCRIPT_HIRAGANA.rawValue)
        case .toFullwidthKatakana: return UInt32(IKARI_SCRIPT_FULLWIDTH_KATAKANA.rawValue)
        case .toHalfwidthKatakana: return UInt32(IKARI_SCRIPT_HALFWIDTH_KATAKANA.rawValue)
        case .toFullwidthAlnum: return UInt32(IKARI_SCRIPT_FULLWIDTH_ALNUM.rawValue)
        case .toHalfwidthAlnum: return UInt32(IKARI_SCRIPT_HALFWIDTH_ALNUM.rawValue)
        case .segmentFocusLeft, .segmentFocusRight, .segmentShrink, .segmentExpand, .reconvert, .candidateNext,
            .candidatePrevious, .modeToggle, .modeEnglish, .modeJapanese:
            preconditionFailure("字種変換ではない操作（KeymapAction.isScriptConversion）")
        }
    }
}

func makeComposer(method: InputMethodKind, romajiTable: Data?) -> any Composing {
    RustComposer(method: method, romajiTable: romajiTable) ?? UnavailableComposer()
}

func makeScriptConverter() -> any ScriptConverting { RustScripts() }

#else

func makeComposer(method: InputMethodKind, romajiTable: Data?) -> any Composing {
    UnavailableComposer()
}

/// FFI なしのビルドの字種変換。Foundation の変換で近似する（ひらがな ⇔ カタカナ、全角 ⇔ 半角）
struct FoundationScripts: ScriptConverting {
    func convert(_ text: String, to script: KeymapAction) -> String {
        let katakana = text.applyingTransform(.hiraganaToKatakana, reverse: false) ?? text
        switch script {
        case .toHiragana: return text
        case .toFullwidthKatakana: return katakana
        case .toHalfwidthKatakana: return katakana.applyingTransform(.fullwidthToHalfwidth, reverse: false) ?? katakana
        case .toFullwidthAlnum: return text.applyingTransform(.fullwidthToHalfwidth, reverse: true) ?? text
        case .toHalfwidthAlnum: return text.applyingTransform(.fullwidthToHalfwidth, reverse: false) ?? text
        case .segmentFocusLeft, .segmentFocusRight, .segmentShrink, .segmentExpand, .reconvert, .candidateNext,
            .candidatePrevious, .modeToggle, .modeEnglish, .modeJapanese: return text
        }
    }
}

func makeScriptConverter() -> any ScriptConverting { FoundationScripts() }

#endif

/// shell-ffi が使えないとき（XCFramework なし、または生成に失敗）の合成器。何も受け取らないので、キーはアプリに渡る
final class UnavailableComposer: Composing {
    let kana = ""
    let pending = ""
    func pushCharacter(_ scalar: Unicode.Scalar) {}
    func pushJis(key: UInt8, shift: Bool) {}
    @discardableResult func backspace() -> Bool { false }
    func flush() {}
    func clear() {}
}
