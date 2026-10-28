// 入力の状態機械（ADR 0004 §6.3 の表）。
// IMK にも AppKit にも依存しない。コントローラは、イベントを `KeyInput` に直して `handle` に渡し、
// 返ってきた `KeyAction` を順に実行する（`.passThrough` を含むときは、ほかの操作を実行したうえでイベントを返さない）。
import Foundation

/// ローマ字またはかな入力の合成器（Rust の `ikari-input` を FFI で呼ぶ実装は IkariIME 側に置く。テストは替え玉）。
public protocol Composing: AnyObject {
    var kana: String { get }
    var pending: String { get }
    func pushCharacter(_ scalar: Unicode.Scalar)
    func pushJis(key: UInt8, shift: Bool)
    @discardableResult func backspace() -> Bool
    func flush()
    func clear()
}

/// 字種変換（F6〜F10）。FFI の `ikari_convert_script` を呼ぶ実装は IkariIME 側。
public protocol ScriptConverting: Sendable {
    func convert(_ text: String, to script: KeymapAction) -> String
}

extension SurroundingText {
    /// エンジンに渡す長さに切る。左は末尾、右は先頭の `maxCharacters` 文字。
    public func truncated() -> SurroundingText {
        SurroundingText(
            left: String(left.suffix(Self.maxCharacters)),
            right: String(right.prefix(Self.maxCharacters)),
            available: available
        )
    }
}

public final class InputStateMachine {
    /// エンジンが受け付ける読みの長さの上限（超えたら殻のフォールバックの候補を出す）
    public static let maxReadingCharacters = 256

    public private(set) var state: InputState = .idle

    private let engine: any EngineClient
    private var composer: any Composing
    private let scripts: any ScriptConverting
    private var keymap: Keymap
    private var settings: Settings
    private var session: SessionInfo
    /// エンジンへ `BeginSession` を送ってから `EndSession` を送るまでの間か（`activateIfNeeded`。#315）
    private var sessionBegun = false

    /// 直近に取った周辺テキスト（Prefetch はこれを使い、打鍵のたびにクライアントへ問い合わせない）。secure のときは使わない
    private var cachedContext: SurroundingText?
    private var contextRev: UInt64 = 0
    /// いまの合成の、文字を打った回数（左文脈の読み直しの回数を絞るのに使う。#1）
    private var composingKeystrokes = 0
    private var lastPrefetchedReading: String?

    /// converting に入る前の状態。読みに戻るときの行き先
    private var origin: ReadingOrigin = .composing
    /// asciiRun の先頭に残したかな（合成中のかなを確定させてから英字を積む）
    private var asciiPrefix = ""
    /// いまの変換の文節の扱い（ADR 0006）。ページは `ConvertingState.segments` が持つ
    private var mode = SegmentMode()
    /// 再変換の BackSpace で読みの合成に戻ったあとも、Esc で元の文字列に戻せるように覚えておく（ADR 0006 §9.6）
    private var restoreTarget: (original: String, replacing: TextRange)?
    /// 絵文字のショートコード（ADR 0027）。状態の本体は `state`（`.shortcode` / `.shortcodeChoice`）と同じものを持つ
    private var shortcode: ShortcodeMachine
    /// ショートコードの打ち直し（`retype`）で、切った状態のキーを流し直している間。新たに入らない
    private var replayingShortcode = false
    /// いま処理しているキーがキーリピートか。リピートでは `:` からショートコードに入らない（§3.2）
    private var inputIsRepeat = false

    /// 診断ログ用の時計（単調時計のナノ秒）。テストが差し替える
    var diagClock: () -> UInt64 = { DispatchTime.now().uptimeNanoseconds }
    /// いまの合成 1 回の数（ADR 0034 §4.2）。idle の間は nil。secure のセッションでは作らない（NFR-18）
    private var episode: DiagEpisode?
    /// 次に始まる合成の `start`
    private var nextEpisodeStart: DiagStart = .typing
    /// いま処理しているキー（`end` の判定に使う）
    private var currentKey: KeyInput.Key?
    /// 外から確定させられた理由（`commitComposition(reason:)`）。キーによる確定のときは nil
    private var externalEnd: DiagEnd?

    private enum ReadingOrigin: Equatable {
        case composing
        case ascii
    }

    /// 変換 1 回ぶんの、エンジンとの付き合い方
    private struct SegmentMode {
        /// 区切りを返すエンジンの変換（`ConvertReply.layout` あり、または再変換）。Shift+←/→ が効く
        var canSegment = false
        /// ページ送りと確定を文節の API（`segmentPage` / `commitSegments`）で行う。
        /// 1 文節のまま区切りを変えていない変換は、今までどおり `page` / `commit`（学習も今と同じ）
        var useSegmentAPI = false
        /// 区切り直しをした
        var layoutChanged = false
    }

    private static let jisDigit1: UInt8 = 0
    private static let jisDigit9: UInt8 = 8
    private static let commandModifiers: KeyModifiers = [.control, .option, .command]
    private static let fallbackScripts: [KeymapAction] = [.toHiragana, .toFullwidthKatakana, .toHalfwidthKatakana]

    public init(
        engine: any EngineClient,
        composer: any Composing,
        scripts: any ScriptConverting,
        keymap: Keymap,
        settings: Settings,
        session: SessionInfo
    ) {
        self.engine = engine
        self.composer = composer
        self.scripts = scripts
        self.keymap = keymap
        self.settings = settings
        self.session = session
        self.shortcode = ShortcodeMachine(table: .empty, mode: Self.shortcodeMode(for: settings.inputMethod))
    }

    private static func shortcodeMode(for method: InputMethodKind) -> ShortcodeMode {
        method == .kana ? .kanaJis : .romaji
    }

    /// ショートコードの名前の表を渡す（セッションの開始時とデータの版が変わったとき。コントローラが非同期に取って渡す）。
    /// 表が空のうちは、ショートコードに入らない。打っている途中に差し替えても、今の候補は変えない
    public func setShortcodeTable(_ table: ShortcodeTable) {
        shortcode.table = table
    }

    // MARK: - セッション

    /// 入力モードが有効になったとき。エンジンへセッションの開始を知らせる
    public func activate() {
        beginSession()
    }

    /// 入力モードが無効になるとき。先に `commitComposition()` を実行すること
    public func deactivate() {
        engine.endSession(sessionID: session.sessionID)
        sessionBegun = false
    }

    /// 打鍵の前に呼ぶ。セッションを始めていない（`activateServer` を経ずに打鍵が来た、`deactivate` のあとに打鍵が来た）
    /// ときだけ `BeginSession` を送る。送らないと、エンジンは要求に `UNKNOWN_SESSION` を返し続ける（#315）
    public func activateIfNeeded() {
        guard !sessionBegun else { return }
        beginSession()
    }

    private func beginSession() {
        engine.beginSession(session)
        sessionBegun = true
    }

    /// 打鍵のたびに `IsSecureEventInputEnabled()` を見て渡す。変わったときだけエンジンへ知らせる。
    /// secure の間は、文脈を取らず、先行計算も確定の通知も送らない（NFR-18）
    public func setSecure(_ secure: Bool) {
        guard secure != session.secure else { return }
        session.secure = secure
        if secure {
            cachedContext = nil
            episode = nil
        }
        beginSession()
    }

    /// 「文脈を共有しないアプリ」の設定が変わったとき、接続中のセッションへ反映する（ADR 0015 D6）。変わったときだけエンジンへ知らせる。
    /// 同じ ID で `BeginSession` を送り直すと、エンジンは確定履歴を残したまま共有の可否だけを更新する（secure が同じとき。契約は変えない）。
    /// secure の間はエンジンが貸し借りしないので、ここでは secure を見て値を変えない
    public func setShareContext(_ share: Bool) {
        guard share != session.shareContext else { return }
        session.shareContext = share
        beginSession()
    }

    /// 英語のモードの文脈のたまり（`EnglishContextPool`。ADR 0026 §5.5）を、確定履歴へ送る。
    /// `reading` は空（読みなし。履歴の読みを使う部品は飛ばす）。空の文字列と secure のときは送らない。返事は待たない
    public func recordEnglishText(_ surface: String) {
        guard !surface.isEmpty, !session.secure else { return }
        engine.commitText(sessionID: session.sessionID, reading: "", surface: surface)
    }

    /// IMK の `commitComposition`。表示中の文字列をそのまま確定する
    /// `reason` は診断ログの `end`（モードの切り替え、`deactivateServer` などの呼び分け）
    public func commitComposition(reason: DiagEnd = .commitComposition) -> [KeyAction] {
        guard state != .idle else { return [] }
        externalEnd = reason
        defer { externalEnd = nil }
        return commitDisplay()
    }

    // MARK: - 設定の反映

