// 設定の処理は shell-core（ikari_settings_*）を通すので、XCFramework のあるビルドだけで流す（ADR 0041 §2.3）
#if IKARI_HAS_SHELL_FFI
import IkariProto
import IkariShellABI
import Testing

@testable import IkariSettings

/// 書いた表記を shell-core が読み戻して、同じ表記と表示になるか（表記の読み書きは shell-core。ADR 0041 §2.3）
private func readsBack(_ chord: KeyChord) -> Bool {
    let text = KeyChordText.string(for: chord)
    guard let info = SettingsCore.chord(.text(text)) else { return false }
    return info.valid && info.text == text && info.display == KeyChordText.display(for: chord)
}

@Suite struct KeyChordTextTests {
    @Test func writesTheSettingsNotationInAFixedOrder() {
        #expect(KeyChordText.string(for: KeyChord(key: "j", modifiers: .control)) == "ctrl+j")
        #expect(KeyChordText.string(for: KeyChord(key: "f6")) == "f6")
        #expect(KeyChordText.string(for: KeyChord(key: ";", modifiers: [.command, .shift, .control])) == "ctrl+shift+command+;")
        #expect(KeyChordText.string(for: KeyChord(key: "+", modifiers: .control)) == "ctrl++")
    }

    @Test func writtenNotationIsReadBack() {
        for chord in [
            KeyChord(key: "j", modifiers: .control), KeyChord(key: "f20"), KeyChord(key: "+", modifiers: .option),
            KeyChord(key: "l", modifiers: [.control, .shift, .option, .command]),
        ] {
            #expect(readsBack(chord))
        }
    }

    @Test func displaysWithModifierSymbols() {
        #expect(KeyChordText.display(for: KeyChord(key: "j", modifiers: .control)) == "⌃J")
        #expect(KeyChordText.display(for: KeyChord(key: "f6")) == "F6")
        #expect(KeyChordText.display(for: KeyChord(key: "k", modifiers: [.command, .shift, .option, .control])) == "⌃⌥⇧⌘K")
        #expect(KeyChordText.display(forText: "ctrl+;") == "⌃;")
        #expect(KeyChordText.display(forText: "???") == "???")
    }

