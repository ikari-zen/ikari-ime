// 「ユーザー辞書」タブのモデル（ADR 0005 §6.2、§8.2）。読み・表記・品詞（簡略化した選択肢）・活用型（動詞のとき）。
// 一覧・追加・削除はエンジンの管理の要求。読みの正規化と品詞の検証はエンジンが行う（ここは明らかな入力漏れだけ弾く）。
import Foundation
import IkariProto
import Observation

typealias UserDictEntry = Ikari_Engine_V1_UserDictEntry
typealias PosOption = Ikari_Engine_V1_PosOption

/// 追加の入力欄
struct DictionaryDraft: Equatable {
    var reading = ""
    var surface = ""
    /// 簡略品詞の名前（`PosOption.name`）
    var posName = ""
    /// 活用型。`PosOption.needs_conj_type` のときだけ使う
    var conjType = ""
    var comment = ""
}

enum DraftProblem: Error, Equatable {
    case readingEmpty
    case surfaceEmpty
    case posNotChosen
    case conjTypeNeeded
    /// 辞書は TSV なので、タブと改行は入れられない（エンジンも拒む）
    case controlCharacters

    var message: String {
        switch self {
        case .readingEmpty: "読みを入力してください"
        case .surfaceEmpty: "表記を入力してください"
        case .posNotChosen: "品詞を選んでください"
        case .conjTypeNeeded: "動詞には活用型を選んでください"
        case .controlCharacters: "タブと改行は使えません"
        }
    }
}

enum DictionaryRules {
    /// 簡略品詞と活用型をつなぐ文字（`動詞:五段-ラ行`。core の `VERB_TYPE_SEPARATOR`）
    static let conjSeparator = ":"

    /// 入力欄を要求の項目にする。問題があれば `DraftProblem`
    static func entry(from draft: DictionaryDraft, options: [PosOption]) -> Result<UserDictEntry, DraftProblem> {
        let reading = draft.reading.trimmingCharacters(in: .whitespacesAndNewlines)
        let surface = draft.surface.trimmingCharacters(in: .whitespacesAndNewlines)
        let comment = draft.comment.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !reading.isEmpty else { return .failure(.readingEmpty) }
        guard !surface.isEmpty else { return .failure(.surfaceEmpty) }
        guard let option = options.first(where: { $0.name == draft.posName }) else { return .failure(.posNotChosen) }
        if [reading, surface, comment].contains(where: hasControlCharacters) { return .failure(.controlCharacters) }
        var pos = option.name
        if option.needsConjType {
            guard !draft.conjType.isEmpty else { return .failure(.conjTypeNeeded) }
            pos += conjSeparator + draft.conjType
        }
        var entry = UserDictEntry()
        entry.reading = reading
        entry.surface = surface
        entry.pos = pos
        entry.comment = comment
        return .success(entry)
    }

    private static func hasControlCharacters(_ text: String) -> Bool {
        text.unicodeScalars.contains { $0 == "\t" || $0 == "\n" || $0 == "\r" }
    }

    /// 一覧に出す品詞。`動詞:五段-ラ行` は `動詞（五段-ラ行）`
    static func posDisplay(_ pos: String) -> String {
        guard let separator = pos.firstIndex(of: Character(conjSeparator)) else { return pos }
        return "\(pos[..<separator])（\(pos[pos.index(after: separator)...])）"
    }

    /// 検索語を空白で分け、すべてが読み・表記・品詞・コメントのどれかに含まれる項目を残す。
    /// 大文字小文字、全角半角、ひらがなとカタカナの違いは無視する
    static func filter(_ entries: [UserDictEntry], query: String) -> [UserDictEntry] {
        let terms = query.split(whereSeparator: \.isWhitespace).map { fold(String($0)) }
        guard !terms.isEmpty else { return entries }
        return entries.filter { entry in
            let fields = [entry.reading, entry.surface, entry.pos, entry.comment].map(fold)
            return terms.allSatisfy { term in fields.contains { $0.contains(term) } }
        }
    }

    private static func fold(_ text: String) -> String {
        let hiragana = text.applyingTransform(.hiraganaToKatakana, reverse: true) ?? text
        return hiragana.folding(options: [.caseInsensitive, .widthInsensitive], locale: nil)
    }