    /// 設定の変更を、入力中でないときに反映する（ADR 0005 §7.2）。
    ///
    /// - `state == .idle` のときだけ反映して true を返す。合成中や変換中は何もせず false を返す（呼び出す側は次に idle になるまで待つ）。
    /// - `composer` は `input.method` が変わったときだけ渡す（nil なら今の合成器のまま）。
    /// - セッションの ID は変えない。`BeginSession` も送り直さない（確定履歴を消さない）。表示中の文字列は変えない。
    @discardableResult
    public func reconfigure(settings: Settings, keymap: Keymap, composer: (any Composing)?) -> Bool {
        guard state == .idle else { return false }
        self.settings = settings
        self.keymap = keymap
        if let composer { self.composer = composer }
        shortcode.mode = Self.shortcodeMode(for: settings.inputMethod)
        return true
    }

    // MARK: - 候補ウィンドウの上下

    /// 候補ウィンドウがアンカーの上下どちらにあるか。殻が `layoutPanel` の結果を通知する（表示前と非表示のときは `below`）。
    /// 上にあるときは 1 位が下に描かれるので、↑↓ の向きを見た目に合わせる（ADR 0014 の決定の節「上に出たときの ↑↓」）
    public private(set) var panelSide: PanelSide = .below

    public func setPanelSide(_ side: PanelSide) {
        panelSide = side
    }

    /// ↑↓ の押下で、次／前の向きを入れ替えるか。候補ウィンドウが上にあり、押したのが ↑ か ↓ のときだけ。
    /// 割り当てを変えて ↑↓ が既定（↓ = 次、↑ = 前）でなくなったときと、別のキーに置いた割り当ては、向きを変えない
    private func reversesArrow(_ input: KeyInput) -> Bool {
        guard panelSide == .above, input.key == .up || input.key == .down else { return false }
        return keymap.action(for: KeyChord(key: "down")) == .candidateNext
            && keymap.action(for: KeyChord(key: "up")) == .candidatePrevious
    }

    // MARK: - キー入力

    /// キーを処理して、実行すべき操作を返す。`surrounding` は `Convert` の直前に取った周辺テキスト
    ///
    /// `selection` は再変換（`reconvert` のキー）のときだけ評価する選択範囲（`SelectedText`。ADR 0006 §9.2）
    ///
    /// `isRepeat` はキーリピート（`NSEvent.isARepeat`）。リピートでは `:` からショートコードに入らない（ADR 0027 §3.2）
    public func handle(
        _ input: KeyInput,
        surrounding: @autoclosure () -> SurroundingText,
        selection: @autoclosure () -> SelectedText? = nil,
        isRepeat: Bool = false
    ) -> [KeyAction] {
        inputIsRepeat = isRepeat
        // secure（パスワード欄など）の間は、機能として OFF にする（ADR 0026 D5、§5.4）。
        // 合成中・変換中なら今の表示のまま確定してから（secure なので履歴・学習には送らない）、キーをアプリへ素通しにする
        if session.secure {
            episode = nil
            guard state != .idle else { return [.passThrough] }
            return commitDisplay() + [.passThrough]
        }
        let keyTime = diagClock()
        currentKey = input.key
        countKey(input)
        let actions = dispatch(input, fetch: surrounding, selection: selection)
        beginEpisodeIfNeeded(at: keyTime)
        return actions
    }

    // MARK: 診断ログ（ADR 0034 §4.2）。数えるのは整数の加算だけ。送るのは idle に戻ったとき 1 回

    private func countKey(_ input: KeyInput) {
        guard episode != nil else { return }
        episode?.keys += 1
        if input.key == .backspace { episode?.bs += 1 }
    }

    /// 状態が idle から外れたとき、合成 1 回の記録を始める（キー 1 つぶんは数えた扱い）
    private func beginEpisodeIfNeeded(at time: UInt64) {
        guard episode == nil, state != .idle, !session.secure else { return }
        let mode: DiagMode
        switch state {
        case .shortcode, .shortcodeChoice: mode = .shortcodeJa
        default: mode = settings.inputMethod == .kana ? .kana : .romaji
        }
        var started = DiagEpisode(mode: mode, start: nextEpisodeStart, startedAt: time)
        started.keys = 1
        episode = started
        nextEpisodeStart = .typing
    }

    /// `idle` に戻る直前に呼ぶ（`state` はまだ前の状態）。1 回の合成につき 1 行。secure は送らない
    private func emitEpisode(_ end: DiagEnd, converting: ConvertingState? = nil) {
        guard let finished = episode else { return }
        episode = nil
        guard !session.secure else { return }
        engine.diag(
            finished.event(
                sessionID: session.sessionID, end: end, prev: end.isExternal ? previousKind() : nil,
                commit: commitInfo(for: end, converting: converting ?? convertingState), now: diagClock()))
    }

    /// 候補ウィンドウを出した（コントローラが、位置を取ったあとに呼ぶ）。合成の最初の 1 回だけ覚える。secure では何もしない
    public func notePanelShown(source: AnchorSource) {
        episode?.notePanel(source)
    }

    /// 英語のモードで終わった合成を送る（`EnglishMode.takeFinishedDiag`）。secure のときは捨てる
    public func recordEnglishEpisode(_ finished: DiagFinishedEpisode?) {
        guard let finished, !session.secure else { return }
        engine.diag(finished.event(sessionID: session.sessionID))
    }

    /// 英語・日本語のモードが替わった（`sh_mode`）。`viaKey` はキー割り当て（`mode.*`）で替えたとき。secure のときは送らない
    public func recordModeSwitch(from: InputMode, to: InputMode, viaKey: Bool) {
        guard !session.secure, from != to else { return }
        engine.diag(DiagModeSwitch.event(sessionID: session.sessionID, from: from, to: to, via: viaKey ? .key : .os))
    }

    private var convertingState: ConvertingState? {
        if case .converting(let current) = state { current } else { nil }
    }

    private func previousKind() -> DiagPrev {
        switch state {
        case .converting(let current):
            current.segments.count > 1 || mode.useSegmentAPI ? .segments : .converting
        case .asciiRun: .asciiRun
        case .shortcode: .shortcode
        case .shortcodeChoice: .shortcodeChoice
        case .idle, .composing: .composing
        }
    }

    private func commitInfo(for end: DiagEnd, converting current: ConvertingState?) -> DiagCommitInfo? {
        if end == .shortcodePick { return DiagCommitInfo(pick: nil, pickKind: shortcode.lastPickKind, segs: nil, segChanged: nil) }
        guard let current, end == .commitCand || end == .commitSegments else { return nil }
        var info = DiagCommitInfo(pick: nil, pickKind: nil, segs: current.segments.count, segChanged: nil)
        if end == .commitCand {
            info.pick = current.candidateOffset + current.selected
            info.pickKind = pickKind(of: current)
        } else {
            info.segChanged = current.segments.filter { $0.literal != nil || $0.candidateIndex != 0 }.count
        }
        return info
    }

    private func pickKind(of current: ConvertingState) -> DiagPickKind {
        if current.isFallback { return .fallback }
        guard current.candidates.indices.contains(current.selected) else { return .dict }
        switch current.candidates[current.selected].kind {
        case .unspecified, .dictionary: return .dict
        case .hiragana: return .hiragana
        case .katakana: return .katakana
        case .rewriter: return .rewriter
        case .literal: return .literal
        }
    }

    private func dispatch(
        _ input: KeyInput, fetch: () -> SurroundingText, selection: () -> SelectedText?
    ) -> [KeyAction] {
        if isShortcodeState {
            // ショートコードの間は、キー割り当てより先に機械へ渡す（Tab、↓、Space、数字が候補の操作になる）。
            // secure になっていたら、今の表示のまま確定してから、キーを通常の経路で処理する
            guard session.secure else { return handleShortcode(input, fetch: fetch, selection: selection) }
            return commitShortcodeDisplay() + dispatch(input, fetch: fetch, selection: selection)
        }
        if let action = boundAction(for: input) {
            if action.isScriptConversion {
                return state == .idle ? [.passThrough] : applyScript(action)
            }
            if action == .reconvert {
                return state == .idle ? startReconversion(selection: selection, fetch: fetch) : [.consume]
            }
            // 候補と文節の操作は変換中だけ効く。ほかの状態では、割り当てがないときと同じに扱う（idle の矢印はアプリへ渡す）
            if case .converting = state {
                switch action {
                case .candidateNext, .candidatePrevious:
                    episode?.candMoves += 1
                    let forward = (action == .candidateNext) != reversesArrow(input)
                    return forward ? nextCandidate() : previousCandidate()
                default: return segmentAction(action)
                }
            }
        }
        switch state {
        case .idle: return handleIdle(input, fetch: fetch)
        case .composing: return handleComposing(input, fetch: fetch)
        case .converting: return handleConverting(input, fetch: fetch)
        case .asciiRun: return handleAsciiRun(input, fetch: fetch)
        case .shortcode, .shortcodeChoice: return handleShortcode(input, fetch: fetch, selection: selection)
        }
    }