    @Test func recordsACharacterWithACommandModifier() {
        // kVK_ANSI_J = 38
        #expect(
            KeyChordText.record(keyCode: 38, charactersIgnoringModifiers: "j", modifiers: .control)
                == .chord(KeyChord(key: "j", modifiers: .control)))
        // Shift が付いたときの大文字は小文字にそろえる
        #expect(
            KeyChordText.record(keyCode: 38, charactersIgnoringModifiers: "J", modifiers: [.control, .shift])
                == .chord(KeyChord(key: "j", modifiers: [.control, .shift])))
        // kVK_ANSI_Semicolon = 41
        #expect(
            KeyChordText.record(keyCode: 41, charactersIgnoringModifiers: ";", modifiers: .control)
                == .chord(KeyChord(key: ";", modifiers: .control)))
    }

    @Test func recordsFunctionKeysWithOrWithoutModifiers() {
        #expect(KeyChordText.record(keyCode: 97, charactersIgnoringModifiers: "", modifiers: []) == .chord(KeyChord(key: "f6")))
        #expect(
            KeyChordText.record(keyCode: 109, charactersIgnoringModifiers: "", modifiers: .shift)
                == .chord(KeyChord(key: "f10", modifiers: .shift)))
    }

    @Test func plainOrShiftOnlyCharactersWouldStealTyping() {
        #expect(KeyChordText.record(keyCode: 38, charactersIgnoringModifiers: "j", modifiers: []) == .rejected(.needsModifier))
        #expect(KeyChordText.record(keyCode: 38, charactersIgnoringModifiers: "J", modifiers: .shift) == .rejected(.needsModifier))
    }

    @Test func keysTheStateMachineDoesNotLookUpAreRefused() {
        #expect(
            KeyChordText.record(keyCode: KeyEvents.space, charactersIgnoringModifiers: " ", modifiers: .control)
                == .rejected(.unsupportedKey))
        #expect(
            KeyChordText.record(keyCode: KeyEvents.returnKey, charactersIgnoringModifiers: "\r", modifiers: .control)
                == .rejected(.unsupportedKey))
        #expect(
            KeyChordText.record(keyCode: KeyEvents.delete, charactersIgnoringModifiers: "\u{7F}", modifiers: .control)
                == .rejected(.unsupportedKey))
        #expect(
            KeyChordText.record(keyCode: KeyEvents.escape, charactersIgnoringModifiers: "\u{1B}", modifiers: .control)
                == .rejected(.unsupportedKey))
    }

    @Test func displaysArrowsAsSymbolsWithModifiers() {
        #expect(KeyChordText.display(for: KeyChord(key: "left")) == "←")
        #expect(KeyChordText.display(for: KeyChord(key: "right", modifiers: .shift)) == "⇧→")
        #expect(KeyChordText.display(for: KeyChord(key: "left", modifiers: [.control, .shift])) == "⌃⇧←")
        #expect(KeyChordText.display(forText: "shift+left") == "⇧←")
        #expect(KeyChordText.display(forText: "right") == "→")
        // 矢印以外は今までどおり大文字
        #expect(KeyChordText.display(for: KeyChord(key: "r", modifiers: [.control, .shift])) == "⌃⇧R")
    }

    @Test func arrowNotationIsWrittenAndReadBack() {
        for chord in [KeyChord(key: "left"), KeyChord(key: "right", modifiers: .shift)] {
            #expect(readsBack(chord))
        }
        #expect(KeyChordText.string(for: KeyChord(key: "left", modifiers: .shift)) == "shift+left")
    }

    @Test func recordsArrowKeysWithOrWithoutModifiers() {
        #expect(
            KeyChordText.record(keyCode: KeyEvents.leftArrow, charactersIgnoringModifiers: "\u{F702}", modifiers: [])
                == .chord(KeyChord(key: "left")))
        #expect(
            KeyChordText.record(keyCode: KeyEvents.rightArrow, charactersIgnoringModifiers: "\u{F703}", modifiers: .shift)
                == .chord(KeyChord(key: "right", modifiers: .shift)))
        #expect(
            KeyChordText.record(keyCode: KeyEvents.leftArrow, charactersIgnoringModifiers: "\u{F702}", modifiers: [.control, .option])
                == .chord(KeyChord(key: "left", modifiers: [.control, .option])))
    }

    @Test func escapeAloneCancelsTheRecording() {
        #expect(KeyChordText.record(keyCode: KeyEvents.escape, charactersIgnoringModifiers: "\u{1B}", modifiers: []) == .cancel)
    }
}

@Suite struct MessagesTests {
    @Test func describesBridgeAndEngineErrorsWithoutTheUsersData() {
        #expect(Messages.describe(BridgeClientError.disconnected).contains("接続が切れました"))
        #expect(Messages.describe(BridgeClientError.versionMismatch).contains("選び直す"))
        #expect(Messages.describe(BridgeClientError.bridge(.invalidSetting, key: "paging.size")).contains("paging.size"))
        #expect(Messages.describe(BridgeClientError.engine(.badRequest)).contains("古い"))
        #expect(Messages.describe(BridgeClientError.bridge(.engineUnavailable, key: "")).contains("つながりません"))
    }

    @Test func describesDataIssues() {
        var issue = Ikari_Engine_V1_DataIssue()
        issue.kind = .schemaUnsupported
        issue.section = "user_dict"
        issue.foundSchema = 2
        issue.supportedSchema = 1
        let text = Messages.describe(issue, changedNothing: true)
        #expect(text.contains("版 2"))
        #expect(text.contains("user_dict"))
        #expect(text.contains("版 1"))
        #expect(text.hasSuffix("何も変更していません"))

        issue = Ikari_Engine_V1_DataIssue()
        issue.kind = .parse
        issue.line = 7
        #expect(Messages.describe(issue).contains("7 行目"))

        issue = Ikari_Engine_V1_DataIssue()
        issue.kind = .invalidEntry
        issue.detail = "読みはひらがな（とー）にしてください"
        #expect(Messages.describe(issue) == "読みはひらがな（とー）にしてください")
        #expect(!Messages.hasIssue(Ikari_Engine_V1_DataIssue()))
    }

    @Test func formatsCountsAsInTheImportConfirmation() {
        var counts = Ikari_Engine_V1_DataCounts()
        counts.userDictEntries = 3
        counts.learningRows = 120
        counts.profileDomains = 2
        counts.ingestSources = 1
        #expect(Messages.describe(counts) == "辞書 3 語、学習 120 行、分野 2、取り込み元 1")
    }
}

#endif
