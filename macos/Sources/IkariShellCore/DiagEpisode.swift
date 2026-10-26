// 合成 1 回の診断の数え上げ（ADR 0034 §4.2 の `sh_episode`）。
// 貯めるのはメモリ上の整数だけ。文字列そのものは持たない（NFR-19）。キーと語は engine の許可リスト（diag_schema.rs）に合わせる。
import Foundation

/// `end` タグ: 状態が `idle` に戻った理由。
/// `secure` は作らない: secure のセッションでは貯めず送らない（NFR-18）ので、途中で secure になった合成は捨てる
public enum DiagEnd: String, Sendable {
    case commitCand = "commit_cand"
    case commitSegments = "commit_segments"
    case commitKana = "commit_kana"
    case escClear = "esc_clear"
    case bsEmpty = "bs_empty"
    case commitComposition = "commit_composition"
    case deactivate
    case modeSwitch = "mode_switch"
    case click
    case shortcodeLiteral = "shortcode_literal"
    case shortcodePick = "shortcode_pick"

    /// 外から確定させられた理由。`prev` を添える
    var isExternal: Bool {
        switch self {
        case .commitComposition, .deactivate, .modeSwitch, .click: true
        default: false
        }
    }
}

/// `prev` タグ: 外から確定させられたときの直前の状態
enum DiagPrev: String, Sendable {
    case composing
    case converting
    case segments
    case asciiRun = "ascii_run"
    case shortcode
    case shortcodeChoice = "shortcode_choice"
}

/// `start` タグ
enum DiagStart: String, Sendable {
    case typing
    case reconvert
}

/// `mode` タグ（`shortcode_en` は英語のモード側が数える。`EnglishMode`）
enum DiagMode: String, Sendable {
    case romaji
    case kana
    case shortcodeJa = "shortcode_ja"
    case shortcodeEn = "shortcode_en"
}

/// `panel` タグ: 候補ウィンドウの位置を最初に何から取ったか（`AnchorSource` と 1 対 1）。出さなかったら `none`
enum DiagPanel: String, Sendable {
    case client
    case accessibility
    case lastKnown = "last_known"
    case mouseScreen = "mouse_screen"
    case absent = "none"

    init(_ source: AnchorSource) {
        switch source {
        case .client: self = .client
        case .accessibility: self = .accessibility
        case .lastKnown: self = .lastKnown
        case .mouseScreen: self = .mouseScreen
        }
    }
}

/// `ctx` タグ: 最初の変換のときに、殻が周辺テキストを取れたか。
/// 権限の拒否（`denied`）は殻から見えない（IMK は「取れなかった」としか返さない）ので作らない
enum DiagContext: String, Sendable {
    case ok
    case absent = "none"
}

/// `pick_kind` タグ
enum DiagPickKind: String, Sendable {
    case emoji
    case smallLetter = "small_letter"
    case dict
    case hiragana
    case katakana
    case rewriter
    case literal
    case fallback
}

/// 確定したときの変換中の状態から取る値
struct DiagCommitInfo: Equatable, Sendable {
    var pick: Int?
    var pickKind: DiagPickKind?
    var segs: Int?
    var segChanged: Int?
}

/// 合成 1 回ぶんの数
struct DiagEpisode: Equatable, Sendable {
    static let kind = "sh_episode"
    private static let nanosPerMilli: UInt64 = 1_000_000
    private static let nanosPerMicro: UInt64 = 1_000

    var mode: DiagMode
    var start: DiagStart
    /// 最初の打鍵の時刻（単調時計のナノ秒）
    let startedAt: UInt64

    var keys = 0
    var keysToConv: Int?
    var rlen: Int?
    /// 最初の Space の時刻
    var convertedAt: UInt64?
    var waitMicros: Int?
    var spaces = 0
    var candMoves = 0
    var pages = 0
    var digitPicks = 0
    var segFocus = 0
    var segResize = 0
    var escBack = 0
    var bs = 0
    var fallback = 0
    /// 候補ウィンドウを最初に出したときの位置の取り方。出していなければ nil（`none` と書く）
    var panel: DiagPanel?
    /// 最初の変換のときの周辺テキスト。変換しなければ nil（書かない）
    var context: DiagContext?

    init(mode: DiagMode, start: DiagStart, startedAt: UInt64) {
        self.mode = mode
        self.start = start
        self.startedAt = startedAt
    }