    // MARK: idle

    private func handleIdle(_ input: KeyInput, fetch: () -> SurroundingText) -> [KeyAction] {
        if hasCommandModifiers(input) { return [.passThrough] }
        if let entered = enterShortcode(input) { return entered }
        switch input.key {
        case .character(let text):
            guard isPrintable(text) else { return [.passThrough] }
            if isShiftedLetter(text, input) { return startAsciiRun(with: text, fetch: fetch) }
            composer.clear()
            feed(text)
            return composingStarted(fetch: fetch)
        case .jis(let key):
            composer.clear()
            composer.pushJis(key: key, shift: input.modifiers.contains(.shift))
            return composingStarted(fetch: fetch)
        case .enter, .escape:
            // 取り残された変換中の文字列があれば、確定するだけにしてキーを消費する（ADR 0006 §16.3）
            return [.resolveStrayMarkedText]
        case .space, .backspace, .left, .right, .up, .down, .function, .other:
            return [.passThrough]
        }
    }

    /// 合成器が何も受け取らなかった（記号など）ときは、アプリにそのまま渡す
    private func composingStarted(fetch: () -> SurroundingText) -> [KeyAction] {
        guard !(composer.kana.isEmpty && composer.pending.isEmpty) else { return [.passThrough] }
        refreshContext(fetch: fetch)
        composingKeystrokes = 1
        lastPrefetchedReading = nil
        return refreshComposing()
    }

    /// 合成の最初の数打鍵のあいだ、左文脈を読み直す（#1）。
    /// 変換中に次の文字を打つと、確定の文字をアプリに入れる前に 1 打鍵目の文脈を読むので、その文脈には直前の確定が入っていない。
    /// 2 打鍵目以降に読み直し、文脈が変わったら（番号が進んだら）、その文脈で先行計算を送り直す
    private func refreshContextOnEarlyKeystroke(fetch: () -> SurroundingText) {
        composingKeystrokes += 1
        guard composingKeystrokes <= Self.contextRefreshKeystrokes, !session.secure else { return }
        let before = contextRev
        refreshContext(fetch: fetch)
        if contextRev != before { lastPrefetchedReading = nil }
    }

    // MARK: composing

    private func handleComposing(_ input: KeyInput, fetch: () -> SurroundingText) -> [KeyAction] {
        if hasCommandModifiers(input) { return commitDisplay() + [.passThrough] }
        switch input.key {
        case .character(let text):
            guard isPrintable(text) else { return [.consume] }
            if isShiftedLetter(text, input) { return startAsciiRun(with: text, fetch: fetch) }
            feed(text)
            refreshContextOnEarlyKeystroke(fetch: fetch)
            return refreshComposing()
        case .jis(let key):
            composer.pushJis(key: key, shift: input.modifiers.contains(.shift))
            refreshContextOnEarlyKeystroke(fetch: fetch)
            return refreshComposing()
        case .space:
            return startConversion(reading: readingToConvert(), origin: .composing, fetch: fetch)
        case .enter:
            return commitDisplay()
        case .escape:
            let displayedLength = (composer.kana + composer.pending).utf16.count
            composer.clear()
            // 再変換の BackSpace で戻った合成は、合成を消すと元の文字列が消えるので、元に戻して終える（ADR 0006 §9.6）
            if let target = restoreTarget {
                resetComposition(ending: .escClear)
                return [Self.restoreAction(target.original, replacing: target.replacing, displayedLength: displayedLength)]
            }
            resetComposition(ending: .escClear)
            return [clearMark()]
        case .backspace:
            if !composer.backspace() {
                composer.clear()
            }
            return refreshComposing()
        case .left, .right, .up, .down, .function, .other:
            return [.consume]
        }
    }

    /// 変換する読み（FR-07）。かなの部分だけで、未確定のローマ字（「へんk」の k）は含めない。
    /// ただし末尾の未確定が「n」1文字のときだけ、変換を始める直前に「ん」を確定する（composer の flush と同じ規則。「へんかn」→「へんかん」）。
    /// 「nn」「n'」は composer がすでに「ん」にしている
    private func readingToConvert() -> String {
        if composer.pending == Self.trailingNPending { composer.flush() }
        return composer.kana
    }

    private static let trailingNPending = "n"

    /// 合成の最初の何打鍵目まで、左文脈を読み直すか（1 打鍵目を含む）。
    /// 暫定。#1。ログの miss=context で効果を測って見直す。値は計測で決める
    static let contextRefreshKeystrokes = 3

    /// かなと pending から表示を作り、読み（かな）が変わったら Prefetch を投げる。空なら idle に戻す
    private func refreshComposing() -> [KeyAction] {
        let kana = composer.kana
        let pending = composer.pending
        guard !(kana.isEmpty && pending.isEmpty) else {
            resetComposition(ending: currentKey == .escape ? .escClear : .bsEmpty)
            return [clearMark()]
        }
        state = .composing(kana: kana, pending: pending)
        prefetchIfChanged(reading: kana)
        return [.setMarkedText(MarkedText(text: kana + pending, highlightedLength: kana.utf16.count))]
    }

    // MARK: asciiRun

    private func startAsciiRun(with text: String, fetch: () -> SurroundingText) -> [KeyAction] {
        if state == .idle {
            composer.clear()
            refreshContext(fetch: fetch)
        } else {
            // 合成中のかなは確定させてから英字を積む（n は ん になる）
            composer.flush()
        }
        asciiPrefix = composer.kana
        lastPrefetchedReading = nil
        return setAsciiRun(asciiPrefix + text)
    }

    private func setAsciiRun(_ text: String) -> [KeyAction] {
        state = .asciiRun(text: text)
        return [.setMarkedText(MarkedText(text: text, highlightedLength: text.utf16.count))]
    }

    private func handleAsciiRun(_ input: KeyInput, fetch: () -> SurroundingText) -> [KeyAction] {
        guard case .asciiRun(let text) = state else { return [.consume] }
        if hasCommandModifiers(input) { return commitDisplay() + [.passThrough] }
        switch input.key {
        case .character(let typed):
            guard isPrintable(typed) else { return [.consume] }
            return setAsciiRun(text + typed)
        case .space:
            return startConversion(reading: text, origin: .ascii, fetch: fetch)
        case .enter:
            return commitDisplay()
        case .escape:
            // 英字の部分を捨てて、残したかなの合成に戻る（かなが無ければ idle）
            return leaveAsciiRun()
        case .backspace:
            if text.count > asciiPrefix.count {
                let shortened = String(text.dropLast())
                return shortened.count > asciiPrefix.count ? setAsciiRun(shortened) : leaveAsciiRun()
            }
            return leaveAsciiRun()
        case .jis, .left, .right, .up, .down, .function, .other:
            return [.consume]
        }
    }

    private func leaveAsciiRun() -> [KeyAction] {
        asciiPrefix = ""
        return refreshComposing()
    }

    // MARK: converting

    private func handleConverting(_ input: KeyInput, fetch: () -> SurroundingText) -> [KeyAction] {
        guard case .converting(let current) = state else { return [.consume] }
        if hasCommandModifiers(input) { return commitDisplay() + [.passThrough] }
        switch input.key {
        case .space:
            // 最初の Space は `startConversion` が数える。ここへ来るのは 2 回目以降（候補の移動）
            episode?.spaces += 1
            episode?.candMoves += 1
            return input.modifiers.contains(.shift) ? previousCandidate() : nextCandidate()
        case .enter:
            return commitDisplay()
        case .escape:
            if case .reconversion(let original, let replacing) = current.origin {
                return restoreOriginal(original, replacing: replacing, from: current)
            }
            episode?.escBack += 1
            return returnToReading(from: current)
        case .backspace:
            return returnToReading(from: current)
        case .character(let text):
            guard isPrintable(text) else { return [.consume] }
            if let number = Int(text), text.count == 1, (1...9).contains(number) {
                return selectNumber(number)
            }
            return commitDisplay() + continueAfterCommit(input, fetch: fetch)
        case .jis(let key):
            if (Self.jisDigit1...Self.jisDigit9).contains(key) && !input.modifiers.contains(.shift) {
                return selectNumber(Int(key - Self.jisDigit1) + 1)
            }
            return commitDisplay() + continueAfterCommit(input, fetch: fetch)
        case .left, .right, .up, .down, .function, .other:
            // 矢印は `boundAction` が先に引く。ここへ来るのは割り当てが外れているとき（受け止めて何もしない）
            return [.consume]
        }
    }

