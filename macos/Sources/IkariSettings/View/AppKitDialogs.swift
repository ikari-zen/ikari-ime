// `DialogPresenting` の AppKit 実装（NSAlert、NSSavePanel、NSOpenPanel、Finder）。ロジックはモデル側で、ここは薄く保つ。
import AppKit
import UniformTypeIdentifiers

@MainActor
struct AppKitDialogs: DialogPresenting {
    func confirm(_ request: ConfirmRequest) -> Bool {
        let alert = NSAlert()
        alert.messageText = request.title
        alert.informativeText = request.message
        alert.alertStyle = request.isDestructive ? .warning : .informational
        let confirmButton = alert.addButton(withTitle: request.confirmTitle)
        if request.isDestructive { confirmButton.hasDestructiveAction = true }
        // 既定のボタン（Return）は「やめる」にする。上書きや消去を勢いで通さない
        alert.addButton(withTitle: "キャンセル")
        confirmButton.keyEquivalent = ""
        alert.buttons[1].keyEquivalent = "\r"
        return alert.runModal() == .alertFirstButtonReturn
    }

    func chooseSaveLocation(suggestedName: String) -> URL? {
        let panel = NSSavePanel()
        panel.nameFieldStringValue = suggestedName
        panel.canCreateDirectories = true
        panel.title = "ikari のデータを書き出す"
        return panel.runModal() == .OK ? panel.url : nil
    }

    func chooseOpenFile() -> URL? {
        let panel = NSOpenPanel()
        panel.canChooseFiles = true
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false
        panel.title = "インポートするファイルを選ぶ"
        return panel.runModal() == .OK ? panel.url : nil
    }

    /// アプリ（`.app`）を 1 つ選ぶ。初期位置は /Applications
    func chooseApp() -> URL? {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [.application]
        panel.canChooseFiles = true
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false
        panel.directoryURL = URL(fileURLWithPath: "/Applications", isDirectory: true)
        panel.title = "文脈を共有しないアプリを選ぶ"
        return panel.runModal() == .OK ? panel.url : nil
    }

    func reveal(_ url: URL) {
        NSWorkspace.shared.open(url)
    }
}
