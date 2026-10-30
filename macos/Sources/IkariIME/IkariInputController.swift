// IMK のコントローラ（ADR 0004 §6.2、ADR 0041 §2.2）。IMK の呼び出しを shell-core のイベントにして `ShellDriver` に渡すだけ。
// 状態（変換、モード、英語のたまり）は shell-core が持つ。入力セッション（クライアントのアプリごと）に 1 つ作られる。
import AppKit
import Carbon
import IkariProto
import IkariShellABI
import InputMethodKit

@objc(IkariInputController)
final class IkariInputController: IMKInputController, ShellSession {
    private var driver: ShellDriver { AppContext.shared.driver }

    // MARK: ShellSession

    var textClient: (any IMKTextInput)? { client() }

    func selectMode(_ modeID: String) {
        client()?.selectMode(modeID)
    }

    // MARK: IMK

    override func activateServer(_ sender: Any!) {
        super.activateServer(sender)
        driver.activate(self)
    }

    override func deactivateServer(_ sender: Any!) {
        driver.deactivate(self)
        super.deactivateServer(sender)
    }

    override func commitComposition(_ sender: Any!) {
        driver.commitComposition(session: self)
    }

    override func inputControllerWillClose() {
        driver.close(self)
    }

    override func handle(_ event: NSEvent!, client sender: Any!) -> Bool {
        guard let event, event.type == .keyDown else { return false }
        let modifiers = KeyEvents.modifiers(fromFlagBits: event.modifierFlags.rawValue)
        // Ctrl / Option / Command を押しているとき、`characters` は制御文字などになる。キー割り当ては押したキーで引く
        let usesChord = modifiers.control || modifiers.option || modifiers.command
        let characters = (usesChord ? event.charactersIgnoringModifiers : event.characters) ?? ""
        let input = KeyEvents.keyInput(
            keyCode: event.keyCode, characters: characters, modifiers: modifiers, isRepeat: event.isARepeat,
            secure: IsSecureEventInputEnabled())
        return driver.key(input, session: self)
    }

    /// macOS からのモードの通知（`kTextServiceInputModePropertyTag`。値はモードの辞書のキー。ADR 0026 §12）
    override func setValue(_ value: Any!, forTag tag: Int, client sender: Any!) {
        if tag == kTextServiceInputModePropertyTag, let modeID = value as? String {
            driver.inputModeChanged(modeID, session: self)
        }
        super.setValue(value, forTag: tag, client: sender)
    }

    // MARK: 入力メニュー（ADR 0005 §5）

    /// 入力メニューを開くたびに呼ばれる（打鍵ごとに呼ばれることもある）。中身が前回と同じなら前回のメニューを返す。エンジンは呼ばない
    override func menu() -> NSMenu! {
        guard let model = driver.currentMenu() else { return NSMenu(title: "ikari") }
        // IMK は `menu()` を主スレッドで呼ぶ。NSMenu は Sendable ではないので、結果は変数で受ける
        nonisolated(unsafe) var menu: NSMenu?
        MainActor.assumeIsolated { menu = SharedMenu.cache.menu(for: model) }
        return menu
    }

    /// メニューの項目を選んだとき（IMK は項目の `action` をコントローラに送る。`target` は使わない）
    @objc func ikariMenuCommand(_ sender: Any?) {
        guard let command = MenuActions.command(from: sender) else { return }
        driver.menuCommand(command)
    }

    /// 「環境設定…」の項目（`action = showPreferences:`）
    override func showPreferences(_ sender: Any!) {
        var command = MenuCommand()
        command.openSettings = Ikari_Shell_V1_OpenSettings()
        driver.menuCommand(command)
    }
}

/// 入力メニューのキャッシュはプロセスで 1 つ（中身は shell-core が 1 つだけ持つ）
@MainActor
private enum SharedMenu {
    static let cache = MenuCache(
        selectors: MenuSelectors(
            command: #selector(IkariInputController.ikariMenuCommand(_:)),
            preferences: #selector(IMKInputController.showPreferences(_:))))
}

/// 本番の画面。候補ウィンドウはプロセスで 1 つを共有する（入力セッションごとに作らない）。
@MainActor
final class PanelUI: ShellUI {
    private let panel = CandidatePanel()
    /// 候補ウィンドウのアンカー。変換文字列の先頭に置き、閉じるまで動かさない（文節の移動でも動かさない。#86）
    private var anchor: CGRect?
    /// 同じアプリで最後に取れた正常なキャレットの矩形（フォールバック②）
    private var lastKnown: [String: CGRect] = [:]

    func showCandidates(_ model: CandidatePanelModel, access: ClientAccess) -> (side: PanelSide, source: AnchorSource) {
        var source = AnchorSource.unspecified
        let rect: CGRect
        if let anchor {
            rect = anchor
        } else {
            let resolved = resolveAnchor(access)
            rect = resolved.rect
            source = resolved.source
            anchor = rect
        }
        let visibleFrame = Self.screen(containing: rect).visibleFrame
        let side = panel.show(model: model, anchor: rect, visibleFrame: visibleFrame, level: access.windowLevel)
        return (side, source)
    }

    func moveSelection(_ model: CandidatePanelModel) -> PanelSide? {
        panel.update(model: model)
    }

    func hideCandidates() {
        anchor = nil
        panel.hide()
    }

    func openSettings(_ tab: Ikari_Shell_V1_SettingsTab) {
        SettingsLauncher.open(tab: tab)
    }

    /// アンカーを選ぶ（どれを使うかは shell-core が決める）。順は、クライアントのキャレット、アクセシビリティ（権限があるときだけ）、
    /// 同じアプリの最後の正常値、マウスのあるスクリーンの中央下
    private func resolveAnchor(_ access: ClientAccess) -> (rect: CGRect, source: AnchorSource) {
        let bundleID = access.bundleIdentifier
        var request = Ikari_Shell_V1_PanelAnchorRequest()
        request.mouseScreenVisibleFrame = .init(Self.mouseScreen().visibleFrame)
        for caret in access.caretRects() {
            request.client = .init(caret)
            if let anchor = ShellPanelGeometry.resolveAnchor(request), anchor.source == .client {
                lastKnown[bundleID] = CGRect(anchor.rect)
                return (CGRect(anchor.rect), .client)
            }
        }
        request.clearClient()
        if let accessibility = AccessibilityAnchor.focusedElementBottomLeft() { request.accessibility = .init(accessibility) }
        if let last = lastKnown[bundleID] { request.lastKnown = .init(last) }
        guard let anchor = ShellPanelGeometry.resolveAnchor(request) else {
            let frame = Self.mouseScreen().visibleFrame
            return (CGRect(x: frame.midX, y: frame.midY, width: 0, height: 0), .mouseScreen)
        }
        return (CGRect(anchor.rect), anchor.source)
    }

    private static func screen(containing rect: CGRect) -> NSScreen {
        let point = CGPoint(x: rect.minX, y: rect.midY)
        return NSScreen.screens.first { $0.frame.contains(point) } ?? mouseScreen()
    }

    private static func mouseScreen() -> NSScreen {
        let mouse = NSEvent.mouseLocation
        return NSScreen.screens.first { $0.frame.contains(mouse) } ?? NSScreen.main ?? NSScreen.screens[0]
    }
}
