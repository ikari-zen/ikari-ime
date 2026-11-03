// エンジンとの境界の型（ADR 0004 §5、§7.5）。`XPCEngineClient`（smoke が使う）の入出力。
// IME の殻は shell-core（Rust）がエンジンと話すので、これらの型を使わない（ADR 0041。M5 で IkariShellCore から移した）。
import Foundation

/// エンジンの LM の状態（`LmState` に対応）。
public enum EngineLmState: Sendable, Equatable {
    case unspecified, disabled, loading, ready, failed
}

public struct EngineHello: Sendable, Equatable {
    public var engineVersion: String
    public var dictLabel: String
    public var lm: EngineLmState

    public init(engineVersion: String, dictLabel: String, lm: EngineLmState) {
        self.engineVersion = engineVersion
        self.dictLabel = dictLabel
        self.lm = lm
    }
}

/// 周辺テキスト。`available == false` は取れなかった（ターミナル、一部の Electron）。
/// `left` は末尾 2000 文字に切って渡す（`SurroundingText.maxCharacters`）。
public struct SurroundingText: Sendable, Equatable {
    public static let maxCharacters = 2000

    public var left: String
    public var right: String
    public var available: Bool

    public init(left: String = "", right: String = "", available: Bool = false) {
        self.left = left
        self.right = right
        self.available = available
    }

    public static let unavailable = SurroundingText()
}

public struct SessionInfo: Sendable, Equatable {
    public var sessionID: UInt64
    public var clientBundleID: String
    /// `IsSecureEventInputEnabled()` が真
    public var secure: Bool

    public init(sessionID: UInt64, clientBundleID: String, secure: Bool) {
        self.sessionID = sessionID
        self.clientBundleID = clientBundleID
        self.secure = secure
    }
}

/// `Prefetch` と `Convert` の共通の入力。
public struct ConversionRequest: Sendable, Equatable {
    /// 上限 256 文字（エンジン側でも検証する）
    public var sessionID: UInt64
    public var reading: String
    public var context: SurroundingText
    /// 周辺テキストを取り直すたびに増やす（先行計算のキャッシュのキーの一部）
    public var contextRev: UInt64
    /// 1..9
    public var pageSize: Int

    public init(sessionID: UInt64, reading: String, context: SurroundingText, contextRev: UInt64, pageSize: Int = 9) {
        self.sessionID = sessionID
        self.reading = reading
        self.context = context
        self.contextRev = contextRev
        self.pageSize = pageSize
    }
}

public enum CandidateKind: Sendable, Equatable {
    case unspecified, dictionary, hiragana, katakana, rewriter
    /// 再変換で読めなかった部分の、元の表記だけの候補（ADR 0006 §9.3）
    case literal
}

public struct CandidateItem: Sendable, Equatable {
    public var surface: String
    /// 2行目に出す注釈。フェーズ2はデータなし（空）
    public var annotation: String
    public var kind: CandidateKind
    /// 行の右に薄く出す名前（ショートコードの `:eyes:`。ADR 0027 §3.4）。空なら出さない
    public var label: String

    public init(surface: String, annotation: String = "", kind: CandidateKind = .dictionary, label: String = "") {
        self.surface = surface
        self.annotation = annotation
        self.kind = kind
        self.label = label
    }
}

/// 1ページ分の候補（`ConvertReply` に対応）。
public struct ConversionPage: Sendable, Equatable {
    /// エンジンが付ける。フォールバックの候補では 0
    public var conversionID: UInt64
    public var pageIndex: Int
    public var candidates: [CandidateItem]
    public var hasNext: Bool
    public var lmApplied: Bool
    /// 1 位と 2 位の差が閾値以上（`ConvertReply.confident`。#69）。`convert` の返事だけで意味を持つ
    public var confident: Bool
    public var engineMicros: Int
    /// 区切り（`ConvertReply.layout`）。古いエンジンは nil で、殻は 1 文節として扱う。`page` の返事では nil
    public var layout: SegmentLayoutInfo?

    public init(
        conversionID: UInt64,
        pageIndex: Int,
        candidates: [CandidateItem],
        hasNext: Bool,
        lmApplied: Bool = false,
        confident: Bool = false,
        engineMicros: Int = 0,
        layout: SegmentLayoutInfo? = nil
    ) {
        self.conversionID = conversionID
        self.pageIndex = pageIndex
        self.candidates = candidates
        self.hasNext = hasNext
        self.lmApplied = lmApplied
        self.confident = confident
        self.engineMicros = engineMicros
        self.layout = layout
    }
}

// MARK: 文節と再変換（ADR 0006 §5.2、§8.5）

/// 文節 1 つの読みと、候補の 0 番（いま表示している表記）。`SegmentInfo` に対応
public struct SegmentSummary: Sendable, Equatable {
    public var reading: String
    public var surface: String

    public init(reading: String, surface: String) {
        self.reading = reading
        self.surface = surface
    }
}