    /// 確定のあと、同じキーから新しい合成を始める
    private func continueAfterCommit(_ input: KeyInput, fetch: () -> SurroundingText) -> [KeyAction] {
        handleIdle(input, fetch: fetch)
    }

    /// 数字キー。1 文節なら、その候補を選んで全体を確定する。2 文節以上なら、フォーカスしている文節の候補を選ぶだけにして、
    /// 次の文節へフォーカスを進める（最後の文節では動かさない。ADR 0006 §16.1-3）
    private func selectNumber(_ number: Int) -> [KeyAction] {
        // 一覧が見えていないので、番号で選ばず、一覧を出す（#69）
        if let revealed = revealCandidatesIfHidden() { return revealed }
        guard case .converting(let current) = state, number - 1 < current.visibleCount(initial: settings.pageSize) else {
            return [.consume]
        }
        episode?.digitPicks += 1
        guard current.segments.count > 1 else { return commitConverting(selecting: number - 1) }

        let selection = select(number - 1)
        if current.focused + 1 < current.segments.count, let moved = moveFocus(by: 1) { return moved }
        return selection
    }

    /// 一覧を隠している変換中なら、一覧を出す（選択は動かさない）。隠していなければ nil
    private func revealCandidatesIfHidden() -> [KeyAction]? {
        guard case .converting(var current) = state, current.candidatesHidden else { return nil }
        current.candidatesHidden = false
        state = .converting(current)
        return [.showCandidates(panelModel(for: current))]
    }

    private func nextCandidate() -> [KeyAction] {
        if let revealed = revealCandidatesIfHidden() { return revealed }
        guard case .converting(let current) = state else { return [.consume] }
        let segment = current.segments[current.focused]
        let visible = current.visibleCount(initial: settings.pageSize)
        if current.selected + 1 < visible {
            return select(current.selected + 1)
        }
        // 見せていない候補がページに残っているときは、ページ全体に広げて次の候補を選ぶ（ADR 0014 §2.3 (c)）
        if visible < current.candidates.count {
            var expanded = current
            expanded.segments[expanded.focused].expanded = true
            state = .converting(expanded)
            return select(current.selected + 1)
        }
        // ページの最後の候補
        let nextIndex = current.pageIndex + 1
        if nextIndex < segment.pages.count {
            episode?.pages += 1
            return show(pageAt: nextIndex, selected: 0)
        }
        if current.hasNext && !current.isFallback {
            guard let page = fetchPage(for: current, segment: current.focused, index: nextIndex) else { return [.consume] }
            var latest = current
            latest.segments[latest.focused].pages.append(page)
            state = .converting(latest)
            episode?.pages += 1
            return show(pageAt: latest.segments[latest.focused].pages.count - 1, selected: 0)
        }
        if settings.pagingWrap {
            episode?.pages += 1
            return show(pageAt: 0, selected: 0)
        }
        return [.consume]
    }

    private func previousCandidate() -> [KeyAction] {
        if let revealed = revealCandidatesIfHidden() { return revealed }
        guard case .converting(let current) = state else { return [.consume] }
        if current.selected > 0 {
            return select(current.selected - 1)
        }
        if current.pageIndex > 0 {
            let previous = current.segments[current.focused].pages[current.pageIndex - 1]
            return show(pageAt: current.pageIndex - 1, selected: previous.candidates.count - 1)
        }
        return [.consume]
    }

    /// 文節 `segment` の `index` ページ目をエンジンに作らせる。失敗、空の返事は nil（いまの表示を変えない）
    private func fetchPage(for current: ConvertingState, segment: Int, index: Int) -> ConversionPage? {
        let timeout = EngineTimeouts.convertTimeout(readingCharacterCount: current.segments[segment].reading.count)
        if mode.useSegmentAPI {
            guard
                let page = try? engine.segmentPage(
                    conversionID: current.conversionID, layoutRev: current.layoutRev, segment: segment, index: index,
                    left: choices(of: current.segments[..<segment]), timeout: timeout),
                !page.candidates.isEmpty
            else { return nil }
            return Self.conversionPage(from: page, conversionID: current.conversionID)
        }
        guard let page = try? engine.page(conversionID: current.conversionID, index: index, timeout: timeout),
            !page.candidates.isEmpty
        else { return nil }
        return page
    }

    private static func conversionPage(from page: SegmentPage, conversionID: UInt64) -> ConversionPage {
        ConversionPage(
            conversionID: conversionID, pageIndex: page.pageIndex, candidates: page.candidates, hasNext: page.hasNext,
            lmApplied: page.lmApplied, engineMicros: page.engineMicros)
    }

    /// フォーカスしている文節の `index` 番目の候補を選ぶ（同じページの中）
    private func select(_ index: Int) -> [KeyAction] {
        guard case .converting(var current) = state else { return [.consume] }
        let hadLiteral = current.segments[current.focused].literal != nil
        current.segments[current.focused].selected = index
        current.segments[current.focused].literal = nil
        current.syncFocused()
        state = .converting(current)
        let model = panelModel(for: current)
        // 字種変換で候補ウィンドウを閉じていたときは、開き直す
        return [.setMarkedText(markedText(for: current)), hadLiteral ? .showCandidates(model) : .moveSelection(model)]
    }

    /// フォーカスしている文節の、取得済みの `position` 番目のページに移る
    private func show(pageAt position: Int, selected: Int) -> [KeyAction] {
        guard case .converting(var current) = state else { return [.consume] }
        let samePage = position == current.pageIndex && current.segments[current.focused].literal == nil
        current.segments[current.focused].pageIndex = position
        current.segments[current.focused].selected = selected
        current.segments[current.focused].literal = nil
        current.syncFocused()
        state = .converting(current)
        let model = panelModel(for: current)
        return [.setMarkedText(markedText(for: current)), samePage ? .moveSelection(model) : .showCandidates(model)]
    }

    private func returnToReading(from current: ConvertingState) -> [KeyAction] {
        if case .reconversion(let original, let replacing) = current.origin {
            // 推定した読みの合成に戻る。この合成の中の Esc は、元の文字列に戻す（ADR 0006 §9.6）。
            // かな入力は読みを合成器に入れられない。何も変えずに、Esc で元に戻せるまま残す
            guard seedComposer(with: current.reading) else { return [.consume] }
            engine.cancel(conversionID: current.conversionID)
            mode = SegmentMode()
            restoreTarget = (original, replacing)
            origin = .composing
            return [.hideCandidates] + refreshComposing()
        }
        if !current.isFallback { engine.cancel(conversionID: current.conversionID) }
        mode = SegmentMode()
        switch origin {
        case .composing:
            return [.hideCandidates] + refreshComposing()
        case .ascii:
            return [.hideCandidates] + setAsciiRun(current.reading)
        }
    }

    /// 合成器に読みを入れ直す。romaji の合成器はかなをそのまま受け取る。入らなかった（かな入力）ら false
    private func seedComposer(with reading: String) -> Bool {
        composer.clear()
        feed(reading)
        if composer.kana == reading { return true }
        composer.clear()
        return false
    }

    /// 元の文字列を書き戻して終える（再変換の Esc。ADR 0006 §9.5、§16.3）。変換中ならエンジンに Cancel を送る
    private func restoreOriginal(_ original: String, replacing: TextRange, from current: ConvertingState) -> [KeyAction] {
        if !current.isFallback { engine.cancel(conversionID: current.conversionID) }
        let displayedLength = markedText(for: current).text.utf16.count
        resetComposition(ending: .escClear)
        return [.hideCandidates, Self.restoreAction(original, replacing: replacing, displayedLength: displayedLength)]
    }

    /// 変換中の文字列は、再変換の選択範囲の先頭から始まる。長さは、いま表示している文字列の UTF-16 の長さ
    private static func restoreAction(_ original: String, replacing: TextRange, displayedLength: Int) -> KeyAction {
        .restore(original, replacing: TextRange(location: replacing.location, length: displayedLength))
    }

    // MARK: 文節の操作（ADR 0006 §8.2）

    private func segmentAction(_ action: KeymapAction) -> [KeyAction] {
        let actions = performSegmentAction(action)
        if actions != [.consume] {
            // 文節の操作は候補ウィンドウを出す。隠していた印は下ろす
            if case .converting(var current) = state, current.candidatesHidden {
                current.candidatesHidden = false
                state = .converting(current)
            }
            switch action {
            case .segmentFocusLeft, .segmentFocusRight: episode?.segFocus += 1
            case .segmentShrink, .segmentExpand: episode?.segResize += 1
            default: break
            }
        }
        return actions
    }

