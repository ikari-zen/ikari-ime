// プロセス全体で共有するもの（設定、エンジンのクライアント、入力メニューが使う状態）。
// 設定は `SettingsStore` が持ち、変わったら各コントローラが idle のときに反映する（ADR 0005 §7.2）。
// `engine.*` が変わったら、エンジンへ `Shutdown{SETTINGS_CHANGED}` を送って起こし直す（§7.3）。
import Foundation
import IkariShellCore
import IkariXPC
import os

final class AppContext: @unchecked Sendable {
    static let shared = AppContext()

    /// ログには文字列（読み、候補、文脈、辞書の語、分野の名前）を決して書かない（NFR-19）。種類と件数だけ
    static let log = Logger(subsystem: IkariIdentifiers.shell, category: "shell")

    /// 署名の方式を build.sh が Info.plist に入れる（lane X）。無ければ既定の `team`
    static let signingModeInfoKey = "IkariSigningMode"
    static let userDirectoryName = "ikari-ime"
    static let settingsFileName = "settings.tsv"
    static let romajiFileName = "romaji.tsv"
    /// 絵文字のショートコードの表（約 2,000 組）の取得の待ち。主スレッドの外で待つ（ADR 0027 §8.2）
    static let emojiTableTimeout: Duration = .seconds(2)
    /// 殻とエンジンのバージョン照合のための `Hello` の待ち（エンジンの起動を前倒しするのが主な目的）
    static let helloTimeout: Duration = .seconds(2)
    /// LM の読み込み中は、読み込みが終わるまで `Hello` で状態を取り直す間隔と上限（メニューの「読み込み中」を直すため）
    static let lmPollInterval: TimeInterval = 2
    static let lmPollLimit = 30

    let store: SettingsStore
    let engine: any EngineClient
    let admin: EngineAdmin
    /// 設定アプリとのブリッジ（ADR 0005 §6.3）。`startBridge()` で待ち始める
    let bridge: BridgeServer
    let userDirectory: URL
    let signing: SigningMode
    /// ユーザーのローマ字表（あれば同梱の表を置き換える）
    let customRomajiTable: Data?

    private let lock = NSLock()
    private var lastSessionID: UInt64 = 0
    private var lmState: EngineLmState?
    private var profile: [ProfileDomain]?
    /// 状態の取り直しが進行中か。エンジンを起こし直すたびに `pollGeneration` を進め、古い取り直しを打ち切る
    private var refreshing = false
    private var pollGeneration = 0
    /// 絵文字のショートコードの表（ADR 0027）。返事が来るまで・失敗したときは空で、ショートコードは入らない。
    /// 表を差し替えるたびに `emojiVersion` を進める（コントローラは版が変わったときだけ状態機械に渡す）
    private var emojiTable = ShortcodeTable.empty
    private var emojiDataLabel = ""
    private var emojiVersion = 0
    private var emojiFetching = false

    /// エンジンの起こし直しと、状態（LM、分野）の取得は、主スレッドを止めないよう、この直列のキューで行う
    private let engineQueue = DispatchQueue(label: "\(Identity.productID).engine-admin", qos: .utility)
    /// 絵文字の表の取得は、LM の取り直しを待たせないよう別のキューで行う
    private let emojiQueue = DispatchQueue(label: "\(Identity.productID).emoji-table", qos: .utility)

    private init() {
        let support = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
            ?? URL(fileURLWithPath: NSHomeDirectory()).appendingPathComponent("Library/Application Support")
        let directory = support.appendingPathComponent(Self.userDirectoryName, isDirectory: true)
        userDirectory = directory

        store = SettingsStore(
            fileURL: directory.appendingPathComponent(Self.settingsFileName),
            loadKeymap: { KeymapLoader.load(settings: $0) })
        customRomajiTable = try? Data(contentsOf: directory.appendingPathComponent(Self.romajiFileName))

        signing = (Bundle.main.object(forInfoDictionaryKey: Self.signingModeInfoKey) as? String)
            .flatMap(SigningMode.init(rawValue:)) ?? .team
        let client = XPCEngineClient(signing: signing)
        engine = client
        admin = EngineAdmin(client: client)
        let shellVersion = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? ""
        bridge = BridgeServer(
            directory: directory, store: store, engine: client, signing: signing, shellVersion: shellVersion)

        store.onChange = { [unowned self] old, new in settingsDidChange(from: old, to: new) }
    }

    func nextSessionID() -> UInt64 {
        lock.lock()
        defer { lock.unlock() }
        lastSessionID &+= 1
        return lastSessionID
    }

    // MARK: 設定の変更

    private func settingsDidChange(from old: Settings, to new: Settings) {
        AppContext.log.info("settings changed (engine keys: \(new.engineSettingsDiffer(from: old)))")
        if new.engineSettingsDiffer(from: old) { restartEngine(lmEnabled: new.engineLM) }
        // つながっている設定アプリは取り直す（メニューや手の編集で変わったとき）
        bridge.notifySettingsChanged()
    }

