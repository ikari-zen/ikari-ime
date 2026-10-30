// ウィンドウの中身。つながっているあいだはサイドバーつきの画面、切れているあいだは再接続の案内。
import SwiftUI

/// サイドバーと窓の幅。**値は暫定で、実測していない**（#61。PR で確認する）。
/// 窓の最小幅は、サイドバーの理想の幅に、前の最小幅（タブの版の 680）を足したもの
enum SidebarMetrics {
    static let minWidth: CGFloat = 160
    static let idealWidth: CGFloat = 190
    static let maxWidth: CGFloat = 240
    static let windowMinWidth: CGFloat = 870
    static let windowDefaultWidth: CGFloat = 950
}

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
        .frame(minWidth: SidebarMetrics.windowMinWidth, minHeight: 540)
    }
}

/// サイドバーに出す名前とアイコン
private extension SettingsTab {
    var title: String {
        switch self {
        case .input: "入力"
        case .conversion: "変換"
        case .userDictionary: "ユーザー辞書"
        case .learning: "学習とデータ"
        case .about: "このソフトについて"
        }
    }

    var systemImage: String {
        switch self {
        case .input: "keyboard"
        case .conversion: "character.textbox"
        case .userDictionary: "text.book.closed"
        case .learning: "externaldrive"
        case .about: "info.circle"
        }
    }
}

/// 左のサイドバーでタブを選ぶ。幅が狭くても項目が窓の外にはみ出さず、名前がいつも見える（#61）
private struct ConnectedView: View {
    @Bindable var app: AppModel
    let session: Session

    var body: some View {
        NavigationSplitView {
            List(SettingsTab.allCases, id: \.self, selection: $app.selectedTab) { tab in
                Label(tab.title, systemImage: tab.systemImage)
            }
            .navigationSplitViewColumnWidth(min: SidebarMetrics.minWidth, ideal: SidebarMetrics.idealWidth, max: SidebarMetrics.maxWidth)
        } detail: {
            detail
        }
        .navigationSplitViewStyle(.balanced)
        // タブを開くたびに内容を取り直す（メニューや CLI での変更を拾う）
        .onChange(of: app.selectedTab) { _, tab in
            Task { await session.reload(tab) }
        }
    }

    @ViewBuilder
    private var detail: some View {
        switch app.selectedTab {
        case .input: InputTab(model: session.settings)
        case .conversion: ConversionTab(model: session.settings)
        case .userDictionary: DictionaryTab(model: session.dictionary)
        case .learning: LearningTab(model: session.learning)
        case .about: AboutTab(model: session.about)
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