    private func performSegmentAction(_ action: KeymapAction) -> [KeyAction] {
        guard case .converting(let current) = state, !current.isFallback, mode.canSegment else { return [.consume] }
        if let layout = current.wholeLayout { return enterSegmentMode(by: action, from: current, layout: layout) }
        switch action {
        case .segmentFocusLeft: return moveFocus(by: -1) ?? [.consume]
        case .segmentFocusRight: return moveFocus(by: 1) ?? [.consume]
        case .segmentShrink: return resizeFocused(delta: -1)
        case .segmentExpand: return resizeFocused(delta: 1)
        default: return [.consume]
        }
    }

    /// 全体の候補を出している間に文節の操作のキーを受けたとき、文節のモードに入ってそのキーを実行する（ADR 0006 §16.6）。
    /// - →: フォーカスは先頭の文節にあるものとして、2 番目の文節へ移す。←: 先頭の文節にフォーカスを置くだけ。
    ///   どちらも表示している表記を変えない。いま選んでいる全体の候補が最初の区切りの表記と違うときは、各文節の
    ///   最初のページの候補の組み合わせで作れるならその選択を引き継ぎ、作れなければ入らずに何もしない
    /// - Shift+←/→: 先頭の文節の区切り直し。区切りの作り直しで選択はすべて 0 番に戻る（§4.2）ので、全体の候補の選択は捨てる
    /// 失敗（時間切れ、端など）は、全体の候補のまま何も変えない
    private func enterSegmentMode(by action: KeymapAction, from current: ConvertingState, layout: SegmentLayoutInfo) -> [KeyAction] {
        let previousMode = mode
        mode.useSegmentAPI = true
        func restore() -> [KeyAction] {
            mode = previousMode
            state = .converting(current)
            return [.consume]
        }
        let carriesSelection = action == .segmentFocusLeft || action == .segmentFocusRight
        guard let next = segmentModeState(from: current, layout: layout, carrying: carriesSelection) else { return restore() }
        state = .converting(next)
        switch action {
        case .segmentFocusLeft:
            return [.setMarkedText(markedText(for: next)), .showCandidates(panelModel(for: next))]
        case .segmentFocusRight:
            return moveFocus(by: 1) ?? restore()
        case .segmentShrink:
            let actions = resizeFocused(delta: -1)
            return actions == [.consume] ? restore() : actions
        case .segmentExpand:
            let actions = resizeFocused(delta: 1)
            return actions == [.consume] ? restore() : actions
        default:
            return restore()
        }
    }

    /// 全体の候補の状態から、文節のモードの状態を作る。`carrying` が真なら、いま表示している表記を保つ。
    /// 偽（区切り直し）なら、すべての文節の選択は 0 番。`mode.useSegmentAPI` は呼ぶ側が立てておく（文節のページの取得に使う）
    private func segmentModeState(from current: ConvertingState, layout: SegmentLayoutInfo, carrying: Bool) -> ConvertingState? {
        guard let built = Self.convertingSegments(from: layout, expectedReading: current.reading) else { return nil }
        var next = current
        next.segments = built.segments
        next.focused = built.focused
        next.conversionID = layout.conversionID
        next.layoutRev = layout.layoutRev
        next.wholeLayout = nil
        next.pageIndex = 0
        next.selected = 0
        next.candidateOffset = 0
        if carrying, let shown = current.segments.first.map(displayedSurface),
            shown != layout.segments.map(\.surface).joined()
        {
            // 作れない表記（全体の候補だけにある表記）でも文節のモードには入る。選択は 0 番のまま（区切りの表記になる）
            if let picks = segmentChoices(forming: shown, in: &next) {
                for (index, pick) in picks.enumerated() { next.segments[index].selected = pick }
            }
        }
        next.syncFocused()
        return next
    }

    /// 各文節の最初のページ（取っていなければここで取る）の候補から 1 つずつ選んでつなぐと `surface` になる組み合わせを探す。
    /// 候補の順位が小さいものを優先する（辞書順の最小）。なければ nil
    private func segmentChoices(forming surface: String, in state: inout ConvertingState) -> [Int]? {
        for index in state.segments.indices where state.segments[index].pages.isEmpty {
            guard let page = fetchPage(for: state, segment: index, index: 0) else { return nil }
            state.segments[index].pages = [page]
        }
        let lists = state.segments.map { $0.pages[0].candidates.map(\.surface) }
        var picks: [Int] = []
        func search(_ segment: Int, _ rest: Substring) -> Bool {
            if segment == lists.count { return rest.isEmpty }
            for (index, candidate) in lists[segment].enumerated() where rest.hasPrefix(candidate) {
                picks.append(index)
                if search(segment + 1, rest.dropFirst(candidate.count)) { return true }
                picks.removeLast()
            }
            return false
        }
        return search(0, Substring(surface)) ? picks : nil
    }

    /// フォーカスを左右の文節へ移す。端では反対の端へ回る（文節が 1 つなら動かない。nil）。まだ候補を持たない文節は、
    /// 先にエンジンから 1 ページ目を取る（失敗したら動かない）。候補ウィンドウは、その文節の候補に替わる
    private func moveFocus(by delta: Int) -> [KeyAction]? {
        guard case .converting(var current) = state else { return nil }
        let count = current.segments.count
        guard count > 1 else { return nil }
        let target = ((current.focused + delta) % count + count) % count
        if current.segments[target].pages.isEmpty {
            guard let page = fetchPage(for: current, segment: target, index: 0) else { return nil }
            current.segments[target].pages = [page]
        }
        current.focused = target
        current.syncFocused()
        state = .converting(current)
        return [.setMarkedText(markedText(for: current)), .showCandidates(panelModel(for: current))]
    }

    /// Shift+←/→。フォーカスしている文節を 1 文字縮める／伸ばす。左の文節はそのまま、フォーカス以降はエンジンが作り直す。
    /// 失敗（時間切れ、古い版など）は、区切りを変えずにいまの表示のまま
    private func resizeFocused(delta: Int) -> [KeyAction] {
        guard case .converting(let current) = state else { return [.consume] }
        let segment = current.segments[current.focused]
        if delta < 0, segment.reading.count <= 1 { return [.consume] }
        if delta > 0, current.focused == current.segments.count - 1 { return [.consume] }

        let timeout = EngineTimeouts.convertTimeout(readingCharacterCount: segment.reading.count)
        guard
            let layout = try? engine.resize(
                conversionID: current.conversionID, layoutRev: current.layoutRev, segment: current.focused, delta: delta,
                left: choices(of: current.segments[..<current.focused]), timeout: timeout),
            let built = Self.convertingSegments(from: layout, expectedReading: current.reading),
            built.focused >= current.focused,
            // 左の文節は変わっていないはず（変わっていたら信用しない）
            zip(current.segments[..<current.focused], built.segments).allSatisfy({ $0.reading == $1.reading })
        else { return [.consume] }

        var next = current
        next.segments = Array(current.segments[..<current.focused]) + built.segments.dropFirst(current.focused)
        next.focused = built.focused
        next.layoutRev = layout.layoutRev
        next.conversionID = layout.conversionID
        next.syncFocused()
        mode.useSegmentAPI = true
        mode.layoutChanged = true
        state = .converting(next)
        return [.setMarkedText(markedText(for: next)), .showCandidates(panelModel(for: next))]
    }

    /// 文節 `segments` の選択を、エンジンに送る形にする（取得前の文節は 0 番）
    private func choices(of segments: ArraySlice<ConvertingSegment>) -> [SegmentChoiceValue] {
        segments.map { segment in
            if let literal = segment.literal { return .literal(literal) }
            return .candidate(segment.candidateIndex)
        }
    }

    /// エンジンの区切りから、文節の列とフォーカスを作る。読みの合計が `expectedReading` と違う、フォーカスが範囲外、
    /// フォーカスの文節の候補がない、のどれかなら信用しない（nil）
    private static func convertingSegments(
        from layout: SegmentLayoutInfo, expectedReading: String?, requireFocusedPage: Bool = true
    ) -> (segments: [ConvertingSegment], focused: Int)? {
        guard layout.segments.indices.contains(layout.focused) else { return nil }
        if let expectedReading, layout.segments.map(\.reading).joined() != expectedReading { return nil }
        let focusedPage = layout.focusedPage.flatMap { $0.candidates.isEmpty ? nil : $0 }
        if requireFocusedPage && focusedPage == nil { return nil }
        let focusedPages = focusedPage.map { [conversionPage(from: $0, conversionID: layout.conversionID)] } ?? []
        let segments = layout.segments.enumerated().map { index, summary in
            ConvertingSegment(
                reading: summary.reading, surface: summary.surface, pages: index == layout.focused ? focusedPages : [])
        }
        return (segments, layout.focused)
    }

    // MARK: 変換の開始