    /// 変換（Space）に入った。2 回目以降の変換では、最初の値（読みの長さ、打鍵数、待ち時間）を保つ
    mutating func noteConversion(readingLength: Int, at now: UInt64, waitedNanos: UInt64?, usedFallback: Bool) {
        spaces += 1
        if usedFallback { fallback += 1 }
        guard convertedAt == nil else { return }
        convertedAt = now
        keysToConv = keys
        rlen = readingLength
        waitMicros = waitedNanos.map { Int($0 / Self.nanosPerMicro) }
    }

    /// 候補ウィンドウを出した。最初の 1 回だけ覚える
    mutating func notePanel(_ source: AnchorSource) {
        if panel == nil { panel = DiagPanel(source) }
    }

    /// 最初の変換のときの周辺テキスト（2 回目以降は見ない）
    mutating func noteContext(available: Bool) {
        if context == nil { context = available ? .ok : DiagContext.absent }
    }

    func event(
        sessionID: UInt64, end: DiagEnd, prev: DiagPrev?, commit: DiagCommitInfo?, now: UInt64
    ) -> DiagEventValue {
        var nums: [String: Int64] = [
            "keys": Int64(keys), "dur_ms": Self.millis(from: startedAt, to: now),
            "spaces": Int64(spaces), "cand_moves": Int64(candMoves), "pages": Int64(pages),
            "digit_pick": Int64(digitPicks), "seg_focus": Int64(segFocus), "seg_resize": Int64(segResize),
            "esc_back": Int64(escBack), "bs": Int64(bs), "fallback": Int64(fallback),
        ]
        var tags: [String: String] = [
            "mode": mode.rawValue, "start": start.rawValue, "end": end.rawValue, "panel": (panel ?? DiagPanel.absent).rawValue,
        ]
        if let context { tags["ctx"] = context.rawValue }
        if let prev, end.isExternal { tags["prev"] = prev.rawValue }
        if let keysToConv { nums["keys_to_conv"] = Int64(keysToConv) }
        if let rlen { nums["rlen"] = Int64(rlen) }
        if let convertedAt { nums["pick_ms"] = Self.millis(from: convertedAt, to: now) }
        if let waitMicros { nums["wait_us"] = Int64(waitMicros) }
        if let commit {
            if let segs = commit.segs { nums["segs"] = Int64(segs) }
            if let pick = commit.pick { nums["pick"] = Int64(pick) }
            if let segChanged = commit.segChanged { nums["seg_changed"] = Int64(segChanged) }
            if let kind = commit.pickKind { tags["pick_kind"] = kind.rawValue }
        }
        return DiagEventValue(sessionID: sessionID, kind: Self.kind, nums: nums, tags: tags)
    }

    private static func millis(from start: UInt64, to end: UInt64) -> Int64 {
        Int64((end >= start ? end - start : 0) / nanosPerMilli)
    }
}

/// 英語のモードで終わった合成 1 回ぶん。`EnglishMode` が作り、コントローラが `InputStateMachine.recordEnglishEpisode` で送る
/// （英語のモードの状態は状態機械の外にある。セッション ID とエンジンは状態機械が持つ）
public struct DiagFinishedEpisode: Equatable, Sendable {
    let episode: DiagEpisode
    let end: DiagEnd
    let prev: DiagPrev?
    let commit: DiagCommitInfo?
    let endedAt: UInt64

    func event(sessionID: UInt64) -> DiagEventValue {
        episode.event(sessionID: sessionID, end: end, prev: prev, commit: commit, now: endedAt)
    }
}

/// `sh_mode`（ADR 0034 §4.2）: 英語・日本語のモードが替わった。値は固定の語だけ
enum DiagModeSwitch {
    static let kind = "sh_mode"

    /// `key`: ikari のキー割り当て（`mode.*`）で替えた。`os`: macOS 経由（入力メニュー、英数・かなキー、入力ソースの切り替えキー）。
    /// macOS の通知（`setValue`）の時点では、後者のどれかは分からない（英数・かなキーは通知のあとに届く）
    enum Via: String {
        case key
        case os
    }

    static func event(sessionID: UInt64, from: InputMode, to: InputMode, via: Via) -> DiagEventValue {
        DiagEventValue(
            sessionID: sessionID, kind: kind, tags: ["from": lang(from), "to": lang(to), "via": via.rawValue])
    }

    private static func lang(_ mode: InputMode) -> String {
        mode == .japanese ? "ja" : "en"
    }
}
