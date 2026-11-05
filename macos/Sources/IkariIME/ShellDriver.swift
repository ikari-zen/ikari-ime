// 殻の 1 本の口（ADR 0041 §4.1）を回す: OS のイベントを `ShellEvent` にして shell-core に渡し、返ってきた `ShellAction` の列を
// 順に実行する。判断はしない（ADR 0041 §2.1-2: 入口に OS の API を呼ばない分岐が増えたら shell-core に移す）。
//
// - shell-core は 1 つのスレッドから呼ぶ（本番は主スレッド。`queue`）。周辺テキストと選択は、dispatch の中から同期に読まれる。
// - エンジンへのバイト列の運び（`EngineHost`）は、どのスレッドからでも並行に呼ばれる。落ちた通知は XPC のキューから
//   `ShellDownNotifier` に直接渡す（主スレッドが返事を待っている間にも届く。§4.3）。
// - dispatch の中（コールバック）から同じ shell を呼び直さない（BUSY）。動作の実行は dispatch が返ってから行う。
// - shell-core が panic したら（PANIC）、その回はキーをアプリに渡し、次の呼び出しで shell を作り直す。
import Foundation
import IkariProto
import IkariShellABI
import IkariXPC
import InputMethodKit
import os

typealias ShellEvent = Ikari_Shell_V1_ShellEvent
typealias ShellAction = Ikari_Shell_V1_ShellAction
typealias AnchorSource = Ikari_Shell_V1_AnchorSource

/// 入力先（IMK のコントローラ 1 つ）。
protocol ShellSession: AnyObject {
    /// IMK のクライアント（取れなければ nil）
    var textClient: (any IMKTextInput)? { get }
    /// OS にモードを選んでもらう（`client.selectMode`）
    func selectMode(_ modeID: String)
}

/// 画面（候補ウィンドウと設定アプリ）。主スレッドでだけ呼ぶ。テストでは差し替えるか持たない。
@MainActor
protocol ShellUI: AnyObject {
    /// 候補ウィンドウを新しく出す（アンカーはこのとき決める）。置いた上下と、アンカーをどこから取ったか
    func showCandidates(_ model: CandidatePanelModel, access: ClientAccess) -> (side: PanelSide, source: AnchorSource)
    /// 出ている候補ウィンドウの中身を替える。出ていなければ nil
    func moveSelection(_ model: CandidatePanelModel) -> PanelSide?
    func hideCandidates()
    func openSettings(_ tab: Ikari_Shell_V1_SettingsTab)
}

/// エンジンへの運び（`IkariShellHost` の engine_*）。どのスレッドからでも並行に呼ばれる。
protocol EngineHost: Sendable {
    func connect() -> UInt64
    func request(_ connection: UInt64, _ bytes: Data, timeoutMs: UInt32) -> ShellTransportResult
    func post(_ connection: UInt64, _ bytes: Data)
    func close(_ connection: UInt64)
    func pid(_ connection: UInt64) -> Int32
}

/// 落ちた通知の宛先（shell を作り直すたびに替わる）。XPC のキューから読む
final class DownNotifierBox: @unchecked Sendable {
    private let lock = NSLock()
    private var notifier: ShellDownNotifier?

    func set(_ notifier: ShellDownNotifier?) {
        lock.withLock { self.notifier = notifier }
    }

    func engineDown(connection: UInt64, kind: ShellDownKind) {
        let notifier = lock.withLock { self.notifier }
        notifier?.engineDown(generation: connection, kind: kind)
    }
}

final class ShellDriver: @unchecked Sendable {
    /// shell の作り方（版、設定のファイル、資源の置き場所）
    struct Config {
        var shellVersion: String
        var settingsPath: String
        var resourcesDir: String
    }

    /// `PanelShown` を返したあとにまた候補ウィンドウが変わったとき、続けて知らせる回数の上限（実際は 1〜2 回）
    private static let maxPanelReports = 4

    private let config: Config
    private let engine: any EngineHost
    let downNotifier: DownNotifierBox
    private let queue: DispatchQueue
    private let isSecure: () -> Bool
    weak var ui: (any ShellUI)?
    /// 設定アプリとの輸送（`BridgeReply`・`BridgeClose` の実行）
    var bridge: BridgeServer?

    private var shell: ShellABI?
    /// 今の入力先（最後に `activate` したコントローラ）
    private weak var active: (any ShellSession)?
    /// dispatch の間だけ、周辺テキストと選択を読む先
    private var currentAccess: ClientAccess?
    private var timers: [UInt32: DispatchWorkItem] = [:]
    /// shell-core が最後に知らせた入力メニューの中身（`ShowMenu`）
    private(set) var menuModel: Ikari_Shell_V1_MenuModel?

    init(
        config: Config, engine: any EngineHost, downNotifier: DownNotifierBox = DownNotifierBox(),
        queue: DispatchQueue = .main, isSecure: @escaping () -> Bool = { false }
    ) {
        self.config = config
        self.engine = engine
        self.downNotifier = downNotifier
        self.queue = queue
        self.isSecure = isSecure
    }

    // MARK: 入力先（IMK）