    private func startConversion(reading: String, origin newOrigin: ReadingOrigin, fetch: () -> SurroundingText) -> [KeyAction] {
        guard !reading.isEmpty else { return [.consume] }
        origin = newOrigin

        var page: ConversionPage?
        var waitedNanos: UInt64?
        if reading.count <= Self.maxReadingCharacters {
            refreshContext(fetch: fetch)
            episode?.noteContext(available: cachedContext?.available ?? false)
            let request = ConversionRequest(
                sessionID: session.sessionID,
                reading: reading,
                context: cachedContext ?? .unavailable,
                contextRev: contextRev,
                pageSize: Settings.enginePageSize
            )
            let timeout = EngineTimeouts.convertTimeout(readingCharacterCount: reading.count)
            let waitStart = diagClock()
            // 失敗（落ちた、時間切れ、壊れた返事）は、入力を止めずに殻だけの候補にする（ADR 0004 §7.5）
            if let reply = try? engine.convert(request, timeout: timeout), !reply.candidates.isEmpty {
                page = reply
            }
            waitedNanos = diagClock() &- waitStart
        }
        episode?.noteConversion(
            readingLength: reading.count, at: diagClock(), waitedNanos: waitedNanos, usedFallback: page == nil)
        if let page {
            return enterConverting(reading: reading, page: page, isFallback: false)
        }
        return enterConverting(reading: reading, page: fallbackPage(for: reading), isFallback: true)
    }

    private func fallbackPage(for reading: String) -> ConversionPage {
        var candidates: [CandidateItem] = []
        for script in Self.fallbackScripts {
            let surface = scripts.convert(reading, to: script)
            guard !surface.isEmpty, !candidates.contains(where: { $0.surface == surface }) else { continue }
            candidates.append(CandidateItem(surface: surface, kind: kind(of: script)))
        }
        return ConversionPage(conversionID: 0, pageIndex: 0, candidates: candidates, hasNext: false)
    }

    /// 変換中に入る。いつも読み全体の候補から始める（ADR 0006 §16.6）。エンジンの返事に 2 文節以上の区切り（`layout`）があれば
    /// `wholeLayout` に覚えて、文節の操作のキーで文節のモードに入れるようにする。なければ（古いエンジン、フォールバック）区切りなし
    private func enterConverting(reading: String, page: ConversionPage, isFallback: Bool) -> [KeyAction] {
        let segments = [ConvertingSegment(reading: reading, surface: page.candidates.first?.surface ?? "", pages: [page])]
        var wholeLayout: SegmentLayoutInfo?
        var layoutRev: UInt32 = 0
        mode = SegmentMode()
        if !isFallback, let layout = page.layout,
            let built = Self.convertingSegments(
                from: layout, expectedReading: reading, requireFocusedPage: layout.segments.count > 1)
        {
            mode.canSegment = true
            layoutRev = layout.layoutRev
            // 2 文節以上でも、最初は読み全体の候補を出す。区切りは表示にだけ使う（ADR 0006 §16.6）
            if built.segments.count > 1 { wholeLayout = layout }
        }
        var converting = ConvertingState(
            reading: reading,
            conversionID: page.conversionID,
            pageIndex: 0,
            candidates: page.candidates,
            hasNext: page.hasNext,
            isFallback: isFallback,
            segments: segments,
            layoutRev: layoutRev,
            wholeLayout: wholeLayout
        )
        converting.syncFocused()
        // 確信が高い読み全体の変換は、1 位だけを出す。一覧は次の Space / ↓ で出す（#69）
        if page.confident, settings.candidateHideWhenConfident, !session.secure, !isFallback, wholeLayout == nil,
            !page.candidates.isEmpty
        {
            converting.candidatesHidden = true
            state = .converting(converting)
            return [.setMarkedText(markedText(for: converting))]
        }
        state = .converting(converting)
        return [.setMarkedText(markedText(for: converting)), .showCandidates(panelModel(for: converting))]
    }

    /// F6〜F10 とプリセットのキー。
    /// - 合成中、英字、1 文節のままの変換: 読みを字種変換して、候補が 1 つの converting に入る（パネルは出さない）
    /// - 区切りを持つ変換（2 文節以上、区切り直しのあと、再変換）: フォーカスしている文節だけを字種変換して `literal` に置く
    private func applyScript(_ action: KeymapAction) -> [KeyAction] {
        let reading: String
        var carriedOrigin = ConversionOrigin.composing
        switch state {
        case .idle, .shortcode, .shortcodeChoice:  // ショートコードの状態は `dispatch` が先に引き取るので、ここには来ない
            return [.passThrough]
        case .composing:
            reading = readingToConvert()
            origin = .composing
        case .asciiRun(let text):
            reading = text
            origin = .ascii
        case .converting(var current):
            if mode.useSegmentAPI && !current.isFallback {
                let focusedReading = current.segments[current.focused].reading
                current.segments[current.focused].literal = scripts.convert(focusedReading, to: action)
                state = .converting(current)
                return [.hideCandidates, .setMarkedText(markedText(for: current))]
            }
            reading = current.reading
            carriedOrigin = current.origin
            if !current.isFallback { engine.cancel(conversionID: current.conversionID) }
        }
        guard !reading.isEmpty else { return [.consume] }

        let surface = scripts.convert(reading, to: action)
        let page = ConversionPage(
            conversionID: 0,
            pageIndex: 0,
            candidates: [CandidateItem(surface: surface, kind: kind(of: action))],
            hasNext: false
        )
        mode = SegmentMode()
        var converting = ConvertingState(
            reading: reading, conversionID: 0, pageIndex: 0, candidates: page.candidates, hasNext: false,
            isFallback: true, segments: [ConvertingSegment(reading: reading, surface: surface, pages: [page])],
            origin: carriedOrigin
        )
        converting.syncFocused()
        state = .converting(converting)
        return [.hideCandidates, .setMarkedText(markedText(for: converting))]
    }

    // MARK: - 再変換（ADR 0006 §9）

    /// 再変換の対象の長さの上限（文字数）
    static let maxReconvertCharacters = 64

    /// idle で `reconvert` のキーを受けた。選択範囲を評価して、断る条件（§9.2）に当たれば何もしない（受け止める）
    private func startReconversion(selection: () -> SelectedText?, fetch: () -> SurroundingText) -> [KeyAction] {
        guard !session.secure, let selected = selection(), Self.isReconvertible(selected) else { return [.consume] }
        refreshContext(fetch: fetch)
        guard let context = cachedContext, context.available else { return [.consume] }

        let request = ReconvertRequest(
            sessionID: session.sessionID, surface: selected.text, context: context, pageSize: Settings.enginePageSize)
        let timeout = EngineTimeouts.convertTimeout(readingCharacterCount: selected.text.count)
        guard let layout = try? engine.reconvert(request, timeout: timeout) else { return [.consume] }
        guard let built = Self.convertingSegments(from: layout, expectedReading: nil),
            // 最初の表示は、選択していた文字列と 1 文字も変わらない（食い違う返事は信用せず、文書に触れない）
            layout.segments.map(\.surface).joined() == selected.text
        else {
            engine.cancel(conversionID: layout.conversionID)
            return [.consume]
        }

        origin = .composing
        nextEpisodeStart = .reconvert
        mode = SegmentMode(canSegment: true, useSegmentAPI: true, layoutChanged: false)
        var converting = ConvertingState(
            reading: layout.segments.map(\.reading).joined(),
            conversionID: layout.conversionID,
            pageIndex: 0,
            candidates: [],
            hasNext: false,
            segments: built.segments,
            focused: built.focused,
            layoutRev: layout.layoutRev,
            origin: .reconversion(original: selected.text, replacing: selected.range)
        )
        converting.syncFocused()
        state = .converting(converting)
        var marked = markedText(for: converting)
        marked.replacementRange = selected.range
        return [.setMarkedText(marked), .showCandidates(panelModel(for: converting))]
    }

    /// §9.2 の断る条件。範囲が正しく、64 文字まで、改行も制御文字だけの文字列でもない
    private static func isReconvertible(_ selected: SelectedText) -> Bool {
        guard selected.range.location >= 0, selected.range.length > 0, !selected.text.isEmpty,
            selected.text.count <= maxReconvertCharacters,
            !selected.text.contains(where: \.isNewline)
        else { return false }
        return !selected.text.unicodeScalars.allSatisfy { $0.properties.generalCategory == .control }
    }

    // MARK: - 確定

    /// 表示中の文字列を確定する。converting なら選択中の候補、composing なら読み（pending の n は ん にして）、asciiRun ならその文字列
    private func commitDisplay() -> [KeyAction] {
        switch state {
        case .idle:
            return []
        case .shortcode, .shortcodeChoice:
            return commitShortcodeDisplay()
        case .converting:
            return commitConverting(selecting: nil)
        case .composing:
            composer.flush()
            let text = composer.kana + composer.pending
            return commitText(text, reading: text)
        case .asciiRun(let text):
            return commitText(text, reading: text)
        }
    }

