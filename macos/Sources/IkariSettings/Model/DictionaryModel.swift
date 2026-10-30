// 「ユーザー辞書」タブのモデル（ADR 0005 §6.2、§8.2）。読み・表記・品詞（簡略化した選択肢）・活用型（動詞のとき）。
// 一覧・追加・削除はエンジンの管理の要求。読みの正規化と品詞の検証はエンジンが行う（明らかな入力漏れは shell-core が弾く）。
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
    /// shell-core を使えない（FFI のないビルド、版のずれ）
    case unavailable

    init(_ problem: Ikari_Settings_V1_DraftProblem) {
        switch problem {
        case .readingEmpty: self = .readingEmpty
        case .surfaceEmpty: self = .surfaceEmpty
        case .posNotChosen: self = .posNotChosen
        case .conjTypeNeeded: self = .conjTypeNeeded
        case .controlCharacters: self = .controlCharacters
        case .unspecified, .UNRECOGNIZED: self = .unavailable
        }
    }

    var message: String {
        switch self {
        case .readingEmpty: "読みを入力してください"
        case .surfaceEmpty: "表記を入力してください"
        case .posNotChosen: "品詞を選んでください"
        case .conjTypeNeeded: "動詞には活用型を選んでください"
        case .controlCharacters: "タブと改行は使えません"
        case .unavailable: "登録の内容を確かめられませんでした。設定アプリを入れ直してください"
        }
    }
}

/// 入力欄の検証、品詞の表示、一覧の並びと検索は shell-core（`settings::admin`。ADR 0041 §2.3）
enum DictionaryRules {
    /// 入力欄を要求の項目にする。問題があれば `DraftProblem`
    static func entry(from draft: DictionaryDraft, options: [PosOption]) -> Result<UserDictEntry, DraftProblem> {
        var request = Ikari_Settings_V1_DictionaryEntryRequest()
        request.draft.reading = draft.reading
        request.draft.surface = draft.surface
        request.draft.posName = draft.posName
        request.draft.conjType = draft.conjType
        request.draft.comment = draft.comment
        request.options = options
        guard case .dictionaryEntry(let result)? = SettingsCore.call(.dictionaryEntry(request)) else {
            return .failure(.unavailable)
        }
        switch result.result {
        case .entry(let entry)?: return .success(entry)
        case .problem(let problem)?: return .failure(DraftProblem(problem))
        case nil: return .failure(.unavailable)
        }
    }

    /// 一覧に出す品詞。`動詞:五段-ラ行` は `動詞（五段-ラ行）`
    static func posDisplay(_ pos: String) -> String {
        var request = Ikari_Settings_V1_PosDisplayRequest()
        request.pos = pos
        return SettingsCore.text(.posDisplay(request)) ?? pos
    }

    /// 検索語を空白で分け、すべてが読み・表記・品詞・コメントのどれかに含まれる項目を残す。
    /// 大文字小文字、全角半角、ひらがなとカタカナの違いは無視する。順は変えない
    static func filter(_ entries: [UserDictEntry], query: String) -> [UserDictEntry] {
        let kept = Set(displayOrder(entries, query: query))
        return entries.indices.filter(kept.contains).map { entries[$0] }
    }

    /// 表示順: 読み、表記の順
    static func sorted(_ entries: [UserDictEntry]) -> [UserDictEntry] {
        displayOrder(entries, query: "").map { entries[$0] }
    }

    /// 表示順に並べ、検索語で絞った添字。shell-core を使えなければ元の順のすべて
    private static func displayOrder(_ entries: [UserDictEntry], query: String) -> [Int] {
        var request = Ikari_Settings_V1_DictionaryListRequest()
        request.entries = entries
        request.query = query
        guard case .dictionaryList(let list)? = SettingsCore.call(.dictionaryList(request)) else {
            return Array(entries.indices)
        }
        return list.indices.map(Int.init).filter(entries.indices.contains)
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
