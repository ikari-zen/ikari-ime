// IMK のコントローラ（ADR 0004 §6.2）。イベントを `KeyInput` に直して状態機械に渡し、返ってきた `KeyAction` を実行するだけにする。
// 入力セッション（クライアントのアプリごと）に1つ作られる。
import AppKit
import Carbon
import IkariShellCore
import InputMethodKit

@objc(IkariInputController)
final class IkariInputController: IMKInputController {
    private let context = AppContext.shared
    private var machine: InputStateMachine?
    /// このコントローラのクライアントの Bundle ID（設定の変更で共有の可否を引き直すため）
    private var clientBundleID = ""
    /// このコントローラに適用済みの設定。`SettingsStore` の版が進んだら、idle のときに反映する（ADR 0005 §7.2）
    private var applied: SettingsStore.Snapshot
    /// 状態機械に渡した絵文字の表の版（`AppContext.shortcodeTableSnapshot`）
    private var appliedEmojiVersion = 0
    /// 候補ウィンドウのアンカー。フォーカスしている文節の先頭（1 文節なら変換文字列の先頭）に置き、
    /// フォーカスが動くか `hideCandidates` するまで動かさない（ADR 0006 §8.4）
    private var panelAnchor: PanelAnchor?
    /// `panelAnchor` を取った文節の先頭（変換文字列の中の UTF-16 の相対位置）
    private var panelAnchorOffset = 0
    /// 今の入力モード。`setValue(_:forTag:client:)` で macOS が教えてくる（ADR 0026 §5.2）。モードが替わってもコントローラ、
    /// セッション ID、`BeginSession` は変えない（確定履歴を切らさない）
    private var inputMode = InputMode.japanese
    /// 英語のモードの文脈のたまり（ADR 0026 §5.5）。区切りで `CommitText`（読みは空）としてエンジンへ送る
    private var english = EnglishMode()
    /// `mode.*` で `selectMode` を頼んだ時刻（`ProcessInfo.systemUptime`）。macOS からの通知が来たとき、近ければキー割り当てによる切り替えとして診断に残す
    private var keySwitchRequestedAt: TimeInterval?
    /// 頼んでから通知までの許容（これを超えたら別の経路の切り替え）
    private static let keySwitchWindow: TimeInterval = 1.0

    override init!(server: IMKServer!, delegate: Any!, client inputClient: Any!) {
        let snapshot = AppContext.shared.store.snapshot
        applied = snapshot
        super.init(server: server, delegate: delegate, client: inputClient)
        let bundleID = (inputClient as? IMKTextInput)?.bundleIdentifier() ?? ""
        clientBundleID = bundleID
        let settings = snapshot.settings
        machine = InputStateMachine(
            engine: context.engine,
            composer: makeComposer(method: settings.inputMethod, romajiTable: context.customRomajiTable),
            scripts: makeScriptConverter(),
            keymap: snapshot.keymap,
            settings: settings,
            session: SessionInfo(
                sessionID: context.nextSessionID(),
                clientBundleID: bundleID,
                secure: IsSecureEventInputEnabled(),
                // 除外のアプリは、ほかのアプリと文脈を貸し借りしない（ADR 0015 D6。判定は殻の中だけ）
                shareContext: settings.sharesContext(bundleID: bundleID))
        )
    }

    // MARK: IMK

    override func activateServer(_ sender: Any!) {
        super.activateServer(sender)
        // 手で編集した settings.tsv に気づく（stat 1 回）。idle のときに反映する
        context.store.reloadIfFileChanged()
        syncSettings()
        machine?.setSecure(IsSecureEventInputEnabled())
        machine?.activate()
        // 絵文字のショートコードの表を、別のキューで取り直す（返事が来るまでは空で、ショートコードは入らない）
        context.refreshEmojiTable()
        // エンジンの起動（XPC サービス、辞書、LM の読み込み）の前倒しと、メニューの状態の取り直し。主スレッドは止めない
        context.refreshEngineInfo()
    }

    override func deactivateServer(_ sender: Any!) {
        run(machine?.commitComposition(reason: .deactivate) ?? [])
        flushEnglishShortcode("deactivateServer", end: .deactivate)
        hidePanel()
        // `endSession` の前に送る（終わったセッションには入らない）
        flushEnglishPool()
        machine?.deactivate()
        super.deactivateServer(sender)
    }