    /// 表示順: 読み、表記の順
    static func sorted(_ entries: [UserDictEntry]) -> [UserDictEntry] {
        entries.sorted { ($0.reading, $0.surface) < ($1.reading, $1.surface) }
    }
}

@MainActor @Observable
final class DictionaryModel {
    private(set) var entries: [UserDictEntry] = []
    private(set) var posOptions: [PosOption] = []
    private(set) var verbConjTypes: [String] = []
    private(set) var isLoaded = false
    private(set) var isBusy = false
    /// 追加・削除・読み込みの失敗（画面に出す文言）
    private(set) var message: String?
    /// 追加できたときの確認の文言
    private(set) var confirmation: String?
    var query = ""
    var draft = DictionaryDraft()

    private let bridge: any SettingsBridge

    init(bridge: any SettingsBridge) {
        self.bridge = bridge
    }

    var visibleEntries: [UserDictEntry] {
        DictionaryRules.filter(entries, query: query)
    }

    var selectedOption: PosOption? {
        posOptions.first { $0.name == draft.posName }
    }

    var needsConjType: Bool { selectedOption?.needsConjType ?? false }

    /// 追加できる状態か（ボタンの有効・無効）
    var canAdd: Bool {
        if case .success = DictionaryRules.entry(from: draft, options: posOptions) { return !isBusy }
        return false
    }

    func load() async {
        isBusy = true
        defer { isBusy = false }
        do {
            let reply = try await listReply(.listUserDict(Ikari_Engine_V1_UserDictList()))
            apply(reply)
            message = nil
        } catch {
            message = Messages.describe(error)
        }
    }

    /// 入力欄から追加する。成功したら読み・表記・コメントを空にする（品詞は残す）
    @discardableResult
    func add() async -> Bool {
        confirmation = nil
        let entry: UserDictEntry
        switch DictionaryRules.entry(from: draft, options: posOptions) {
        case .failure(let problem):
            message = problem.message
            return false
        case .success(let value):
            entry = value
        }
        isBusy = true
        defer { isBusy = false }
        do {
            var request = Ikari_Engine_V1_UserDictAdd()
            request.entry = entry
            let reply = try await listReply(.addUserDict(request))
            apply(reply)
            if Messages.hasIssue(reply.issue) {
                message = Messages.describe(reply.issue)
                return false
            }
            message = nil
            confirmation = "「\(entry.surface)」を登録しました"
            draft.reading = ""
            draft.surface = ""
            draft.comment = ""
            return true
        } catch {
            message = Messages.describe(error)
            return false
        }
    }

    func remove(_ entry: UserDictEntry) async {
        confirmation = nil
        isBusy = true
        defer { isBusy = false }
        do {
            var request = Ikari_Engine_V1_UserDictRemove()
            request.reading = entry.reading
            request.surface = entry.surface
            let reply = try await listReply(.removeUserDict(request))
            apply(reply)
            message = Messages.hasIssue(reply.issue) ? Messages.describe(reply.issue) : nil
        } catch {
            message = Messages.describe(error)
        }
    }

    func dismissMessage() { message = nil }

    private func listReply(_ body: EngineBody) async throws -> Ikari_Engine_V1_UserDictListReply {
        guard case .userDictListReply(let reply) = try await bridge.engine(body) else {
            throw BridgeClientError.unexpectedReply
        }
        return reply
    }

    private func apply(_ reply: Ikari_Engine_V1_UserDictListReply) {
        entries = DictionaryRules.sorted(reply.entries)
        posOptions = reply.posOptions
        verbConjTypes = reply.verbConjTypes
        isLoaded = true
        if selectedOption == nil { draft.posName = posOptions.first?.name ?? "" }
        normalizeConjType()
    }

    /// 品詞を選ぶ。動詞のときは活用型の既定（先頭）を入れ、動詞でなければ空にする
    func selectPos(_ name: String) {
        draft.posName = name
        normalizeConjType()
    }

    private func normalizeConjType() {
        if !needsConjType {
            draft.conjType = ""
        } else if !verbConjTypes.contains(draft.conjType) {
            draft.conjType = verbConjTypes.first ?? ""
        }
    }
}
