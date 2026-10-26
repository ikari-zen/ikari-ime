// エンジンとの境界（ADR 0004 §5、§7.5）。契約。
// IkariShellCore は protobuf を知らない。protobuf との変換は IkariXPC が行う。
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
    /// ほかのアプリと「最近の確定」を貸し借りしてよいか（ADR 0015 D6）。除外するアプリは false。
    /// 既定の false は「貸し借りしない」で、今までの挙動と同じ。除外の判定は殻の中だけで行う
    public var shareContext: Bool

    public init(sessionID: UInt64, clientBundleID: String, secure: Bool, shareContext: Bool = false) {
        self.sessionID = sessionID
        self.clientBundleID = clientBundleID
        self.secure = secure
        self.shareContext = shareContext
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
    public var engineMicros: Int
    /// 区切り（`ConvertReply.layout`）。古いエンジンは nil で、殻は 1 文節として扱う。`page` の返事では nil
    public var layout: SegmentLayoutInfo?

    public init(
        conversionID: UInt64,
        pageIndex: Int,
        candidates: [CandidateItem],
        hasNext: Bool,
        lmApplied: Bool = false,
        engineMicros: Int = 0,
        layout: SegmentLayoutInfo? = nil
    ) {
        self.conversionID = conversionID
        self.pageIndex = pageIndex
        self.candidates = candidates
        self.hasNext = hasNext
        self.lmApplied = lmApplied
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

/// 殻から見たエンジン。IkariXPC が XPC で実装し、テストはモックで差し替える。
/// `convert` と `page` は同期で待つ（IMK の主スレッドを止めてよい。ADR 0004 §7.5）。
/// それ以外の通知は返事を待たない。
public protocol EngineClient: Sendable {
    /// 版の照合とエンジンの起動の前倒し（`activateServer` で呼ぶ）
    func hello(shellVersion: String, timeout: Duration) throws -> EngineHello
    func beginSession(_ session: SessionInfo)
    func endSession(sessionID: UInt64)
    /// 読みが変わるたびに呼ぶ。返事を待たない
    func prefetch(_ request: ConversionRequest)
    func convert(_ request: ConversionRequest, timeout: Duration) throws -> ConversionPage
    func page(conversionID: UInt64, index: Int, timeout: Duration) throws -> ConversionPage
    /// `candidateIndex` は生成済みの列の中の通し番号（ページをまたぐ）
    func commit(conversionID: UInt64, candidateIndex: Int)
    /// 変換せずに確定した文字列（確定履歴のため）
    func commitText(sessionID: UInt64, reading: String, surface: String)
    func cancel(conversionID: UInt64)
    // ---- 文節と再変換（ADR 0006 §5.2、§8.5）。同期で待つ（待ちの上限は `EngineTimeouts.convertTimeout(文節の読みの文字数)`）----
    /// 文節 `segment` の `index` ページ目。`left` は文節 0〜segment-1 の選択（初めて作るときだけ使われる）
    func segmentPage(
        conversionID: UInt64, layoutRev: UInt32, segment: Int, index: Int, left: [SegmentChoiceValue], timeout: Duration
    ) throws -> SegmentPage
    /// 区切り直し。`delta` は縮める = 負、伸ばす = 正。返事は新しい区切り（`layoutRev` が上がる）
    func resize(
        conversionID: UInt64, layoutRev: UInt32, segment: Int, delta: Int, left: [SegmentChoiceValue], timeout: Duration
    ) throws -> SegmentLayoutInfo
    /// 文節ごとの選択で確定する。通知（返事を待たない）。secure のときは呼ばない
    func commitSegments(conversionID: UInt64, layoutRev: UInt32, choices: [SegmentChoiceValue])
    /// 再変換の要求。読みが決まらなければ `engine(.badRequest)` などで失敗する（殻は何もしない）
    func reconvert(_ request: ReconvertRequest, timeout: Duration) throws -> SegmentLayoutInfo
    /// エンジンに保存して終了するよう頼む。終了は異常として数えない
    func shutdown(reason: EngineShutdownReason)
    /// 管理の要求（ADR 0005 §8.3）。`Envelope` のバイト列をそのまま送り、返事のバイト列をそのまま返す。
    /// `request_id` は実装が付け直す。待ちの上限は `timeout`（`EngineTimeouts.admin` / `.adminTransfer`）。
    /// 殻は種類を見て許可リストで断る（`IkariBridge.RelayPolicy`）。ここでは中身を解釈しない
    func relay(_ request: Data, timeout: Duration) throws -> Data
    /// 絵文字のショートコードの表（ADR 0027 §9.2）。セッションの開始時とデータの版が変わったときに、
    /// 主スレッドの外で（非同期に）頼む。英語のモードの経路では同期で呼ばない。
    /// 契約（B47）: 既定の実装は空の表を返す。`XPCEngineClient` の実装はレーン S
    func emojiTable(timeout: Duration) throws -> EmojiTableInfo
    /// 診断ログの事象（ADR 0034 §2.2）。返事を待たない。secure のセッションでは呼ばない。
    /// 契約（D2）: 既定の実装は何もしない。`XPCEngineClient` で送るのは D4
    func diag(_ event: DiagEventValue)
}

extension EngineClient {
    public func diag(_ event: DiagEventValue) {}
    public func emojiTable(timeout: Duration) throws -> EmojiTableInfo { .empty }
}

/// `EmojiTableReply` の写し（ADR 0027 §9.2）
public struct EmojiTableInfo: Sendable, Equatable {
    /// 名前の昇順
    public var entries: [ShortcodeEntry]
    /// データの版の目印。変わったら表を取り直す
    public var dataLabel: String

    public static let empty = EmojiTableInfo(entries: [], dataLabel: "")

    public init(entries: [ShortcodeEntry], dataLabel: String) {
        self.entries = entries
        self.dataLabel = dataLabel
    }
}

/// `DiagEvent` の写し（ADR 0034 §2.2）。値は数と固定の語だけ。kind・キー・語は engine の許可リスト
/// （`diag_schema.rs`）にあるものだけが書かれる。ユーザーの文字は入れない（NFR-19）
public struct DiagEventValue: Sendable, Equatable {
    public var sessionID: UInt64
    public var kind: String
    public var nums: [String: Int64]
    public var tags: [String: String]

    public init(sessionID: UInt64, kind: String, nums: [String: Int64] = [:], tags: [String: String] = [:]) {
        self.sessionID = sessionID
        self.kind = kind
        self.nums = nums
        self.tags = tags
    }
}