    override func commitComposition(_ sender: Any!) {
        run(machine?.commitComposition() ?? [])
        flushEnglishShortcode("commitComposition", end: .commitComposition)
    }

    override func inputControllerWillClose() {
        flushEnglishShortcode("inputControllerWillClose", end: .deactivate)
        hidePanel()
        flushEnglishPool()
        machine?.deactivate()
    }

    override func handle(_ event: NSEvent!, client sender: Any!) -> Bool {
        guard let event, event.type == .keyDown, let machine else { return false }
        syncSettings()
        syncEmojiTable(machine)
        // JIS の英数・かなキーは、macOS が入力ソースを切り替えたあと、同じキーが切り替え先にも届く。
        // そのキーの文字は見えない空白なので、アプリに返すと空白が入る。どのモードでも受け止めるだけで何もしない
        // （ADR 0005 §15.2-5、ADR 0026 §12 の T1）
        // その前にショートコードの表示があれば確定する（ADR 0026 §5.2）
        if EnglishMode.absorbsKeyCode(event.keyCode) {
            flushEnglishShortcode("eisuKana", end: .modeSwitch)
            return true
        }
        // 打鍵のたびに secure を見る。パスワード欄などでは文脈を送らず、学習もさせない（NFR-18）
        let secure = IsSecureEventInputEnabled()
        machine.setSecure(secure)
        // activateServer を経ずに打鍵が来たとき（殻のプロセスが起動し直した直後、deactivateServer のあと）も、
        // エンジンにセッションを知らせてから送る。始めていれば何もしない（#315）
        machine.activateIfNeeded()

        let modifiers = Self.modifiers(of: event.modifierFlags)
        // Ctrl / Option / Command を押しているとき、`characters` は制御文字などになる。キー割り当ては押したキーで引く
        let usesChord = !modifiers.intersection([.control, .option, .command]).isEmpty
        let characters = (usesChord ? event.charactersIgnoringModifiers : event.characters) ?? ""
        if inputMode == .english {
            return handleEnglish(
                keyCode: event.keyCode, characters: characters, modifiers: modifiers, secure: secure, isRepeat: event.isARepeat)
        }
        let input = KeyCodes.keyInput(
            keyCode: event.keyCode, characters: characters, modifiers: modifiers, inputMethod: applied.settings.inputMethod)
        // キー割り当ての `mode.*` は、変換より先に見る（合成中なら今の表示のまま確定してから切り替える）
        if let target = EnglishMode.modeSwitchTarget(for: input, keymap: applied.keymap, current: inputMode) {
            selectMode(target)
            return true
        }

        // 周辺テキストは Convert の直前だけ、選択範囲は `reconvert` のキーのときだけ読まれる（autoclosure）
        let actions = machine.handle(
            input,
            surrounding: currentClient()?.surroundingText() ?? .unavailable,
            selection: currentClient()?.selectedText(),
            isRepeat: event.isARepeat)
        return run(actions)
    }

    // MARK: 英語のモード（ADR 0026 §5.2。エンジンは同期で呼ばない。送るのは片道の `CommitText` だけ）

    /// 英語のモードの打鍵。順は、secure → `mode.*` → ショートコードの口（B48）→ 文脈のたまりに足して素通し。
    /// 英語のモードは、かな入力の設定でも `.jis` にしない（文字で読む）
    private func handleEnglish(
        keyCode: UInt16, characters: String, modifiers: KeyModifiers, secure: Bool, isRepeat: Bool
    ) -> Bool {
        let input = KeyCodes.keyInput(keyCode: keyCode, characters: characters, modifiers: modifiers, inputMethod: .romaji)
        switch EnglishMode.classify(input: input, keymap: applied.keymap, secure: secure, current: inputMode) {
        case .passThroughSecure:
            // ショートコードの表示があれば、そのまま確定する（ADR 0026 §5.2）。たまりは送らずに捨てる
            // （secure の前後を 1 件につなげない）。何も記録しない
            flushEnglishShortcode("secure", end: .commitComposition)
            english.discard()
            return false
        case .switchMode(let target):
            if target != inputMode { selectMode(target) }
            return true
        case .text:
            // ショートコードの口（ADR 0027 §3）。関係ないキーは、たまりに足して素通し
            let environment = EnglishShortcodeEnvironment(
                settings: applied.settings, clientBundleID: clientBundleID, secure: false, isRepeat: isRepeat)
            let outcome = english.handleText(input, environment: environment)
            apply(outcome)
            if let request = outcome.lookup { requestPostalLookup(request) }
            machine?.recordEnglishEpisode(english.takeFinishedDiag())
            return !outcome.passKey
        }
    }

