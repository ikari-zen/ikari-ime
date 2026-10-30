// アプリ全体のモデル: 殻との接続（自動の再試行を含む）、選択中のタブ、接続ごとのタブのモデル。
// 接続が切れたら操作を無効にし（`session = nil`）、殻が戻れば再接続でつなぎ直す（ADR 0005 §6.3「切れたとき」）。
import Foundation
import IkariBridge
import Observation

/// 1 回の接続に結びついたタブのモデル。接続が切れたら捨て、つなぎ直したら作り直す
@MainActor
final class Session {
    let client: BridgeClient
    let settings: SettingsModel
    let dictionary: DictionaryModel
    let learning: LearningModel
    let about: AboutModel

    init(client: BridgeClient, dialogs: any DialogPresenting, presets: KeymapPresetTable, dataDirectory: URL) {
        self.client = client
        settings = SettingsModel(bridge: client, presets: presets)
        dictionary = DictionaryModel(bridge: client)
        learning = LearningModel(bridge: client, dialogs: dialogs, dataDirectory: dataDirectory)
        about = AboutModel(bridge: client)
    }

    /// 開いたタブの内容を取り直す
    func reload(_ tab: SettingsTab) async {
        switch tab {
        case .input, .conversion: await settings.load()
        case .userDictionary: await dictionary.load()
        case .learning: await learning.load()
        case .about: await about.load()
        }
    }
}

enum ConnectionPhase: Equatable {
    case connecting(attempt: Int)
    case connected
    case disconnected(DisconnectReason)
}

enum DisconnectReason: Equatable {
    /// 殻が待っていない（ikari が動いていない）
    case shellNotRunning
    /// 版が合わない
    case versionMismatch
    /// つながっていた殻が止まった（`killall ikari`、入れ替え）
    case lost
    /// そのほか
    case failed

    var message: String {
        switch self {
        case .shellNotRunning:
            return "ikari に接続できません。ikari を入力ソースに選ぶと自動で起動します。選んでから「再接続」を押してください"
        case .versionMismatch:
            return Messages.describe(BridgeClientError.versionMismatch)
        case .lost:
            return "ikari との接続が切れました。ikari を入力ソースに選び直してから「再接続」を押してください"
        case .failed:
            return "ikari に接続できませんでした。「再接続」を押してください"
        }
    }
}

@MainActor @Observable
final class AppModel {
    var selectedTab: SettingsTab
    private(set) var phase: ConnectionPhase = .connecting(attempt: 0)
    private(set) var session: Session?

    private let connector: any BridgeConnecting
    private let dialogs: any DialogPresenting
    private let presets: KeymapPresetTable
    private let dataDirectory: URL
    private let appVersion: String
    private let sleep: @Sendable (Duration) async -> Void
    private var connectTask: Task<Void, Never>?

    init(
        connector: any BridgeConnecting,
        dialogs: any DialogPresenting,
        initialTab: SettingsTab = .input,
        presets: KeymapPresetTable = .bundled(),
        dataDirectory: URL = LearningModel.defaultDataDirectory(),
        appVersion: String = AboutModel.bundleVersion(),
        sleep: @escaping @Sendable (Duration) async -> Void = { try? await Task.sleep(for: $0) }
    ) {
        self.connector = connector
        self.dialogs = dialogs
        self.selectedTab = initialTab
        self.presets = presets
        self.dataDirectory = dataDirectory
        self.appVersion = appVersion
        self.sleep = sleep
    }

    /// つなぐ（失敗したら 5 秒おきに 1 分まで自動で試す）。すでに動いていれば何もしない
    func start() {
        guard connectTask == nil else { return }
        connectTask = Task { [weak self] in
            await self?.connectLoop()
            self?.connectTask = nil
        }
    }

    /// 自動の再試行をやめている（または接続していない）ので、手動の再接続を受けられる
    var reconnectAvailable: Bool { connectTask == nil }

    /// 手動の再接続
    func reconnect() {
        guard connectTask == nil else { return }
        start()
    }

    /// 起動の引数や殻の通知で渡されるタブ名（`SettingsTab.rawValue`）。知らない名前は無視する
    func openTab(named name: String) {
        guard let tab = SettingsTab(rawValue: name) else { return }
        selectedTab = tab
    }

    /// 接続しているあいだ、殻の通知を処理する。切れたら戻る
    private func serve(_ client: BridgeClient) async {
        for await event in client.events {
            switch event {
            case .settingsChanged:
                await session?.settings.reloadSettings()
            case .openTab(let name):
                openTab(named: name)
            case .disconnected:
                break
            }
        }
    }

    func stop() {
        connectTask?.cancel()
        session?.client.close()
    }

    private enum Outcome {
        case connected(BridgeClient)
        case failed(DisconnectReason)
    }

    private func connectLoop() async {
        var attempt = 0
        while !Task.isCancelled {
            phase = .connecting(attempt: attempt)
            switch await tryConnect() {
            case .connected(let client):
                let session = Session(client: client, dialogs: dialogs, presets: presets, dataDirectory: dataDirectory)
                self.session = session
                phase = .connected
                await session.reload(selectedTab)
                await serve(client)
                // 切れた。操作を無効にして、自動の再試行に入る
                self.session = nil
                phase = .disconnected(.lost)
                attempt = 0
            case .failed(let reason):
                phase = .disconnected(reason)
                // 版の不一致は待っても直らない
                guard reason != .versionMismatch else { return }
            }
            guard ReconnectPolicy.shouldRetry(afterFailedAttempt: attempt) else { return }
            await sleep(ReconnectPolicy.interval)
            attempt += 1
        }
    }

    private func tryConnect() async -> Outcome {
        do {
            let transport = try connector.makeTransport()
            return .connected(try await BridgeClient.connect(transport: transport, appVersion: appVersion))
        } catch SocketConnectError.notListening {
            return .failed(.shellNotRunning)
        } catch BridgeClientError.versionMismatch {
            return .failed(.versionMismatch)
        } catch BridgeClientError.disconnected {
            // 殻が受けた直後に切った（相手の署名の確認に通らなかったときもここに来る）
            return .failed(.shellNotRunning)
        } catch {
            return .failed(.failed)
        }
    }
}