    /// 変換中の全体を確定する。`selecting` があれば、フォーカスしている文節にその候補を選んでから確定する（1 文節の数字キー）。
    /// エンジンへは、1 文節のまま変えていない変換は今までどおり `Commit`、区切りを持つ変換は `CommitSegments`。
    /// 再変換で何も変えていなければ `Cancel`（学習しない。ADR 0006 §9.7）。secure は何も送らない
    private func commitConverting(selecting index: Int?) -> [KeyAction] {
        guard case .converting(var current) = state else { return [.consume] }
        if let index {
            guard current.candidates.indices.contains(index) else { return [.consume] }
            current.segments[current.focused].selected = index
            current.segments[current.focused].literal = nil
            current.syncFocused()
        }
        let surface = current.segments.map(displayedSurface).joined()
        if !session.secure {
            if current.isFallback {
                engine.commitText(sessionID: session.sessionID, reading: current.reading, surface: surface)
            } else if !mode.useSegmentAPI {
                engine.commit(conversionID: current.conversionID, candidateIndex: current.candidateOffset + current.selected)
            } else if case .reconversion = current.origin, isUnchanged(current) {
                engine.cancel(conversionID: current.conversionID)
            } else {
                engine.commitSegments(
                    conversionID: current.conversionID, layoutRev: current.layoutRev,
                    choices: choices(of: current.segments[...]))
            }
        }
        let splitsSegments = current.segments.count > 1 || mode.useSegmentAPI
        resetComposition(
            ending: externalEnd ?? (splitsSegments ? .commitSegments : .commitCand), converting: current)
        return [.commit(surface), .hideCandidates]
    }

    /// 区切りも、すべての文節の選択も、最初のまま
    private func isUnchanged(_ current: ConvertingState) -> Bool {
        !mode.layoutChanged && current.segments.allSatisfy { $0.literal == nil && $0.candidateIndex == 0 }
    }

    private func commitText(_ text: String, reading: String) -> [KeyAction] {
        resetComposition(ending: externalEnd ?? (currentKey == .enter ? .commitKana : .commitComposition))
        guard !text.isEmpty else { return [clearMark()] }
        if !session.secure {
            engine.commitText(sessionID: session.sessionID, reading: reading, surface: text)
        }
        return [.commit(text)]
    }

    /// `end` は診断ログの終わった理由。nil なら記録を捨てる（打ち直しなど、1 回の合成として数えないとき）
    private func resetComposition(ending end: DiagEnd?, converting: ConvertingState? = nil) {
        if let end { emitEpisode(end, converting: converting) } else { episode = nil }
        composer.clear()
        state = .idle
        origin = .composing
        asciiPrefix = ""
        mode = SegmentMode()
        restoreTarget = nil
        lastPrefetchedReading = nil
        composingKeystrokes = 0
        // 確定すると左文脈が変わる。次の合成の始めに取り直す
        cachedContext = nil
    }

    // MARK: - ショートコード（ADR 0027）

    private var isShortcodeState: Bool {
        switch state {
        case .shortcode, .shortcodeChoice: true
        default: false
        }
    }

    private var isShortcodeChoice: Bool {
        if case .shortcodeChoice = state { true } else { false }
    }

    /// 入ってよい条件（§3.2 の共通とモードごと）。機械は表が空のとき、修飾キー付きのとき、入りのキーでないときに自分で断る
    private var canEnterShortcode: Bool {
        state == .idle && !replayingShortcode && !inputIsRepeat && !session.secure && settings.emojiShortcode
            && !settings.emojiShortcodeExclude.contains(session.clientBundleID)
    }

    /// idle で入りのキー（ローマ字は `:`、かなは Shift+けのキーか `:` の文字）を受けたら、`shortcode` に入る
    private func enterShortcode(_ input: KeyInput) -> [KeyAction]? {
        guard canEnterShortcode else { return nil }
        let result = shortcode.handle(input, canEnter: true)
        guard case .typing(let marked, _) = result.outcome, case .typing(let typing) = result.phase else { return nil }
        composer.clear()
        state = .shortcode(typing)
        return [.setMarkedText(MarkedText(text: marked, highlightedLength: marked.utf16.count))]
    }

    private func handleShortcode(
        _ input: KeyInput, fetch: () -> SurroundingText, selection: () -> SelectedText?
    ) -> [KeyAction] {
        let previous = state
        let wasChoice = isShortcodeChoice
        let result = shortcode.handle(input, canEnter: false)
        switch result.outcome {
        case .notHandled:
            // 機械と状態がずれた（起きないはず）。表示を消して idle に戻し、キーを通常の経路で処理する
            resetComposition(ending: nil)
            return [clearMark(), .hideCandidates] + dispatch(input, fetch: fetch, selection: selection)
        case .typing(let marked, let hidesCandidates):
            guard case .typing(let typing) = result.phase else { return [.consume] }
            state = .shortcode(typing)
            let mark = KeyAction.setMarkedText(MarkedText(text: marked, highlightedLength: marked.utf16.count))
            return hidesCandidates ? [mark, .hideCandidates] : [mark]
        case .cancelled:
            // `:` だけで Backspace（Esc は打ち直しになる）
            resetComposition(ending: .bsEmpty)
            return [clearMark()]
        case .showChoices(let choosing):
            state = .shortcodeChoice(choosing)
            return [shortcodeMark(choosing), .showCandidates(shortcodePanelModel(choosing))]
        case .moveChoice(let choosing):
            state = .shortcodeChoice(choosing)
            var samePage = false
            if case .shortcodeChoice(let before) = previous {
                samePage = Self.shortcodePage(of: before) == Self.shortcodePage(of: choosing)
            }
            let model = shortcodePanelModel(choosing)
            return [shortcodeMark(choosing), samePage ? .moveSelection(model) : .showCandidates(model)]
        case .commit(let commit):
            var actions = commitShortcodeText(commit, hidesCandidates: wasChoice, end: wasChoice ? .shortcodePick : .shortcodeLiteral)
            switch commit.after {
            case .consume: break
            case .passKey: actions.append(.passThrough)
            case .continueWith(let key): actions += continueAfterCommit(key, fetch: fetch)
            }
            return actions
        case .retype(let keys, _):
            // 日本語のモードの Esc だけ、今のキーを流さずに（合成に戻って）消費する
            return retypeShortcode(keys, includesCurrentKey: input.key != .escape, fetch: fetch)
        }
    }

    /// 打ち直し（§3.3）。入ってから受けたキーを、ショートコードを切った idle から、そのモードのふつうの経路に順に流す。
    /// 見えていた `:ey` は先に消す。途中のキーは、もうアプリへ渡せないので `passThrough` を捨てる
    /// （かなの Shift+け は今も何も出ない）。最後のキーが今のキーなら、その `passThrough` は残す（Cmd 付きなど）
    private func retypeShortcode(_ keys: [KeyInput], includesCurrentKey: Bool, fetch: () -> SurroundingText) -> [KeyAction] {
        resetComposition(ending: nil)
        replayingShortcode = true
        defer { replayingShortcode = false }
        var actions: [KeyAction] = [clearMark()]
        for (index, key) in keys.enumerated() {
            var replayed = dispatch(key, fetch: fetch, selection: { nil })
            if !(includesCurrentKey && index == keys.count - 1) { replayed.removeAll { $0 == .passThrough } }
            actions += replayed
        }
        return actions
    }

    /// 今の表示のまま確定する（モードの切り替え、secure、`deactivateServer`、クリック）
    private func commitShortcodeDisplay() -> [KeyAction] {
        let choosing = isShortcodeChoice
        guard let commit = shortcode.flush() else {
            resetComposition(ending: externalEnd ?? .commitComposition)
            return choosing ? [clearMark(), .hideCandidates] : [clearMark()]
        }
        return commitShortcodeText(commit, hidesCandidates: choosing, end: externalEnd ?? .commitComposition)
    }

    /// 確定の文字列を挿入し、確定履歴に送る（読みは `:` + 名前。§3.7）。学習には送らない
    private func commitShortcodeText(_ commit: ShortcodeCommit, hidesCandidates: Bool, end: DiagEnd) -> [KeyAction] {
        resetComposition(ending: end)
        if !session.secure {
            engine.commitText(sessionID: session.sessionID, reading: commit.reading, surface: commit.text)
        }
        return hidesCandidates ? [.commit(commit.text), .hideCandidates] : [.commit(commit.text)]
    }

    private func shortcodeMark(_ choosing: ShortcodeChoosing) -> KeyAction {
        .setMarkedText(MarkedText(text: choosing.marked, highlightedLength: choosing.marked.utf16.count))
    }

    private static func shortcodePage(of choosing: ShortcodeChoosing) -> Int {
        choosing.selected / ShortcodeMachine.candidatePageSize
    }

