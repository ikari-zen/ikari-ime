// 英語のモードの絵文字のショートコード（ADR 0027 §3.2、§3.3、§3.4 の英語の列。B48）。
// `ShortcodeMachine`（`.english`）への委譲と、文脈のたまりへの反映をここにまとめる。AppKit にもエンジンにも依存しない純関数と値型で、
// コントローラ（`IkariInputController`）は返り値の `actions` を実行し、`passKey` ならキーをアプリに返すだけにする。
// 英語の経路でエンジンを同期で呼ばない（表は殻のメモリのもの。ADR 0026 §9）。
import Foundation

/// ショートコードの入りの判定に要る、打鍵のときの状況
public struct EnglishShortcodeEnvironment: Sendable, Equatable {
    public var settings: Settings
    public var clientBundleID: String
    public var secure: Bool
    public var isRepeat: Bool

    public init(settings: Settings, clientBundleID: String, secure: Bool, isRepeat: Bool) {
        self.settings = settings
        self.clientBundleID = clientBundleID
        self.secure = secure
        self.isRepeat = isRepeat
    }
}

/// 英語のモードの打鍵（`.text`）を処理した結果
public struct EnglishTextOutcome: Sendable, Equatable {
    /// 殻が実行する操作（変換中の文字列、確定、候補ウィンドウ）。`passThrough` は含めない
    public var actions: [KeyAction]
    /// 今のキーをアプリに渡す（`handle` が false）。false ならキーを消費する
    public var passKey: Bool
    /// 区切りで取り出した文脈のたまり。殻が `CommitText`（読みは空）としてエンジンに送る
    public var flushed: [String]

    public init(actions: [KeyAction] = [], passKey: Bool, flushed: [String] = []) {
        self.actions = actions
        self.passKey = passKey
        self.flushed = flushed
    }
}

extension EnglishMode {
    /// ショートコードの状態に入っている（変換中の文字列か候補を出している）
    public var isShortcodeActive: Bool { shortcode.isActive }

    /// 殻が受け取った絵文字の表を渡す
    public mutating func setShortcodeTable(_ table: ShortcodeTable) {
        shortcode.table = table
    }

    /// 語の頭か（ADR 0027 §3.2）。直前の文字が、ない（区切りのあと。直前が分からない）、空白・改行、`(`、`[`、`{` のどれか
    public static func isWordStart(after last: Character?) -> Bool {
        guard let last else { return true }
        return last.isWhitespace || last.isNewline || wordStartOpeners.contains(last)
    }

    private static let wordStartOpeners: Set<Character> = ["(", "[", "{"]

    /// 入ってよいか（§3.2 の共通の条件と、英語の語の頭）。修飾キーと表が空かどうかは `ShortcodeMachine` が見る
    static func canEnterShortcode(environment: EnglishShortcodeEnvironment, last: Character?) -> Bool {
        environment.settings.emojiShortcode && !environment.secure && !environment.isRepeat
            && !environment.settings.emojiShortcodeExclude.contains(environment.clientBundleID)
            && isWordStart(after: last)
    }

    /// 英語のモードで、`mode.*` でも secure でもない打鍵。ショートコードの口を通し、関係なければ文脈のたまりに足して素通しにする
    /// （ADR 0026 §5.2 の 5-3）。確定した文字列（`:eyes:` でも絵文字でも）はたまりに足す
    public mutating func handleText(
        _ input: KeyInput, environment: EnglishShortcodeEnvironment, now: UInt64 = DispatchTime.now().uptimeNanoseconds
    ) -> EnglishTextOutcome {
        let before = shortcode.phase
        let canEnter = Self.canEnterShortcode(environment: environment, last: lastCharacter)
        shortcode.smallLetters = environment.settings.emojiSmallLetters
        let result = shortcode.handle(input, canEnter: canEnter)
        let wasChoice: Bool
        if case .choosing = before { wasChoice = true } else { wasChoice = false }
        noteDiag(input, outcome: result.outcome, wasChoice: wasChoice, now: now)
        var actions: [KeyAction] = []
        var committed: String?
        var passKey = false
        switch result.outcome {
        case .notHandled:
            return EnglishTextOutcome(passKey: true, flushed: record(input).map { [$0] } ?? [])
        case .typing(let marked, let hidesCandidates):
            actions = [Self.mark(marked, underlined: false)]
            if hidesCandidates { actions.append(.hideCandidates) }
        case .cancelled:
            // `:` だけで BackSpace。見えていた `:` を消し、BackSpace は消費する
            actions = [Self.mark("")]
        case .showChoices(let choosing):
            actions = [Self.mark(choosing.marked), .showCandidates(choosing.panelModel)]
        case .moveChoice(let choosing):
            var samePage = false
            if case .choosing(let previous) = before { samePage = previous.page == choosing.page }
            actions = [
                Self.mark(choosing.marked),
                samePage ? .moveSelection(choosing.panelModel) : .showCandidates(choosing.panelModel),
            ]
        case .commit(let commit):
            committed = commit.text
            actions = [.commit(commit.text)]
            if wasChoice { actions.append(.hideCandidates) }
            // 英語のモードの `after` は `.consume` と `.passKey` だけ（`.continueWith` は日本語のモード用）
            if case .consume = commit.after { passKey = false } else { passKey = true }
        case .retype(_, let literal):
            // 過去のキーは渡し直せないので、見えていた文字列を確定し、今のキーだけアプリに渡す（§3.3）
            committed = literal
            actions = [.commit(literal)]
            if wasChoice { actions.append(.hideCandidates) }
            passKey = true
        }
        var flushed: [String] = []
        if let committed, let overflow = appendCommitted(committed) { flushed.append(overflow) }
        if passKey, let key = record(input) { flushed.append(key) }
        return EnglishTextOutcome(actions: actions, passKey: passKey, flushed: flushed)
    }

