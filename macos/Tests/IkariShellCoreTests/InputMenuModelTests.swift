// InputMenuModel.build（ADR 0005 §5.2、§15.2-3/4）。
import Testing

@testable import IkariShellCore

@Suite struct InputMenuModelTests {
    private func build(
        settings: Settings = .default, lm: Bool = true, lmState: EngineLmState? = nil, profile: [ProfileDomain]? = nil
    ) -> [MenuItemSpec] {
        InputMenuModel.build(state: InputMenuState(settings: settings, lmEnabled: lm, lmState: lmState, profile: profile))
    }

    private let domains = [
        ProfileDomain(id: "it", name: "IT・ソフトウェア", weight: .often),
        ProfileDomain(id: "infra", name: "インフラ・クラウド", weight: .sometimes),
        ProfileDomain(id: "novel", name: "小説", weight: .off),
    ]

    private func radios(_ items: [MenuItemSpec], _ group: MenuRadioGroup) -> [MenuItemSpec] {
        items.filter { $0.kind == .radio(group) }
    }

    @Test func listsTheItemsInTheOrderOfTheAdr() {
        let titles = build(profile: domains).map(\.title)
        #expect(
            titles == [
                "キー割り当て: macOS 風", "キー割り当て: MS-IME 風", "",
                "入力方式: ローマ字", "入力方式: かな", "",
                "LM（zenz）で並べ替える", "",
                "分野: IT・ソフトウェア", "分野: インフラ・クラウド（たまに）", "分野: 小説", "",
                "ユーザー辞書と学習…", "環境設定…",
            ])
    }

    @Test(arguments: KeymapPreset.allCases)
    func exactlyOneKeymapPresetIsCheckedAndItIsTheCurrentOne(preset: KeymapPreset) {
        let items = radios(build(settings: Settings(keymapPreset: preset)), .keymapPreset)
        #expect(items.count == 2)
        #expect(items.filter(\.isChecked).count == 1)
        #expect(items.first(where: \.isChecked)?.command == .selectKeymapPreset(preset))
    }

    @Test(arguments: [InputMethodKind.romaji, .kana])
    func exactlyOneInputMethodIsCheckedAndItIsTheCurrentOne(method: InputMethodKind) {
        let items = radios(build(settings: Settings(inputMethod: method)), .inputMethod)
        #expect(items.count == 2)
        #expect(items.filter(\.isChecked).count == 1)
        #expect(items.first(where: \.isChecked)?.command == .selectInputMethod(method))
    }

    @Test func everyRadioGroupHasExactlyOneCheckForEveryCombination() {
        for preset in KeymapPreset.allCases {
            for method in [InputMethodKind.romaji, .kana] {
                let items = build(settings: Settings(inputMethod: method, keymapPreset: preset))
                for group in MenuRadioGroup.allCases {
                    #expect(radios(items, group).filter(\.isChecked).count == 1)
                }
            }
        }
    }

    @Test func lmItemTogglesAndShowsTheState() throws {
        func lmItem(_ items: [MenuItemSpec]) throws -> MenuItemSpec {
            try #require(items.first { $0.title.hasPrefix("LM（zenz）") })
        }
        let on = try lmItem(build(lm: true))
        #expect(on.kind == .toggle && on.isChecked && on.command == .setLM(enabled: false))
        let off = try lmItem(build(lm: false))
        #expect(!off.isChecked && off.command == .setLM(enabled: true))

        #expect(try lmItem(build(lm: true, lmState: .loading)).title == "LM（zenz）で並べ替える（読み込み中）")
        #expect(try lmItem(build(lm: true, lmState: .failed)).title == "LM（zenz）で並べ替える（使えません）")
        #expect(try lmItem(build(lm: true, lmState: .ready)).title == "LM（zenz）で並べ替える")
        #expect(try lmItem(build(lm: true, lmState: .unspecified)).title == "LM（zenz）で並べ替える")
        // 切っているときは、古い状態が残っていても添えない
        #expect(try lmItem(build(lm: false, lmState: .loading)).title == "LM（zenz）で並べ替える")
    }

    @Test func domainItemsToggleBetweenIncludedAndExcluded() throws {
        let items = build(profile: domains)
        let it = try #require(items.first { $0.title == "分野: IT・ソフトウェア" })
        #expect(it.kind == .toggle && it.isChecked)
        #expect(it.command == .setProfileDomain(id: "it", included: false))

        // sometimes はチェックを付けて名前に添える。選ぶと外す（3 状態を回さない）
        let infra = try #require(items.first { $0.title.hasPrefix("分野: インフラ") })
        #expect(infra.isChecked)
        #expect(infra.command == .setProfileDomain(id: "infra", included: false))

        let novel = try #require(items.first { $0.title == "分野: 小説" })
        #expect(!novel.isChecked)
        #expect(novel.command == .setProfileDomain(id: "novel", included: true))
    }

    @Test func domainSectionIsOmittedUntilTheProfileIsKnown() {
        for profile in [nil, []] as [[ProfileDomain]?] {
            let items = build(profile: profile)
            #expect(!items.contains { $0.title.hasPrefix("分野") })
            // 区切りが連続しない
            for (a, b) in zip(items, items.dropFirst()) {
                #expect(!(a.kind == .separator && b.kind == .separator))
            }
        }
    }

    @Test func endsWithTheSettingsItems() {
        let items = build()
        #expect(items[items.count - 2].command == .openSettings(tab: .userDictionary))
        #expect(items[items.count - 1].command == .openSettings(tab: nil))
        #expect(items[items.count - 1].kind == .action)
    }

    @Test func noSeparatorAtTheEdgesAndEveryOtherItemHasACommand() {
        let items = build(profile: domains)
        #expect(items.first?.kind != .separator && items.last?.kind != .separator)
        for item in items {
            if item.kind == .separator {
                #expect(item.command == nil)
            } else {
                #expect(item.command != nil)
                #expect(item.isEnabled)
            }
        }
    }

    @Test func sameStateGivesTheSameResult() {
        let state = InputMenuState(settings: Settings(keymapPreset: .msIme), lmEnabled: true, lmState: .ready, profile: domains)
        #expect(InputMenuModel.build(state: state) == InputMenuModel.build(state: state))
    }
}
