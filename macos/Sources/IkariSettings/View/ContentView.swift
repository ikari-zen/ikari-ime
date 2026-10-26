// ウィンドウの中身。つながっているあいだはタブ、切れているあいだは再接続の案内。
import IkariShellCore
import SwiftUI

struct ContentView: View {
    @Bindable var app: AppModel

    var body: some View {
        Group {
            if let session = app.session {
                ConnectedView(app: app, session: session)
            } else {
                DisconnectedView(app: app)
            }
        }
        .frame(minWidth: 680, minHeight: 540)
    }
}

private struct ConnectedView: View {
    @Bindable var app: AppModel
    let session: Session

    var body: some View {
        TabView(selection: $app.selectedTab) {
            InputTab(model: session.settings)
                .tabItem { Label("入力", systemImage: "keyboard") }
                .tag(SettingsTab.input)
            ConversionTab(model: session.settings)
                .tabItem { Label("変換", systemImage: "character.textbox") }
                .tag(SettingsTab.conversion)
            DictionaryTab(model: session.dictionary)
                .tabItem { Label("ユーザー辞書", systemImage: "text.book.closed") }
                .tag(SettingsTab.userDictionary)
            LearningTab(model: session.learning)
                .tabItem { Label("学習とデータ", systemImage: "externaldrive") }
                .tag(SettingsTab.learning)
            AboutTab(model: session.about)
                .tabItem { Label("このソフトについて", systemImage: "info.circle") }
                .tag(SettingsTab.about)
        }
        // タブを開くたびに内容を取り直す（メニューや CLI での変更を拾う）
        .onChange(of: app.selectedTab) { _, tab in
            Task { await session.reload(tab) }
        }
    }
}

private struct DisconnectedView: View {
    let app: AppModel

    var body: some View {
        VStack(spacing: 16) {
            switch app.phase {
            case .connecting(let attempt):
                ProgressView()
                Text(attempt == 0 ? "ikari に接続しています…" : "ikari に接続しています…（再試行 \(attempt)）")
            case .connected:
                ProgressView()
            case .disconnected(let reason):
                Image(systemName: "bolt.horizontal.circle").font(.system(size: 40)).foregroundStyle(.secondary)
                Text(reason.message)
                    .multilineTextAlignment(.center)
                    .frame(maxWidth: 420)
                Button("再接続") { app.reconnect() }
                    .disabled(!app.reconnectAvailable)
                    .keyboardShortcut(.defaultAction)
            }
        }
        .padding(32)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}
