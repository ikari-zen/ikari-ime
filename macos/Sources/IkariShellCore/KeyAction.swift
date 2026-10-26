// 状態機械の入力（KeyInput）と出力（KeyAction）。契約。
// IMK のコントローラは、イベントを KeyInput に直して渡し、返ってきた KeyAction を順に実行するだけにする。
import Foundation

/// IMK のイベントから作る、AppKit に依存しないキー入力。
public struct KeyInput: Sendable, Equatable {
    public enum Key: Sendable, Equatable {
        /// 文字を生むキー（ローマ字入力。`characters` は修飾キーを適用したあとの文字）
        case character(String)
        /// かな入力用の物理キー（JIS 配列の番号。`IkariJisKey`）
        case jis(UInt8)
        case space, enter, escape, backspace
        /// ←/→（修飾キーの有無を問わない。ADR 0006 §8.1）。idle ではアプリに渡し、合成中は受け止めて何もしない
        case left, right
        /// ↑/↓（`candidate.previous` / `candidate.next`。ADR 0014 §2.4）。idle ではアプリに渡し、合成中は受け止めて何もしない
        case up, down
        /// F1〜F20
        case function(Int)
        case other
    }

    public var key: Key
    public var modifiers: KeyModifiers

    public init(key: Key, modifiers: KeyModifiers = []) {
        self.key = key
        self.modifiers = modifiers
    }
}

/// マーク中のテキスト（下線つきの未確定文字列）。
public struct MarkedText: Sendable, Equatable {
    public var text: String
    /// 変換中は文節全体、合成中は `kana` の部分だけを強調するための範囲（UTF-16 の単位）
    public var highlightedLength: Int
    /// 文節ごとの UTF-16 の長さ（先頭から順。合計は `text` の UTF-16 の長さ）。空なら文節に分けない（合成中、1文節。ADR 0006 §8.3）。
    /// 空でないときは、`focused` の文節だけ太い下線、ほかは細い下線にし、各文節に `markedClauseSegment` を付ける。
    /// このとき `highlightedLength` は使わない
    public var segments: [Int]
    /// フォーカスしている文節の番号。`segments` が空なら nil
    public var focused: Int?
    /// 再変換を始めるときだけ入れる。選択していた範囲（`setMarkedText(replacementRange:)` に渡す。ADR 0006 §9.5）。
    /// 通常は nil（`NSNotFound`）
    public var replacementRange: TextRange?
    /// 下線を付けるか。既定は true。英語のモードのショートコードを打っている途中だけ false（ADR 0027 §9.5、B51）
    public var underlined: Bool

    public init(
        text: String, highlightedLength: Int = 0, segments: [Int] = [], focused: Int? = nil,
        replacementRange: TextRange? = nil, underlined: Bool = true
    ) {
        self.text = text
        self.highlightedLength = highlightedLength
        self.segments = segments
        self.focused = focused
        self.replacementRange = replacementRange
        self.underlined = underlined
    }
}

/// 文書の中の範囲（UTF-16 の単位。`NSRange` と同じ。AppKit に依存させないための型）
public struct TextRange: Sendable, Equatable {
    public var location: Int
    public var length: Int

    public init(location: Int, length: Int) {
        self.location = location
        self.length = length
    }
}

/// 再変換の対象としてコントローラが渡す、選択範囲とその文字列（ADR 0006 §9.2）。
/// コントローラは `reconvert` のキーのときだけ `selectedRange()` と `string(from:actualRange:)` を呼び、
/// 選択がない、取れない、`actualRange != selectedRange` のときは nil を渡す。
/// 長さ、改行、制御文字、周辺テキストの有無による断りは状態機械が判定する
public struct SelectedText: Sendable, Equatable {
    public var range: TextRange
    public var text: String

    public init(range: TextRange, text: String) {
        self.range = range
        self.text = text
    }
}

/// 候補ウィンドウに出す内容（縦一列。1行に 1 候補）。
public struct CandidatePanelModel: Sendable, Equatable {
    /// 表示する候補（番号は 1 から振る。3 件のとき 1〜3、広げたあとは 1〜9）
    public var candidates: [CandidateItem]
    public var selected: Int
    /// 注釈。注釈のデータが入るまで、殻は行を出さない（ADR 0014 §9 Q7）
    public var annotation: String
    /// 位置を決めるときに見込む行の数。このあと広がる、またはページが進み得るなら 9、そうでなければページの件数。
    /// 広げたときの高さの変化で、候補ウィンドウの上下が入れ替わらないようにする（ADR 0014 §2.2）
    public var reservedRows: Int
    /// 候補ウィンドウの先頭の行に出す、変換結果の文節ごとの表記。2 文節以上のときだけ入れる（空なら行を出さない。#292）
    public var clauses: [String]
    /// `clauses` のうちフォーカスしている文節。全体の候補を出している間は nil
    public var focusedClause: Int?

    public init(
        candidates: [CandidateItem], selected: Int, annotation: String, reservedRows: Int? = nil,
        clauses: [String] = [], focusedClause: Int? = nil
    ) {
        self.clauses = clauses
        self.focusedClause = focusedClause
        self.candidates = candidates
        self.selected = selected
        self.annotation = annotation
        self.reservedRows = max(reservedRows ?? candidates.count, candidates.count)
    }
}

/// 状態機械が返す操作。
public enum KeyAction: Sendable, Equatable {
    /// このキーを IME が処理しない（アプリにそのまま渡す）
    case passThrough
    /// 処理した（何も表示を変えない）
    case consume
    case setMarkedText(MarkedText)
    /// テキストを確定してマークを消す
    case commit(String)
    case showCandidates(CandidatePanelModel)
    case moveSelection(CandidatePanelModel)
    case hideCandidates
    /// 元の文字列を、変換中の範囲を置き換える 1 回の `insertText(_, replacementRange: markedRange)` で書き戻す。
    /// 再変換の Esc と、再変換中の合成の Esc で使う。`setMarkedText("")` や `unmarkText` を前後に呼ばない
    /// （Electron の「元に戻す」の履歴に変換中の状態を残しにくくするため。ADR 0006 §16.3）
    /// `replacing` は、いま表示している変換中の文字列の範囲（文書の絶対位置。再変換を始めた選択範囲の先頭と、表示中の長さ）。
    /// 殻は `client.markedRange()` を当てにせず、まずこれを使う（アプリによって markedRange が相対位置や空を返すため）
    case restore(String, replacing: TextRange)
    /// 殻が何も入力していない状態で Enter / Esc を受けた。`client.markedRange()` が有効な範囲を返すなら、
    /// その文字列を `insertText` で確定してキーを消費し、そうでなければキーをアプリに渡す
    /// （取り残された変換中の文字列を守る。意図しない送信を防ぐ。ADR 0006 §16.3）
    case resolveStrayMarkedText
    /// 読みが変わったので、必要なら周辺テキストを取り直してエンジンへ Prefetch を投げる（状態機械が直接投げてもよい）
    case prefetch(reading: String)
}
