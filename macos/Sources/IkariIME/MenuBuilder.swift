// 入力メニューの `NSMenu` への写しとキャッシュ（ADR 0005 §5.1、§15.2-4）。
// 項目の中身は shell-core が決めて `ShowMenu` で知らせる（`Ikari_Shell_V1_MenuModel`）。ここは `NSMenuItem` に写すだけ。
// `menu()` は打鍵ごとに呼ばれる（S0-1: 30 分で 2048 回）ので、中身が前回と同じなら前回の `NSMenu` を使い回す。
// IMK は項目の `target` を使わない。`action` はコントローラのメソッドのセレクタで、選ばれた項目の `representedObject` から
// 命令を取り出す。
import AppKit
import IkariProto
import InputMethodKit

typealias MenuCommand = Ikari_Shell_V1_MenuCommand

/// `representedObject` に入れる、命令の包み
final class MenuCommandBox: NSObject {
    let command: MenuCommand

    init(_ command: MenuCommand) {
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
    private var cachedModel: Ikari_Shell_V1_MenuModel?
    private var cachedMenu: NSMenu?

    /// 作り直した回数（測定とテストのため）
    private(set) var buildCount = 0

    init(selectors: MenuSelectors) {
        self.selectors = selectors
    }

    /// `model` が前回と同じなら、前回の `NSMenu` をそのまま返す（打鍵ごとに呼ばれても軽くする）
    func menu(for model: Ikari_Shell_V1_MenuModel) -> NSMenu {
        if let cachedMenu, cachedModel == model { return cachedMenu }
        let menu = Self.makeMenu(model, selectors: selectors)
        cachedModel = model
        cachedMenu = menu
        buildCount += 1
        return menu
    }

    static func makeMenu(_ model: Ikari_Shell_V1_MenuModel, selectors: MenuSelectors) -> NSMenu {
        let menu = NSMenu(title: "ikari")
        menu.autoenablesItems = false
        for item in model.items {
            guard item.kind != .separator, item.hasCommand else {
                menu.addItem(.separator())
                continue
            }
            // 「環境設定…」（前回のタブを開く）は IMK の標準のセレクタにする
            let isPreferences: Bool = {
                if case .openSettings(let open)? = item.command.command { return open.tab == .unspecified }
                return false
            }()
            // keyEquivalent は付けない（入力メニューでは効かない）。target も付けない（IMK は使わない）
            let menuItem = NSMenuItem(
                title: item.title, action: isPreferences ? selectors.preferences : selectors.command, keyEquivalent: "")
            menuItem.state = item.checked ? .on : .off
            menuItem.isEnabled = item.enabled
            menuItem.representedObject = MenuCommandBox(item.command)
            menu.addItem(menuItem)
        }
        return menu
    }
}

enum MenuActions {
    /// アクションが受けた引数から命令を取り出す。引数は選ばれた `NSMenuItem` か、IMK の infoDictionary
    /// （`kIMKCommandMenuItemName` に `NSMenuItem` が入る）。どちらでも受ける（S0-1 では項目の実行は直接の呼び出しだった）
    static func command(from sender: Any?) -> MenuCommand? {
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