/// 文節の候補の 1 ページ（`SegmentPageReply` に対応）
public struct SegmentPage: Sendable, Equatable {
    public var segment: Int
    public var pageIndex: Int
    public var candidates: [CandidateItem]
    public var hasNext: Bool
    public var lmApplied: Bool
    public var engineMicros: Int

    public init(
        segment: Int, pageIndex: Int, candidates: [CandidateItem], hasNext: Bool, lmApplied: Bool = false,
        engineMicros: Int = 0
    ) {
        self.segment = segment
        self.pageIndex = pageIndex
        self.candidates = candidates
        self.hasNext = hasNext
        self.lmApplied = lmApplied
        self.engineMicros = engineMicros
    }
}

/// 区切りと、フォーカスしている文節の 1 ページ目（`SegmentLayout` に対応）
public struct SegmentLayoutInfo: Sendable, Equatable {
    public var conversionID: UInt64
    public var layoutRev: UInt32
    public var segments: [SegmentSummary]
    public var focused: Int
    public var focusedPage: SegmentPage?
    public var engineMicros: Int

    public init(
        conversionID: UInt64, layoutRev: UInt32, segments: [SegmentSummary], focused: Int = 0,
        focusedPage: SegmentPage? = nil, engineMicros: Int = 0
    ) {
        self.conversionID = conversionID
        self.layoutRev = layoutRev
        self.segments = segments
        self.focused = focused
        self.focusedPage = focusedPage
        self.engineMicros = engineMicros
    }
}

/// 文節の選択（`SegmentChoice` に対応）。`literal` は殻が作った表記で、学習しない。上限は 256 文字
public enum SegmentChoiceValue: Sendable, Equatable {
    /// 生成済みの列の中の通し番号（ページをまたぐ）
    case candidate(Int)
    case literal(String)
}

/// 再変換の要求（`Reconvert` に対応）。`surface` は 64 文字まで（殻でも断る。ADR 0006 §9.2）
public struct ReconvertRequest: Sendable, Equatable {
    public var sessionID: UInt64
    public var surface: String
    /// 左は選択範囲の手前の周辺テキスト
    public var context: SurroundingText
    /// 1..9
    public var pageSize: Int

    public init(sessionID: UInt64, surface: String, context: SurroundingText, pageSize: Int = 9) {
        self.sessionID = sessionID
        self.surface = surface
        self.context = context
        self.pageSize = pageSize
    }
}

public enum EngineErrorCode: Sendable, Equatable {
    case schemaMismatch, badRequest, unknownSession, unknownConversion, engineNotReady, `internal`, unspecified
    /// `layout_rev` が古い（ADR 0006 §5.2）
    case staleLayout
}

public enum EngineClientError: Error, Sendable, Equatable {
    /// エンジンにつながらない（落ちた、再起動中、連続して落ちて休止中）
    case unavailable
    /// `lm_budget(読み) + 40ms` を超えた（返事は捨てる）
    case timeout
    /// 返事が壊れている、または候補の数や長さが範囲外
    case invalidReply
    case engine(EngineErrorCode)
}

/// `Shutdown` を送る理由（`ShutdownReason` に対応。ADR 0005 §8.2）。どちらもエンジンは学習を保存して終了し、
/// 殻は「予期した終了」として扱う（落ちた回数に数えない）。
public enum EngineShutdownReason: Sendable, Equatable {
    /// 殻とエンジンの版が違う（入れ替えた直後に古いエンジンが残っている）
    case versionMismatch
    /// `engine.*` の設定が変わった。止めたあと `hello` で起こし直すと、新しい設定で開く（ADR 0005 §7.3）
    case settingsChanged
}

// 変換1回の待ち時間（ADR 0004 §3.4、§7.5）。エンジン側の `LmBudget` と同じ式。
public enum EngineTimeouts {
    public static let base: Duration = .milliseconds(30)
    public static let perCharacter: Duration = .milliseconds(6)
    public static let maximum: Duration = .milliseconds(300)
    /// 殻が `lm_budget` に足す IPC の余裕
    public static let ipcMargin: Duration = .milliseconds(40)

    /// `min(300ms, max(30ms, 6ms × 読みの文字数))`
    public static func lmBudget(readingCharacterCount: Int) -> Duration {
        let count = max(0, readingCharacterCount)
        return min(maximum, max(base, perCharacter * count))
    }

    /// 管理の要求の待ちの上限（ADR 0005 §8.3、docs/phase2-contract.md §5.3）。インポートとエクスポート以外
    public static let admin: Duration = .seconds(3)
    /// インポートとエクスポートの待ちの上限
    public static let adminTransfer: Duration = .seconds(30)

    /// `Convert` と `Page` を同期で待つ上限 = `lm_budget(読み) + 40ms`
    public static func convertTimeout(readingCharacterCount: Int) -> Duration {
        lmBudget(readingCharacterCount: readingCharacterCount) + ipcMargin
    }
}