    /// 郵便番号の住所を、主スレッドの外で 1 回だけ問い合わせる（ADR 0032 §9.1）。返事が来たとき状態が同じなら候補に足す。
    /// コードも住所もログに書かない
    private func requestPostalLookup(_ request: PostalLookupRequest) {
        let engine = context.engine
        // コントローラは主スレッドでしか触らない。待ちの上限（`postalLookupTimeout`）の間だけ強く持つ
        nonisolated(unsafe) let target = self
        DispatchQueue.global(qos: .userInitiated).async {
            let addresses = fetchPostalAddresses(engine: engine, code: request.code)
            DispatchQueue.main.async {
                // パスワード欄に移っていたら何もしない
                guard !IsSecureEventInputEnabled(),
                    let outcome = target.english.postalLookupArrived(request, addresses: addresses)
                else { return }
                target.apply(outcome)
            }
        }
    }

    /// ショートコードの操作を実行し、取り出したたまりを確定履歴に送る
    private func apply(_ outcome: EnglishTextOutcome) {
        if !outcome.actions.isEmpty { run(outcome.actions) }
        outcome.flushed.forEach { send($0) }
    }

    /// ショートコードの表示があれば、今の表示のまま確定する（モードの切り替え、secure、`deactivateServer`、クリック、英数・かなキー）
    private func flushEnglishShortcode(_ reason: StaticString, end: DiagEnd) {
        // 郵便番号をためていれば、打った文字のまま確定する（ショートコードとは同時に状態に入らない）
        if let outcome = english.flushPostal() { apply(outcome) }
        if let outcome = english.flushShortcode(end: end) {
            // 調査用（#291）。文字は書かず、何が状態を終わらせたかだけ残す
            AppContext.log.info("english shortcode flushed by \(reason, privacy: .public)")
            apply(outcome)
            machine?.recordEnglishEpisode(english.takeFinishedDiag())
        }
    }

    /// たまりを確定履歴に送る（`reading` は空）
    private func send(_ pooled: String?) {
        guard let pooled else { return }
        machine?.recordEnglishText(pooled)
    }

    private func flushEnglishPool() {
        send(english.flush())
    }

    /// キー割り当てでモードを替える。今のモードの表示を確定してから、macOS に選んでもらう。
    /// 実際の切り替えは `setValue(_:forTag:client:)` で知る（選べなかったときは、今のモードのまま）
    private func selectMode(_ target: InputMode) {
        keySwitchRequestedAt = ProcessInfo.processInfo.systemUptime
        leaveCurrentMode()
        client()?.selectMode(target.modeID)
    }

    /// モードを替える前の後始末。合成中なら今の表示のまま確定し（変換しない）、候補ウィンドウを閉じ、
    /// 英語の文脈のたまりを送る（どちらの向きでも。ADR 0026 §5.2）。何度呼んでもよい
    private func leaveCurrentMode() {
        run(machine?.commitComposition(reason: .modeSwitch) ?? [])
        flushEnglishShortcode("leaveMode", end: .modeSwitch)
        hidePanel()
        flushEnglishPool()
    }

    /// macOS からのモードの通知（`kTextServiceInputModePropertyTag`。値はモードの辞書のキー。ADR 0026 §12）
    override func setValue(_ value: Any!, forTag tag: Int, client sender: Any!) {
        if tag == kTextServiceInputModePropertyTag, let modeID = value as? String,
            let newMode = InputMode(modeID: modeID), newMode != inputMode
        {
            leaveCurrentMode()
            let previous = inputMode
            inputMode = newMode
            recordModeSwitch(from: previous, to: newMode)
        }
        super.setValue(value, forTag: tag, client: sender)
    }

