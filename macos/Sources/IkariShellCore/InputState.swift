// 入力の状態（ADR 0004 §6.3）。契約。
import Foundation

/// 変換に入る前の状態。読みに戻る（Esc、BackSpace）ときの行き先（ADR 0006 §8.5、§9.5）
public enum ConversionOrigin: Sendable, Equatable {
    case composing
    case ascii
    /// 確定済みの文字列の再変換。`original` は選択していた文字列、`replacing` はその範囲（`NSNotFound` ではなく実際の範囲）。
    /// Esc は `original` を書き戻す（`KeyAction.restore`）
    case reconversion(original: String, replacing: TextRange)
}

/// 変換中の 1 つの文節（ADR 0006 §8.5）。ページの往復と先頭への折り返しは、文節ごとに取得済みのページを使う
public struct ConvertingSegment: Sendable, Equatable {
    public var reading: String
    /// 候補の 0 番（最初に表示した表記）。区切りの表示と、「何も変えていない」判定に使う
    public var surface: String
    /// 取得済みのページ（古い順）
    public var pages: [ConversionPage]
    /// 現在のページの番号（`pages` の添字）
    public var pageIndex: Int
    /// 現在のページの中の選択位置
    public var selected: Int
    /// 字種変換で殻が作った表記。あれば `selected` より優先して表示し、`SegmentChoiceValue.literal` として送る
    public var literal: String?
    /// 最初のページを全部見せている（3 件目からさらに進んで広げた。ADR 0014 §2.3 (c)）。文節ごとに持つ
    public var expanded: Bool

    public init(
        reading: String, surface: String, pages: [ConversionPage] = [], pageIndex: Int = 0, selected: Int = 0,
        literal: String? = nil, expanded: Bool = false
    ) {
        self.reading = reading
        self.surface = surface
        self.pages = pages
        self.pageIndex = pageIndex
        self.selected = selected
        self.literal = literal
        self.expanded = expanded
    }
}

/// 変換中の状態。候補はエンジンが返したページを1つずつ持つ。
///
/// 文節（ADR 0006）: `segments` が空のときは 1 文節で、下の平らなフィールドがその全部。
/// `segments` が空でないときは、文節ごとのデータは `segments` が持ち、平らなフィールド
/// （`reading` 以外の `conversionID` / `pageIndex` / `candidates` / `hasNext` / `selected` / `candidateOffset`）は
/// **フォーカスしている文節の現在のページの表示**を写したもの（状態機械が同期を保つ。候補ウィンドウと `MarkedText` はこれから作る）。
/// `reading` は読み全体。
public struct ConvertingState: Sendable, Equatable {
    public var reading: String
    /// エンジンの `conversion_id`。フォールバックの候補のときは 0
    public var conversionID: UInt64
    public var pageIndex: Int
    /// 現在のページの候補
    public var candidates: [CandidateItem]
    public var hasNext: Bool
    /// 現在のページの中の選択位置（0 始まり）
    public var selected: Int
    /// 生成済みの列の中の通し番号に直すための、現在のページより前の候補の数
    public var candidateOffset: Int
    /// エンジンなし、またはタイムアウトで殻だけが作った候補（ひらがな、カタカナ、半角カタカナ）
    public var isFallback: Bool
    /// 文節の列。空なら 1 文節（上の説明）
    public var segments: [ConvertingSegment]
    /// フォーカスしている文節の番号（`segments` が空なら 0）
    public var focused: Int
    /// 区切りの版。エンジンの `SegmentLayout.layout_rev`（区切り直しで上がる）
    public var layoutRev: UInt32
    public var origin: ConversionOrigin
    /// 全体の候補を出している間（最初の Space のあと）の、エンジンが返した 2 文節以上の区切り（ADR 0006 §16.6）。
    /// 非 nil の間は `segments` が読み全体の 1 文節で、候補・選択・確定は今までの変換と同じ（`page` / `commit`）。
    /// 区切りは表示（`markedClauseSegment`）にだけ使い、文節の操作のキーで文節のモードに入るとき `segments` に展開して nil にする
    public var wholeLayout: SegmentLayoutInfo?

    public init(
        reading: String,
        conversionID: UInt64,
        pageIndex: Int,
        candidates: [CandidateItem],
        hasNext: Bool,
        selected: Int = 0,
        candidateOffset: Int = 0,
        isFallback: Bool = false,
        segments: [ConvertingSegment] = [],
        focused: Int = 0,
        layoutRev: UInt32 = 0,
        origin: ConversionOrigin = .composing,
        wholeLayout: SegmentLayoutInfo? = nil
    ) {
        self.reading = reading
        self.conversionID = conversionID
        self.pageIndex = pageIndex
        self.candidates = candidates
        self.hasNext = hasNext
        self.selected = selected
        self.candidateOffset = candidateOffset
        self.isFallback = isFallback
        self.segments = segments
        self.focused = focused
        self.layoutRev = layoutRev
        self.origin = origin
        self.wholeLayout = wholeLayout
    }
}

public enum InputState: Sendable, Equatable {
    case idle
    /// `kana` は確定したかな、`pending` は未確定のローマ字（`n` など）。表示は `kana + pending`
    case composing(kana: String, pending: String)
    case converting(ConvertingState)
    /// Shift＋英字の一時英数。英字を ASCII のまま読みに積む（「API」「Kubernetes」）
    case asciiRun(text: String)
    /// ショートコード（ADR 0027 §3.3）を打っている途中。変換中の文字列は `:` + 名前。
    /// 契約（B47）: ケースだけ。`InputStateMachine` はまだこの状態に入らない（配線はレーン S）
    case shortcode(ShortcodeTyping)
    /// ショートコードの候補を出している（ADR 0027 §3.4）。変換中の文字列は選んでいる候補
    case shortcodeChoice(ShortcodeChoosing)
}