    /// `activateServer`。前の入力先が残っていれば先に終える（その表示はその入力先に確定する）
    func activate(_ session: any ShellSession) {
        if let previous = active, previous !== session { deactivate(previous) }
        active = session
        var activate = Ikari_Shell_V1_Activate()
        activate.bundleID = session.textClient?.bundleIdentifier() ?? ""
        activate.secure = isSecure()
        var event = ShellEvent()
        event.activate = activate
        dispatch(event, session: session)
    }

    /// `deactivateServer`。今の入力先でなければ何もしない（すでに別の入力先に移った）
    func deactivate(_ session: any ShellSession) {
        guard active === session else { return }
        var event = ShellEvent()
        event.deactivate = Ikari_Shell_V1_Empty()
        dispatch(event, session: session)
        active = nil
    }

    /// `inputControllerWillClose`。今の入力先なら終える
    func close(_ session: any ShellSession) {
        deactivate(session)
    }

    /// キー。戻り値はキーを食べたか（false ならアプリに渡す）
    func key(_ input: Ikari_Shell_V1_KeyInput, session: any ShellSession) -> Bool {
        // `activateServer` を経ずに打鍵が来た（殻が起動し直した直後、`deactivateServer` のあと。#315）
        if active !== session { activate(session) }
        var event = ShellEvent()
        event.key = input
        return dispatch(event, session: session)
    }

    /// IMK の `commitComposition`
    func commitComposition(session: any ShellSession) {
        guard active === session else { return }
        var event = ShellEvent()
        event.commitComposition = Ikari_Shell_V1_Empty()
        dispatch(event, session: session)
    }

    /// macOS からのモードの通知（`kTextServiceInputModePropertyTag`）
    func inputModeChanged(_ modeID: String, session: any ShellSession) {
        var changed = Ikari_Shell_V1_InputModeChanged()
        changed.modeID = modeID
        changed.secure = isSecure()
        var event = ShellEvent()
        event.inputModeChanged = changed
        dispatch(event, session: active === session ? session : nil)
    }

    func menuCommand(_ command: MenuCommand) {
        var event = ShellEvent()
        event.menuCommand = command
        dispatch(event, session: active)
    }

    /// 入力メニューの中身。まだ知らせを受けていなければ、設定を読ませて受け取る（エンジンは呼ばない）
    func currentMenu() -> Ikari_Shell_V1_MenuModel? {
        if menuModel == nil {
            var event = ShellEvent()
            event.settingsFileChanged = Ikari_Shell_V1_Empty()
            dispatch(event, session: nil)
        }
        return menuModel
    }

    // MARK: 設定アプリ（受信のスレッドから呼んでよい。`queue` に移して渡す）

    func bridgeFrame(_ frame: Data, connection: UInt64) {
        queue.async { [self] in
            var bridgeFrame = Ikari_Shell_V1_BridgeFrame()
            bridgeFrame.conn = connection
            bridgeFrame.frame = frame
            var event = ShellEvent()
            event.bridgeFrame = bridgeFrame
            dispatch(event, session: active)
        }
    }

    func bridgeClosed(connection: UInt64) {
        queue.async { [self] in
            var closed = Ikari_Shell_V1_BridgeClosed()
            closed.conn = connection
            var event = ShellEvent()
            event.bridgeClosed = closed
            dispatch(event, session: active)
        }
    }

    // MARK: 1 本の口

    /// イベントを 1 つ渡し、動作を実行する。戻り値はキーを食べたか
    @discardableResult
    func dispatch(_ event: ShellEvent, session: (any ShellSession)?) -> Bool {
        dispatch(event, session: session, panelReports: 0)
    }

    private func dispatch(_ event: ShellEvent, session: (any ShellSession)?, panelReports: Int) -> Bool {
        guard let shell = shellOrCreate() else { return false }
        let access = session?.textClient.map(ClientAccess.init(client:))
        currentAccess = access
        let (status, actions) = shell.dispatch(event)
        currentAccess = nil
        switch status {
        case .ok:
            break
        case .panic:
            // 一度 panic した shell は使わない。次の呼び出しで作り直す（この回のキーはアプリへ）
            AppContext.log.fault("shell-core panicked; recreating the shell")
            self.shell = nil
            downNotifier.set(nil)
            cancelAllTimers()
            return false
        case .busy:
            AppContext.log.error("shell dispatch re-entered (busy)")
            return false
        default:
            AppContext.log.error("shell dispatch failed: \(String(describing: status), privacy: .public)")
        }
        return perform(actions, session: session, access: access, panelReports: panelReports)
    }

    private func shellOrCreate() -> ShellABI? {
        if let shell { return shell }
        let host = ShellHost(
            surrounding: { [weak self] in self?.currentAccess?.surroundingText() },
            selection: { [weak self] in self?.currentAccess?.selectedText() },
            engineConnect: { [engine] in engine.connect() },
            engineRequest: { [engine] in engine.request($0, $1, timeoutMs: $2) },
            enginePost: { [engine] in engine.post($0, $1) },
            engineClose: { [engine] in engine.close($0) },
            enginePid: { [engine] in engine.pid($0) })
        do {
            let created = try ShellABI(
                shellVersion: config.shellVersion, settingsPath: config.settingsPath,
                resourcesDir: config.resourcesDir, host: host)
            shell = created
            downNotifier.set(created.downNotifier)
            return created
        } catch {
            // 作れない（FFI がない、資源が読めない）ときは、キーをすべてアプリに渡す
            AppContext.log.error("cannot create the shell")
            return nil
        }
    }