    /// `engine.*` が変わったとき: エンジンに保存して終了してもらい（`SETTINGS_CHANGED`。落ちた回数に数えない）、
    /// 続けて `Hello` で起こし直す。セッションは `XPCEngineClient` の再同期で送り直される。
    /// 起こし直しの間（辞書を開く間）は、殻のフォールバックの候補が出ることがある（ADR 0005 §7.3）
    private func restartEngine(lmEnabled: Bool) {
        lock.lock()
        lmState = lmEnabled ? .loading : .disabled
        pollGeneration += 1
        refreshing = true
        let generation = pollGeneration
        lock.unlock()
        engineQueue.async { [self] in
            engine.shutdown(reason: .settingsChanged)
            refreshEngineInfoNow(attempt: 0, generation: generation)
        }
    }

    // MARK: 絵文字のショートコードの表（ADR 0027 §9.4。打鍵の経路でエンジンを呼ばない）

    /// 今の表と版。ロックを取るだけで、エンジンは呼ばない
    func shortcodeTableSnapshot() -> (version: Int, table: ShortcodeTable) {
        lock.lock()
        defer { lock.unlock() }
        return (emojiVersion, emojiTable)
    }

    /// `activateServer`（セッションの開始）から呼ぶ。別のキューで表を取り、データの版（`data_label`）が変わっていたら差し替える。
    /// 取れなかった（エンジンが落ちている、時間切れ、壊れた返事）ときは、今の表のまま（変換を止めない）
    func refreshEmojiTable() {
        lock.lock()
        let busy = emojiFetching
        emojiFetching = true
        lock.unlock()
        guard !busy else { return }
        emojiQueue.async { [self] in
            let info = try? engine.emojiTable(timeout: Self.emojiTableTimeout)
            lock.lock()
            defer { lock.unlock() }
            emojiFetching = false
            guard let info else {
                AppContext.log.info("emoji table: not available")
                return
            }
            guard info.dataLabel != emojiDataLabel || emojiTable.isEmpty else { return }
            emojiTable = ShortcodeTable(entries: info.entries)
            emojiDataLabel = info.dataLabel
            emojiVersion += 1
            AppContext.log.info("emoji table: \(info.entries.count) entries")
        }
    }

    // MARK: 入力メニューの状態（キャッシュ。`menu()` の中でエンジンを呼ばない）

    func menuState() -> InputMenuState {
        let settings = store.settings
        lock.lock()
        defer { lock.unlock() }
        return InputMenuState(settings: settings, lmEnabled: settings.engineLM, lmState: lmState, profile: profile)
    }

    private func setLmState(_ state: EngineLmState?) {
        lock.lock()
        lmState = state
        lock.unlock()
    }

    private func setProfile(_ domains: [ProfileDomain]) {
        lock.lock()
        profile = domains
        lock.unlock()
    }

    /// `activateServer` から呼ぶ。エンジンの起動の前倒し（`Hello`）と、LM の状態・分野の取り直しを、別のキューで行う
    func refreshEngineInfo() {
        lock.lock()
        let busy = refreshing
        refreshing = true
        let generation = pollGeneration
        lock.unlock()
        guard !busy else { return }
        engineQueue.async { [self] in refreshEngineInfoNow(attempt: 0, generation: generation) }
    }

    /// `engineQueue` の上で呼ぶ。LM が読み込み中の間は、間隔をあけて状態を取り直す（キューを塞がないよう、次の回は予約する）。
    /// `generation` が古くなったら（エンジンを起こし直した）、その回で打ち切る
    private func refreshEngineInfoNow(attempt: Int, generation: Int) {
        lock.lock()
        let current = generation == pollGeneration
        lock.unlock()
        guard current else { return }

        let version = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? ""
        if let hello = try? engine.hello(shellVersion: version, timeout: Self.helloTimeout) {
            setLmState(hello.lm)
            if attempt == 0, let domains = try? admin.profile() { setProfile(domains) }
            if hello.lm == .loading, attempt < Self.lmPollLimit {
                engineQueue.asyncAfter(deadline: .now() + Self.lmPollInterval) { [self] in
                    refreshEngineInfoNow(attempt: attempt + 1, generation: generation)
                }
                return
            }
        }
        lock.lock()
        if generation == pollGeneration { refreshing = false }
        lock.unlock()
    }

    // MARK: 入力メニューの動作

    func perform(_ command: MenuCommand) {
        switch command {
        case .selectKeymapPreset(let preset):
            write([("keymap.preset", preset.rawValue)])
        case .selectInputMethod(let method):
            write([("input.method", method.rawValue)])
        case .setLM(let enabled):
            write([("engine.lm", enabled ? "on" : "off")])
        case .setProfileDomain(let id, let included):
            // エンジンを呼ぶので別のキューで。最新の状態を取り直してから 1 つだけ変える
            engineQueue.async { [self] in
                if let domains = try? admin.setWeight(included ? .often : .off, forDomain: id) {
                    setProfile(domains)
                } else {
                    AppContext.log.error("profile update failed")
                }
            }
        case .openSettings(let tab):
            openSettings(tab: tab)
        }
    }

    private func write(_ entries: [(key: String, value: String)]) {
        do {
            try store.update(set: entries, reset: [])
        } catch {
            AppContext.log.error("settings update failed")
        }
    }

    /// 設定アプリとのブリッジを始める（起動時に 1 回）
    func startBridge() {
        bridge.start()
    }

    func openSettings(tab: SettingsTab?) {
        // すでに動いていてつながっているなら、開くタブをブリッジで伝える（起動の引数は動作中のアプリには届かない）
        if let tab { bridge.sendOpenTab(tab) }
        Task { @MainActor in
            SettingsLauncher.open(tab: tab)
        }
    }
}