    /// `sh_mode`（ADR 0034 §4.2）。合成中に替えたときの `sh_episode`（`mode_switch`）は `leaveCurrentMode` で先に出ている
    private func recordModeSwitch(from: InputMode, to: InputMode) {
        let requested = keySwitchRequestedAt
        keySwitchRequestedAt = nil
        let viaKey = requested.map { ProcessInfo.processInfo.systemUptime - $0 < Self.keySwitchWindow } ?? false
        machine?.setSecure(IsSecureEventInputEnabled())
        machine?.recordModeSwitch(from: from, to: to, viaKey: viaKey)
    }

    /// 絵文字の表の版が進んでいたら、状態機械に渡す（ロックを取るだけ。エンジンは呼ばない）
    private func syncEmojiTable(_ machine: InputStateMachine) {
        let latest = context.shortcodeTableSnapshot()
        guard latest.version != appliedEmojiVersion else { return }
        appliedEmojiVersion = latest.version
        machine.setShortcodeTable(latest.table)
        english.setShortcodeTable(latest.table)
    }

    // MARK: 設定の反映

    /// 設定の版が進んでいたら、idle のときだけ状態機械に反映する。合成中や変換中は、次に idle になるまで待つ（表示中の文字列を変えない）。
    /// `input.method` が変わったときだけ、新しい合成器を作って渡す。セッションの ID は変えず、`BeginSession` も送り直さない
    private func syncSettings() {
        let latest = context.store.snapshot
        guard latest.version != applied.version, let machine else { return }
        // 共有の可否は入力の途中でも変えてよい（表示中の文字列に関係ない）。idle になるまで待たない
        machine.setShareContext(latest.settings.sharesContext(bundleID: clientBundleID))
        let composer: (any Composing)? =
            latest.settings.inputMethod == applied.settings.inputMethod
            ? nil : makeComposer(method: latest.settings.inputMethod, romajiTable: context.customRomajiTable)
        if machine.reconfigure(settings: latest.settings, keymap: latest.keymap, composer: composer) {
            applied = latest
        }
    }

    // MARK: 入力メニュー（ADR 0005 §5）

    /// 入力メニューを開くたびに呼ばれる（打鍵ごとに呼ばれることもある）。状態が前回と同じなら前回のメニューを返す。エンジンは呼ばない
    override func menu() -> NSMenu! {
        let state = context.menuState()
        // IMK は `menu()` を主スレッドで呼ぶ。NSMenu は Sendable ではないので、結果は変数で受ける
        nonisolated(unsafe) var menu: NSMenu?
        MainActor.assumeIsolated { menu = SharedMenu.cache.menu(for: state) }
        return menu
    }

    /// メニューの項目を選んだとき（IMK は項目の `action` をコントローラに送る。`target` は使わない）
    @objc func ikariMenuCommand(_ sender: Any?) {
        guard let command = MenuActions.command(from: sender) else { return }
        context.perform(command)
    }

    /// 「環境設定…」の項目（`action = showPreferences:`）
    override func showPreferences(_ sender: Any!) {
        context.perform(.openSettings(tab: nil))
    }

    // MARK: 実行

    /// 操作を順に実行する。`.passThrough` を含むときは、ほかの操作を実行したうえで、イベントをアプリに返す（戻り値 false）
    @discardableResult
    private func run(_ actions: [KeyAction]) -> Bool {
        guard let access = currentClient() else { return false }
        var handled = true
        for action in actions {
            switch action {
            case .passThrough:
                handled = false
            case .consume, .prefetch:
                break
            case .setMarkedText(let marked):
                access.setMarkedText(marked)
                // アンカーは、フォーカスしている文節の先頭。フォーカスが動いたときだけ取り直す（打鍵とページ送りでは動かさない）
                let offset = anchorOffset(of: marked)
                if offset != panelAnchorOffset {
                    panelAnchor = nil
                    panelAnchorOffset = offset
                }
            case .commit(let text):
                access.insert(text)
            case .showCandidates(let model):
                showPanel(model, access: access)
            case .moveSelection(let model):
                let side = MainActor.assumeIsolated { SharedPanel.shared.panel.update(model: model) }
                if let side { machine?.setPanelSide(side) }
            case .hideCandidates:
                hidePanel()
            case .restore(let text, let replacing):
                // 元の文字列を、変換中の範囲を置き換える 1 回の insertText で書き戻す（ADR 0006 §16.3）
                access.restore(text, replacing: replacing)
            case .resolveStrayMarkedText:
                // 何も入力していないのに変換中の文字列が残っているときは、確定するだけにしてキーをアプリへ渡さない
                // （チャット欄で Enter が送信になるのを防ぐ）。なければキーはアプリへ
                if let text = access.strayMarkedText() {
                    access.commitStray(text)
                } else {
                    handled = false
                }
            }
        }
        return handled
    }