    // MARK: 動作の実行

    /// 動作を順に実行する。`passThrough` を含むときは、ほかの動作を実行したうえで、キーをアプリに渡す（戻り値 false）
    private func perform(
        _ actions: [ShellAction], session: (any ShellSession)?, access: ClientAccess?, panelReports: Int
    ) -> Bool {
        var handled = true
        // 候補ウィンドウを出した・動かしたら、置いた上下（と新しいアンカーの出どころ）を shell-core に返す
        var panelReport: Ikari_Shell_V1_PanelShown?
        for action in actions {
            switch action.action {
            case .passThrough:
                handled = false
            case .consume, .prefetch, nil:
                break
            case .setMarkedText(let marked):
                access?.setMarkedText(marked)
            case .commit(let commit):
                access?.insert(commit.text)
            case .showCandidates(let model):
                guard let access, let ui else { break }
                let (side, source) = MainActor.assumeIsolated { ui.showCandidates(model, access: access) }
                var report = Ikari_Shell_V1_PanelShown()
                report.side = side
                report.source = source
                panelReport = report
            case .moveSelection(let model):
                guard let ui, let side = MainActor.assumeIsolated({ ui.moveSelection(model) }) else { break }
                var report = panelReport ?? Ikari_Shell_V1_PanelShown()
                report.side = side
                panelReport = report
            case .hideCandidates:
                if let ui { MainActor.assumeIsolated { ui.hideCandidates() } }
                // 閉じたときの上下は shell-core が自分で戻す
                panelReport = nil
            case .restore(let restore):
                access?.restore(restore.text, replacing: restore.replacing)
            case .deleteText:
                // 契約の時点では shell-core がまだ返さない。実行（insertText の 1 回）はレーン C（ADR 0050 §10）
                break
            case .resolveStrayMarkedText:
                // 何も入力していないのに変換中の文字列が残っているときは、確定するだけにしてキーをアプリへ渡さない
                // （チャット欄で Enter が送信になるのを防ぐ）。なければキーはアプリへ
                if let access, let text = access.strayMarkedText() {
                    access.commitStray(text)
                } else {
                    handled = false
                }
            case .showMenu(let menu):
                menuModel = menu
            case .scheduleTimer(let timer):
                schedule(timer.id, afterMs: timer.afterMs)
            case .cancelTimer(let timer):
                timers.removeValue(forKey: timer.id)?.cancel()
            case .bridgeReply(let reply):
                bridge?.send(reply.frame, to: reply.conn)
            case .bridgeClose(let close):
                bridge?.close(close.conn)
            case .openSettings(let open):
                if let ui { MainActor.assumeIsolated { ui.openSettings(open.tab) } }
            case .selectMode(let select):
                session?.selectMode(select.modeID)
            }
        }
        if let panelReport, panelReports < Self.maxPanelReports {
            var event = ShellEvent()
            event.panelShown = panelReport
            // この知らせの返す動作（見回りで届いた結果など）も実行する。キーを食べたかには数えない
            dispatch(event, session: session, panelReports: panelReports + 1)
        }
        // 入力先が取れないキーは、アプリに渡す（今の `run(actions)` と同じ）
        return handled && (session == nil || access != nil)
    }

    // MARK: タイマー

    private func schedule(_ id: UInt32, afterMs: UInt32) {
        timers.removeValue(forKey: id)?.cancel()
        let item = DispatchWorkItem { [weak self] in
            guard let self else { return }
            timers.removeValue(forKey: id)
            var timer = Ikari_Shell_V1_TimerFired()
            timer.id = id
            var event = ShellEvent()
            event.timer = timer
            dispatch(event, session: active)
        }
        timers[id] = item
        queue.asyncAfter(deadline: .now() + .milliseconds(Int(afterMs)), execute: item)
    }

    private func cancelAllTimers() {
        timers.values.forEach { $0.cancel() }
        timers.removeAll()
    }
}

/// XPC の接続の表を `EngineHost` にする
struct XPCEngineHost: EngineHost {
    let connections: XPCEngineConnections

    func connect() -> UInt64 { connections.connect() }
    func request(_ connection: UInt64, _ bytes: Data, timeoutMs: UInt32) -> ShellTransportResult {
        switch connections.request(connection, bytes, timeoutMs: timeoutMs) {
        case .success(let reply): .ok(reply)
        case .failure(.timeout): .timeout
        case .failure(.unavailable): .unavailable
        }
    }
    func post(_ connection: UInt64, _ bytes: Data) { connections.post(connection, bytes) }
    func close(_ connection: UInt64) { connections.close(connection) }
    func pid(_ connection: UInt64) -> Int32 { connections.pid(connection) }
}
