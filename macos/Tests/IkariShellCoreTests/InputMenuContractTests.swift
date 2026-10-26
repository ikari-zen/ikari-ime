import Testing

@testable import IkariShellCore

/// 契約コミットで固定する型の性質（`InputMenuModel.build` と `SettingsFile.update` の中身はレーン S3 のテストが持つ）。
@Suite struct InputMenuContractTests {
    @Test func settingsTabRawValuesAreTheNamesCarriedByTheBridge() {
        #expect(SettingsTab.allCases.map(\.rawValue) == ["input", "conversion", "dictionary", "learning", "about"])
        #expect(SettingsTab(rawValue: "dictionary") == .userDictionary)
    }

    @Test func menuStateIsAnEquatableKeyForCachingTheMenu() {
        let base = InputMenuState(settings: .default, lmEnabled: true)
        #expect(base == InputMenuState(settings: .default, lmEnabled: true))
        #expect(base != InputMenuState(settings: Settings(keymapPreset: .msIme), lmEnabled: true))
        #expect(base != InputMenuState(settings: .default, lmEnabled: false))
        #expect(base != InputMenuState(settings: .default, lmEnabled: true, lmState: .loading))
        let domain = ProfileDomain(id: "it", name: "IT", weight: .often)
        #expect(base != InputMenuState(settings: .default, lmEnabled: true, profile: [domain]))
    }

    @Test func radioGroupsCoverTheKeymapPresetAndTheInputMethod() {
        #expect(Set(MenuRadioGroup.allCases) == [.keymapPreset, .inputMethod])
        let selected = MenuItemSpec(
            kind: .radio(.keymapPreset), title: "macOS 風", command: .selectKeymapPreset(.macos), isChecked: true)
        let other = MenuItemSpec(kind: .radio(.keymapPreset), title: "MS-IME 風", command: .selectKeymapPreset(.msIme))
        #expect(selected.kind == other.kind)
        #expect(selected.isChecked && !other.isChecked)
        #expect(MenuItemSpec.separator.command == nil)
    }
}
