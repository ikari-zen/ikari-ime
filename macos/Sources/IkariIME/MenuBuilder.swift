// 入力メニューの `NSMenu` への写しとキャッシュ（ADR 0005 §5.1、§15.2-4）。
// 項目の中身は `IkariShellCore.InputMenuModel`（純関数）が決め、ここは `NSMenuItem` に写すだけにする。
// `menu()` は打鍵ごとに呼ばれる（S0-1: 30 分で 2048 回）ので、`InputMenuState` が前回と同じなら前回の `NSMenu` を使い回す。
// IMK は項目の `target` を使わない。`action` はコントローラのメソッドのセレクタで、選ばれた項目の `representedObject` から
// `MenuCommand` を取り出す（`MenuCommand` は enum なので、クラスに包んで入れる）。
import AppKit
import IkariShellCore
import InputMethodKit

/// `representedObject` に入れる、`MenuCommand` の包み
final class MenuCommandBox: NSObject {
    let command: IkariShellCore.MenuCommand

    init(_ command: IkariShellCore.MenuCommand) {
        self.command = command
    }
}

/// 項目を選んだときにコントローラが受けるセレクタ。`IkariInputController` の `@objc` メソッドを指す
struct MenuSelectors {
    /// 通常の項目（`ikariMenuCommand:`）
    var command: Selector
    /// 「環境設定…」の項目（`showPreferences:`。ADR 0005 §5.2）
    var preferences: Selector
}

@MainActor
final class MenuCache {
    private let selectors: MenuSelectors
    private var cachedState: InputMenuState?
    private var cachedMenu: NSMenu?

    /// 作り直した回数（測定とテストのため）
    private(set) var buildCount = 0

    init(selectors: MenuSelectors) {
        self.selectors = selectors
    }

    /// `state` が前回と同じなら、前回の `NSMenu` をそのまま返す（打鍵ごとに呼ばれても軽くする）
    func menu(for state: InputMenuState) -> NSMenu {
        if let cachedMenu, cachedState == state { return cachedMenu }
        let menu = Self.makeMenu(InputMenuModel.build(state: state), selectors: selectors)
        cachedState = state
        cachedMenu = menu
        buildCount += 1
        return menu
    }

    static func makeMenu(_ specs: [MenuItemSpec], selectors: MenuSelectors) -> NSMenu {
        let menu = NSMenu(title: "ikari")
        menu.autoenablesItems = false
        for spec in specs {
            guard let command = spec.command else {
                menu.addItem(.separator())
                continue
            }
            let selector = command == .openSettings(tab: nil) ? selectors.preferences : selectors.command
            // keyEquivalent は付けない（入力メニューでは効かない）。target も付けない（IMK は使わない）
            let item = NSMenuItem(title: spec.title, action: selector, keyEquivalent: "")
            item.state = spec.isChecked ? .on : .off
            item.isEnabled = spec.isEnabled
            item.representedObject = MenuCommandBox(command)
            menu.addItem(item)
        }
        return menu
    }
}

enum MenuActions {
    /// アクションが受けた引数から `MenuCommand` を取り出す。引数は選ばれた `NSMenuItem` か、IMK の infoDictionary
    /// （`kIMKCommandMenuItemName` に `NSMenuItem` が入る）。どちらでも受ける（S0-1 では項目の実行は直接の呼び出しだった）
    static func command(from sender: Any?) -> IkariShellCore.MenuCommand? {
        let item: NSMenuItem?
        if let direct = sender as? NSMenuItem {
            item = direct
        } else if let info = sender as? [AnyHashable: Any] {
            item = info[kIMKCommandMenuItemName] as? NSMenuItem
        } else {
            item = nil
        }
        return (item?.representedObject as? MenuCommandBox)?.command
    }
}