    private func currentClient() -> ClientAccess? {
        guard let client = client() else { return nil }
        return ClientAccess(client: client)
    }

    // MARK: 候補ウィンドウ

    private func showPanel(_ model: CandidatePanelModel, access: ClientAccess) {
        let anchor = panelAnchor ?? resolveAnchor(access)
        panelAnchor = anchor
        // 診断ログ（合成の最初の 1 回だけ覚える）
        machine?.notePanelShown(source: anchor.source)
        english.notePanelShown(source: anchor.source)
        let visibleFrame = Self.screen(containing: anchor.rect).visibleFrame
        let level = access.windowLevel
        let side = MainActor.assumeIsolated {
            SharedPanel.shared.panel.show(model: model, anchor: anchor.rect, visibleFrame: visibleFrame, level: level)
        }
        machine?.setPanelSide(side)
    }

    private func hidePanel() {
        panelAnchor = nil
        panelAnchorOffset = 0
        machine?.setPanelSide(.below)
        MainActor.assumeIsolated { SharedPanel.shared.panel.hide() }
    }

    /// キャレットが取れないときのフォールバック: アクセシビリティ（権限があるときだけ）、同じアプリの最後の正常値、マウスのあるスクリーンの中央下
    private func resolveAnchor(_ access: ClientAccess) -> PanelAnchor {
        let bundleID = access.bundleIdentifier
        let client = access.caretRect(characterOffset: panelAnchorOffset)
        let accessibility = (client == nil) ? AccessibilityAnchor.focusedElementBottomLeft() : nil
        let anchor = IkariShellCore.resolveAnchor(
            client: client,
            accessibility: accessibility,
            lastKnown: AnchorMemory.shared.lastKnown(for: bundleID),
            mouseScreenVisibleFrame: Self.mouseScreen().visibleFrame)
        if anchor.source == .client { AnchorMemory.shared.remember(anchor.rect, for: bundleID) }
        return anchor
    }

    private static func screen(containing rect: CGRect) -> NSScreen {
        let point = CGPoint(x: rect.minX, y: rect.midY)
        return NSScreen.screens.first { $0.frame.contains(point) } ?? mouseScreen()
    }

    private static func mouseScreen() -> NSScreen {
        let mouse = NSEvent.mouseLocation
        return NSScreen.screens.first { $0.frame.contains(mouse) } ?? NSScreen.main ?? NSScreen.screens[0]
    }

    private static func modifiers(of flags: NSEvent.ModifierFlags) -> KeyModifiers {
        KeyCodes.modifiers(fromFlagBits: flags.rawValue)
    }
}

/// 入力メニューのキャッシュはプロセスで 1 つ（コントローラごとに作らない。設定は全体で 1 つなので、どのコントローラでも同じ中身になる）
@MainActor
private enum SharedMenu {
    static let cache = MenuCache(
        selectors: MenuSelectors(
            command: #selector(IkariInputController.ikariMenuCommand(_:)),
            preferences: #selector(IMKInputController.showPreferences(_:))))
}

/// 候補ウィンドウはプロセスで1つを共有する（入力セッションごとに作らない）。
@MainActor
private final class SharedPanel {
    static let shared = SharedPanel()
    let panel = CandidatePanel()
}

/// 同じアプリで最後に取れた正常なキャレットの矩形（フォールバック②）。
private final class AnchorMemory: @unchecked Sendable {
    static let shared = AnchorMemory()
    private let lock = NSLock()
    private var rects: [String: CGRect] = [:]

    func lastKnown(for bundleID: String) -> CGRect? {
        lock.lock()
        defer { lock.unlock() }
        return rects[bundleID]
    }

    func remember(_ rect: CGRect, for bundleID: String) {
        lock.lock()
        defer { lock.unlock() }
        rects[bundleID] = rect
    }
}