    /// 候補ウィンドウ。候補は 9 件ずつのページで見せる（数字キーはそのページの番号）
    private func shortcodePanelModel(_ choosing: ShortcodeChoosing) -> CandidatePanelModel {
        let size = ShortcodeMachine.candidatePageSize
        let start = Self.shortcodePage(of: choosing) * size
        let page = choosing.candidates[start..<min(start + size, choosing.candidates.count)]
        return CandidatePanelModel(
            candidates: page.map {
                CandidateItem(surface: $0.surface, kind: $0.label == nil ? .literal : .unspecified, label: $0.label ?? "")
            },
            selected: choosing.selected - start,
            annotation: "",
            reservedRows: min(choosing.candidates.count, size))
    }

    // MARK: - 文脈と先行計算

    private func refreshContext(fetch: () -> SurroundingText) {
        guard !session.secure else {
            cachedContext = nil
            return
        }
        let fresh = fetch().truncated()
        if fresh != cachedContext {
            cachedContext = fresh
            contextRev &+= 1
        }
    }

    private func prefetchIfChanged(reading: String) {
        guard !session.secure, !reading.isEmpty, reading.count <= Self.maxReadingCharacters,
            reading != lastPrefetchedReading
        else {
            if reading.isEmpty { lastPrefetchedReading = nil }
            return
        }
        lastPrefetchedReading = reading
        engine.prefetch(
            ConversionRequest(
                sessionID: session.sessionID,
                reading: reading,
                context: cachedContext ?? .unavailable,
                contextRev: contextRev,
                pageSize: Settings.enginePageSize
            ))
    }

    // MARK: - 小さな部品

    private func feed(_ text: String) {
        for scalar in text.unicodeScalars { composer.pushCharacter(scalar) }
    }

    private func hasCommandModifiers(_ input: KeyInput) -> Bool {
        !input.modifiers.intersection(Self.commandModifiers).isEmpty
    }

    /// キー割り当て。F キーと、Ctrl / Option / Command を伴う 1 文字は常に引く。
    /// ←/→ は修飾キーの有無を問わず引く（`segment.*`。idle ではアプリへ渡し、合成中は受け止める）
    private func boundAction(for input: KeyInput) -> KeymapAction? {
        switch input.key {
        case .function(let number):
            return keymap.action(for: KeyChord(key: "f\(number)", modifiers: input.modifiers))
        case .character(let text):
            guard hasCommandModifiers(input), text.count == 1 else { return nil }
            return keymap.action(for: KeyChord(key: text.lowercased(), modifiers: input.modifiers))
        case .left:
            return keymap.action(for: KeyChord(key: "left", modifiers: input.modifiers))
        case .right:
            return keymap.action(for: KeyChord(key: "right", modifiers: input.modifiers))
        case .up:
            return keymap.action(for: KeyChord(key: "up", modifiers: input.modifiers))
        case .down:
            return keymap.action(for: KeyChord(key: "down", modifiers: input.modifiers))
        default:
            return nil
        }
    }

    private func isShiftedLetter(_ text: String, _ input: KeyInput) -> Bool {
        guard input.modifiers.contains(.shift), text.count == 1, let scalar = text.unicodeScalars.first else {
            return false
        }
        return scalar.isASCII && scalar.properties.isAlphabetic
    }

    /// 制御文字と私用領域（矢印キーなどが返す U+F700 台）は入力として扱わない
    private func isPrintable(_ text: String) -> Bool {
        !text.isEmpty
            && text.unicodeScalars.allSatisfy {
                switch $0.properties.generalCategory {
                case .control, .privateUse, .surrogate, .unassigned: false
                default: true
                }
            }
    }

    private func clearMark() -> KeyAction {
        .setMarkedText(MarkedText(text: ""))
    }

    /// 文節 1 つの、いま表示している表記。字種変換の `literal` があればそれ、まだ候補を取っていない文節は最初の表記
    private func displayedSurface(_ segment: ConvertingSegment) -> String {
        if let literal = segment.literal { return literal }
        if segment.pages.indices.contains(segment.pageIndex),
            segment.pages[segment.pageIndex].candidates.indices.contains(segment.selected)
        {
            return segment.pages[segment.pageIndex].candidates[segment.selected].surface
        }
        return segment.surface
    }

    /// 1 文節なら今までの見た目（全体を太い下線）。2 文節以上なら、文節ごとの UTF-16 の長さとフォーカス。
    /// 全体の候補を出している間（`wholeLayout`）は、表示が最初の区切りの表記のままなら、区切りだけを付ける（フォーカスなし。§16.6）
    private func markedText(for state: ConvertingState) -> MarkedText {
        let surfaces = state.segments.map(displayedSurface)
        if let layout = state.wholeLayout {
            let surface = surfaces.first ?? ""
            let parts = layout.segments.map(\.surface)
            if parts.joined() == surface {
                return MarkedText(text: surface, segments: parts.map { $0.utf16.count }, focused: nil)
            }
            return MarkedText(text: surface, highlightedLength: surface.utf16.count)
        }
        guard surfaces.count > 1 else {
            let surface = surfaces.first ?? state.candidates[state.selected].surface
            return MarkedText(text: surface, highlightedLength: surface.utf16.count)
        }
        return MarkedText(text: surfaces.joined(), segments: surfaces.map { $0.utf16.count }, focused: state.focused)
    }

    private func panelModel(for state: ConvertingState) -> CandidatePanelModel {
        // 見せる件数は `visibleCount`。このあと広がる、またはページが進み得るときは、位置を決める高さを
        // ページの上限で見込む（広げたときに上下が入れ替わらないように。ADR 0014 §2.2）
        let visible = state.visibleCount(initial: settings.pageSize)
        let mayGrow = state.hasNext || state.pageIndex > 0
        let clauseHeader = clauseHeader(for: state)
        return CandidatePanelModel(
            candidates: Array(state.candidates.prefix(visible)),
            selected: state.selected,
            annotation: state.candidates[state.selected].annotation,
            reservedRows: mayGrow ? Settings.enginePageSize : state.candidates.count,
            clauses: clauseHeader.clauses, focusedClause: clauseHeader.focused
        )
    }

    /// 候補ウィンドウの先頭の行に出す文節（#292）。2 文節以上のときだけ。規則は `markedText(for:)` と同じ
    private func clauseHeader(for state: ConvertingState) -> (clauses: [String], focused: Int?) {
        let surfaces = state.segments.map(displayedSurface)
        if let layout = state.wholeLayout {
            let parts = layout.segments.map(\.surface)
            guard parts.count > 1, parts.joined() == (surfaces.first ?? "") else { return ([], nil) }
            return (parts, nil)
        }
        guard surfaces.count > 1 else { return ([], nil) }
        return (surfaces, state.focused)
    }

    private func kind(of script: KeymapAction) -> CandidateKind {
        switch script {
        case .toHiragana: return .hiragana
        case .toFullwidthKatakana, .toHalfwidthKatakana: return .katakana
        case .toFullwidthAlnum, .toHalfwidthAlnum: return .unspecified
        case .segmentFocusLeft, .segmentFocusRight, .segmentShrink, .segmentExpand, .reconvert, .candidateNext,
            .candidatePrevious, .modeToggle, .modeEnglish, .modeJapanese:
            return .unspecified
        }
    }
}

// MARK: - 文節の表示と状態の同期（モジュール内）

extension ConvertingSegment {
    /// 取得済みの列の中の通し番号（ページをまたぐ）。まだ候補を取っていない文節は 0 番
    var candidateIndex: Int {
        guard pages.indices.contains(pageIndex) else { return 0 }
        return pages[..<pageIndex].reduce(0) { $0 + $1.candidates.count } + selected
    }
}

extension ConvertingState {
    /// 候補ウィンドウに見せる件数。最初のページは `initial` 件（最大でページの件数）から始め、広げたあと、
    /// および 2 ページ目以降はページ全部を見せる（ADR 0014 §2.3 (c)）
    func visibleCount(initial: Int) -> Int {
        let expanded = segments.indices.contains(focused) ? segments[focused].expanded : true
        if expanded || pageIndex > 0 { return candidates.count }
        return min(max(initial, 1), candidates.count)
    }

    /// フラットなフィールド（候補ウィンドウと `MarkedText` の元）を、フォーカスしている文節の現在のページに合わせる
    mutating func syncFocused() {
        guard segments.indices.contains(focused) else { return }
        let segment = segments[focused]
        guard segment.pages.indices.contains(segment.pageIndex) else { return }
        let page = segment.pages[segment.pageIndex]
        pageIndex = segment.pageIndex
        candidates = page.candidates
        hasNext = page.hasNext
        selected = segment.selected
        candidateOffset = segment.pages[..<segment.pageIndex].reduce(0) { $0 + $1.candidates.count }
    }
}