    /// モードの切り替え、secure、`deactivateServer`、クリック、`inputControllerWillClose` のとき、今の表示のまま確定する。
    /// 状態に入っていなければ nil。たまりは取り出さない（呼ぶ側が続けて `flush` する）
    /// `end` は診断ログの終わった理由（呼び出しの理由に合わせる）
    public mutating func flushShortcode(
        end: DiagEnd = .commitComposition, now: UInt64 = DispatchTime.now().uptimeNanoseconds
    ) -> EnglishTextOutcome? {
        let wasChoice: Bool
        if case .choosing = shortcode.phase { wasChoice = true } else { wasChoice = false }
        guard let commit = shortcode.flush() else { return nil }
        finishDiag(end, prev: wasChoice ? .shortcodeChoice : .shortcode, commit: nil, now: now)
        var actions: [KeyAction] = [.commit(commit.text)]
        if wasChoice { actions.append(.hideCandidates) }
        return EnglishTextOutcome(actions: actions, passKey: false, flushed: appendCommitted(commit.text).map { [$0] } ?? [])
    }

    // MARK: 診断ログ（ADR 0034 §4.2）。数えるのは整数の加算だけ。文字は持たない

    /// 候補ウィンドウを出した（コントローラが、位置を取ったあとに呼ぶ）。合成の最初の 1 回だけ覚える
    public mutating func notePanelShown(source: AnchorSource) {
        diagEpisode?.notePanel(source)
    }

    /// 終わった合成を取り出す（なければ nil）。secure のときは呼ぶ側（`recordEnglishEpisode`）が捨てる
    public mutating func takeFinishedDiag() -> DiagFinishedEpisode? {
        defer { finishedDiag = nil }
        return finishedDiag
    }

    private mutating func noteDiag(_ input: KeyInput, outcome: ShortcodeOutcome, wasChoice: Bool, now: UInt64) {
        switch outcome {
        case .notHandled:
            return
        case .retype:
            // 打ち直しは 1 回の合成として数えない（日本語のモードと同じ）
            diagEpisode = nil
            return
        default:
            break
        }
        if diagEpisode == nil {
            var started = DiagEpisode(mode: .shortcodeEn, start: .typing, startedAt: now)
            started.keys = 0
            diagEpisode = started
        }
        diagEpisode?.keys += 1
        if input.key == .backspace { diagEpisode?.bs += 1 }
        switch outcome {
        case .showChoices:
            if var episode = diagEpisode, episode.convertedAt == nil {
                episode.convertedAt = now
                episode.keysToConv = episode.keys
                diagEpisode = episode
            }
        case .moveChoice:
            diagEpisode?.candMoves += 1
        case .cancelled:
            finishDiag(.bsEmpty, prev: .shortcode, commit: nil, now: now)
        case .commit:
            let commit = wasChoice ? DiagCommitInfo(pick: nil, pickKind: shortcode.lastPickKind, segs: nil, segChanged: nil) : nil
            finishDiag(wasChoice ? .shortcodePick : .shortcodeLiteral, prev: nil, commit: commit, now: now)
        case .typing, .notHandled, .retype:
            break
        }
    }

    private mutating func finishDiag(_ end: DiagEnd, prev: DiagPrev?, commit: DiagCommitInfo?, now: UInt64) {
        guard let episode = diagEpisode else { return }
        diagEpisode = nil
        finishedDiag = DiagFinishedEpisode(
            episode: episode, end: end, prev: end.isExternal ? prev : nil, commit: commit, endedAt: now)
    }

    /// 打っている途中（`.typing`）は下線なし、候補を選んでいる間（`.choosing`）は下線つき（B51）
    private static func mark(_ text: String, underlined: Bool = true) -> KeyAction {
        .setMarkedText(MarkedText(text: text, highlightedLength: text.utf16.count, underlined: underlined))
    }
}

extension ShortcodeChoosing {
    /// 選んでいる候補のページ（1 ページは 9 件。数字キーはそのページの番号）
    var page: Int { selected / ShortcodeMachine.candidatePageSize }

    /// 候補ウィンドウ。絵文字の行の右に名前（`label`）を薄く出し、「そのままの入力」の行は `.literal`
    var panelModel: CandidatePanelModel {
        let size = ShortcodeMachine.candidatePageSize
        let start = page * size
        let rows = candidates[start..<min(start + size, candidates.count)]
        return CandidatePanelModel(
            candidates: rows.map {
                CandidateItem(surface: $0.surface, kind: $0.label == nil ? .literal : .unspecified, label: $0.label ?? "")
            },
            selected: selected - start,
            annotation: "",
            reservedRows: min(candidates.count, size))
    }
}
