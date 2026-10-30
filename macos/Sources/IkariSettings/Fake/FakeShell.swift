// 偽の殻とエンジン（ADR 0005 §11.3 の U の「最初に置くもの」）。`BridgeTransport` の殻側の端を受け持ち、
// メモリ上の設定・ユーザー辞書・学習・プロファイルで決まった返事を返す。
//
// 使い道:
//  - 単体テスト（`FileDescriptorTransport.socketPair()` の片方を渡す）
//  - `IkariSettings --fake-bridge`: 本物の殻とエンジンなしで全タブを動かして見る
// 本物の殻（S3）とエンジン（E3）の挙動を写すが、置き換えではない（品詞の検証などは粗い）。
import Foundation
import IkariBridge
import IkariProto
import IkariShellABI
import IkariShellCore

/// 小さな排他つきの箱（テストが状態を覗くため）
final class Locked<Value>: @unchecked Sendable {
    private let lock = NSLock()
    private var value: Value

    init(_ value: Value) { self.value = value }

    func withLock<Result>(_ body: (inout Value) throws -> Result) rethrows -> Result {
        lock.lock()
        defer { lock.unlock() }
        return try body(&value)
    }

    var snapshot: Value { withLock { $0 } }
}

/// 偽のエンジンと殻の状態
struct FakeState: Sendable {
    typealias UserEntry = Ikari_Engine_V1_UserDictEntry

    static let defaultSettings: [String: String] = [
        "input.method": "romaji",
        "keymap.preset": "macos",
        "paging.wrap": "true",
        "paging.size": "3",
        "engine.lm": "on",
        "engine.idle_unload_secs": "300",
        "engine.lm_deadline_ms": "300",
        "engine.convert_wait_ms": "30",
        "context.shared_exclude": SettingsValues.defaults.contextSharedExclude.joined(separator: ","),
        "emoji.shortcode": "off",
        "emoji.small_letters": "off",
        "postal.japanese": "on",
        "postal.english": "off",
        "postal.te_prefix": "off",
        "unicode.notation": "off",
        "date.weekday_dates": "off",
        "candidate.hide_when_confident": "on",
        "emoji.shortcode_exclude": SettingsValues.defaults.emojiShortcodeExclude.joined(separator: ","),
    ]
    static let posOptions: [(name: String, needsConjType: Bool)] = [
        ("名詞", false), ("サ変名詞", false), ("形状詞", false), ("固有名詞", false), ("組織", false),
        ("人名", false), ("姓", false), ("名", false), ("地名", false), ("副詞", false), ("感動詞", false),
        ("短縮よみ", false), ("動詞", true), ("形容詞", false),
    ]
    static let verbConjTypes = [
        "五段-ラ行", "五段-カ行", "五段-ガ行", "五段-サ行", "五段-タ行", "五段-ナ行", "五段-バ行", "五段-マ行",
        "五段-ワア行", "上一段-ア行", "下一段-ア行", "サ行変格", "カ行変格",
    ]
    static let domains: [(id: String, name: String)] = [
        ("it", "IT・ソフトウェア"), ("infra", "インフラ・クラウド"), ("novel", "小説"), ("business", "ビジネス"),
    ]
    static let exportHeader = "#ikari-export\tschema=1"
    static let verbSeparator = ":"

    var settings = FakeState.defaultSettings
    var userDict: [UserEntry] = []
    var usageRows: UInt32 = 1234
    var initialRows: UInt32 = 5678
    let maxRows: UInt32 = 200_000
    var profile: [String: Ikari_Engine_V1_ProfileWeightKind] = ["it": .profileWeightOften]
    var backupFile = ""
    /// 受けた要求の種類（テストの確認用。中身は入れない）
    var requestLog: [String] = []
    /// 次の管理の要求に返す `ErrorReply`（1 回で戻る）
    var failNextEngineRequest: Ikari_Engine_V1_ErrorCode?
    /// 最後にインポートで適用したバイト列
    var lastImportedContent = Data()
    /// 殻が `ENGINE_UNAVAILABLE` を返す（エンジンが落ちている）
    var engineUnavailable = false
}

final class FakeShell: @unchecked Sendable {
    let state: Locked<FakeState>
    private let transport: any BridgeTransport
    private let shellVersion: String
    private let acceptedBridgeVersion: UInt32
    private var thread: Thread?

    /// `acceptedBridgeVersion` を変えると、版の不一致の殻を演じられる
    init(
        transport: any BridgeTransport,
        state: FakeState = FakeState(),
        shellVersion: String = "0.0.0-fake",
        acceptedBridgeVersion: UInt32 = BridgeProtocol.version
    ) {
        self.transport = transport
        self.state = Locked(state)
        self.shellVersion = shellVersion
        self.acceptedBridgeVersion = acceptedBridgeVersion
    }

    func start() {
        let thread = Thread { [self] in serve() }
        thread.name = "ikari.settings.fake-shell"
        self.thread = thread
        thread.start()
    }

    /// 殻が止まった（`killall ikari`）ことにする
    func stop() {
        transport.close()
    }

    func sendEvent(_ kind: Ikari_Bridge_V1_Event.OneOf_Kind) {
        var event = Ikari_Bridge_V1_Event()
        event.kind = kind
        var frame = Frame()
        frame.event = event
        try? transport.send(frame)
    }

    func notifySettingsChanged() { sendEvent(.settingsChanged(Ikari_Bridge_V1_SettingsChanged())) }

    func notifyOpenTab(_ tab: SettingsTab) {
        var open = Ikari_Bridge_V1_OpenTab()
        open.tab = tab.rawValue
        sendEvent(.openTab(open))
    }

    // MARK: 受け付け

    private func serve() {
        while true {
            guard let frame = try? transport.receive() else { return }
            let reply = handle(frame)
            do { try transport.send(reply) } catch { return }
        }
    }

    private func handle(_ frame: Frame) -> Frame {
        let id = frame.requestID
        switch frame.body {
        case .hello(let hello)?:
            if hello.bridgeVersion != acceptedBridgeVersion { return .error(requestID: id, code: .versionMismatch) }
            return BridgeHandshake.reply(to: hello, requestID: id, shellVersion: shellVersion)
        case .getSettings?:
            return settingsFrame(id: id)
        case .updateSettings(let update)?:
            return state.withLock { applyUpdate(update, id: id, state: &$0) }
        case .engine(let envelope)?:
            if let rejection = RelayPolicy.rejection(for: frame) { return rejection }
            return state.withLock { engineFrame(for: envelope, id: id, state: &$0) }
        default:
            return .error(requestID: id, code: .notAllowed)
        }
    }

    // MARK: 設定

    private func settingsFrame(id: UInt64) -> Frame {
        let values = state.withLock { $0.settings }
        return Self.settingsFrame(values, id: id)
    }

    private static func settingsFrame(_ values: [String: String], id: UInt64) -> Frame {
        var snapshot = Ikari_Bridge_V1_SettingsSnapshot()
        snapshot.effective = values.sorted { $0.key < $1.key }.map { key, value in
            var entry = SettingEntry()
            entry.key = key
            entry.value = value
            return entry
        }
        var frame = Frame()
        frame.requestID = id
        frame.settings = snapshot
        return frame
    }

    private func applyUpdate(_ update: Ikari_Bridge_V1_UpdateSettings, id: UInt64, state: inout FakeState) -> Frame {
        for entry in update.set where !Self.isValid(key: entry.key, value: entry.value) {
            return .error(requestID: id, code: .invalidSetting, key: entry.key)
        }
        for key in update.reset where !Self.isKnown(key: key) {
            return .error(requestID: id, code: .invalidSetting, key: key)
        }
        let enginePrefix = "engine."
        let before = state.settings.filter { $0.key.hasPrefix(enginePrefix) }
        for entry in update.set { state.settings[entry.key] = entry.value }
        for key in update.reset {
            if let fallback = FakeState.defaultSettings[key] {
                state.settings[key] = fallback
            } else {
                state.settings[key] = nil
            }
        }
        if state.settings.filter({ $0.key.hasPrefix(enginePrefix) }) != before {
            state.requestLog.append("engine-restart")
        }
        return Self.settingsFrame(state.settings, id: id)
    }

    private static func isKnown(key: String) -> Bool {
        if FakeState.defaultSettings[key] != nil { return true }
        return key.hasPrefix("keymap.") && KeymapAction(rawValue: String(key.dropFirst("keymap.".count))) != nil
    }

    private static func isValid(key: String, value: String) -> Bool {
        guard isKnown(key: key) else { return false }
        switch key {
        case "engine.lm": return value == "on" || value == "off"
        case "engine.idle_unload_secs", "engine.lm_deadline_ms", "engine.convert_wait_ms":
            return Int(value).map { $0 >= 0 } ?? false
        default:
            // 殻（shell-core の `settings::validate`）が付ける警告が 0 のときだけ有効
            return ShellSettings.validate("\(key)\t\(value)")?.issues.isEmpty ?? false
        }
    }

    // MARK: エンジン

    private func engineFrame(for envelope: EngineEnvelope, id: UInt64, state: inout FakeState) -> Frame {
        func wrap(_ build: (inout EngineEnvelope) -> Void) -> Frame {
            var out = EngineEnvelope()
            out.schemaVersion = envelope.schemaVersion
            out.requestID = envelope.requestID
            build(&out)
            var frame = Frame()
            frame.requestID = id
            frame.engine = out
            return frame
        }
        if state.engineUnavailable { return .error(requestID: id, code: .engineUnavailable) }
        state.requestLog.append(Self.kind(of: envelope.body))
        if let code = state.failNextEngineRequest {
            state.failNextEngineRequest = nil
            return wrap {
                var error = Ikari_Engine_V1_ErrorReply()
                error.code = code
                $0.error = error
            }
        }
        switch envelope.body {
        case .hello?:
            return wrap {
                var reply = Ikari_Engine_V1_HelloReply()
                reply.engineVersion = shellVersion
                reply.dictLabel = "SudachiDict Core 20260723 (fake)"
                reply.lm = state.settings["engine.lm"] == "off" ? .lmDisabled : .lmReady
                $0.helloReply = reply
            }
        case .listUserDict?:
            return wrap { $0.userDictListReply = userDictReply(state) }
        case .addUserDict(let add)?:
            let issue = Self.validate(add.entry)
            if issue == nil {
                state.userDict.removeAll { $0.reading == add.entry.reading && $0.surface == add.entry.surface }
                state.userDict.append(add.entry)
            }
            return wrap { $0.userDictListReply = userDictReply(state, issue: issue) }
        case .removeUserDict(let remove)?:
            let before = state.userDict.count
            state.userDict.removeAll { $0.reading == remove.reading && $0.surface == remove.surface }
            let issue = state.userDict.count == before ? Self.issue(.notFound) : nil
            return wrap { $0.userDictListReply = userDictReply(state, issue: issue) }
        case .learningStatus?:
            return wrap { $0.learningStatusReply = learningReply(state) }
        case .resetLearning(let reset)?:
            switch reset.layer {
            case .usage: state.usageRows = 0
            case .initial: state.initialRows = 0
            case .all:
                state.usageRows = 0
                state.initialRows = 0
            default: break
            }
            state.backupFile = "before-reset-20260101T000000Z.tsv"
            return wrap { $0.learningStatusReply = learningReply(state) }
        case .exportData?:
            let content = Self.exportContent(state)
            return wrap {
                var reply = Ikari_Engine_V1_ExportReply()
                reply.content = content
                reply.schema = 1
                reply.counts = Self.counts(of: state)
                $0.exportReply = reply
            }
        case .importData(let request)?:
            return wrap { $0.importReply = importReply(for: request, state: &state) }
        case .getProfile?:
            return wrap { $0.profileReply = profileReply(state) }
        case .setProfile(let set)?:
            state.profile = [:]
            for domain in set.domains where domain.weight != .profileWeightOff {
                state.profile[domain.id] = domain.weight
            }
            return wrap { $0.profileReply = profileReply(state) }
        default:
            return wrap {
                var error = Ikari_Engine_V1_ErrorReply()
                error.code = .badRequest
                $0.error = error
            }
        }
    }

    private static func kind(of body: EngineBody?) -> String {
        switch body {
        case .hello?: "hello"
        case .listUserDict?: "list_user_dict"
        case .addUserDict?: "add_user_dict"
        case .removeUserDict?: "remove_user_dict"
        case .learningStatus?: "learning_status"
        case .resetLearning?: "reset_learning"
        case .exportData?: "export_data"
        case .importData(let request)?: request.dryRun ? "import_data_dry_run" : "import_data"
        case .getProfile?: "get_profile"
        case .setProfile?: "set_profile"
        default: "other"
        }
    }

    private static func issue(_ kind: Ikari_Engine_V1_DataIssueKind, detail: String = "") -> Ikari_Engine_V1_DataIssue {
        var issue = Ikari_Engine_V1_DataIssue()
        issue.kind = kind
        issue.detail = detail
        return issue
    }

    private func userDictReply(
        _ state: FakeState, issue: Ikari_Engine_V1_DataIssue? = nil
    ) -> Ikari_Engine_V1_UserDictListReply {
        var reply = Ikari_Engine_V1_UserDictListReply()
        reply.entries = state.userDict
        reply.posOptions = FakeState.posOptions.map { option in
            var out = Ikari_Engine_V1_PosOption()
            out.name = option.name
            out.needsConjType = option.needsConjType
            return out
        }
        reply.verbConjTypes = FakeState.verbConjTypes
        if let issue { reply.issue = issue }
        return reply
    }

    /// 粗い検証: 読みはひらがな（と長音）、表記は空でない、品詞は簡略名（動詞は `動詞:<活用型>`）
    private static func validate(_ entry: FakeState.UserEntry) -> Ikari_Engine_V1_DataIssue? {
        let hiragana: ClosedRange<UInt32> = 0x3041...0x3096
        let prolonged: UInt32 = 0x30FC
        let reading = entry.reading.unicodeScalars
        guard !reading.isEmpty, reading.allSatisfy({ hiragana.contains($0.value) || $0.value == prolonged }) else {
            return issue(.invalidEntry, detail: "読みはひらがな（とー）にしてください")
        }
        guard !entry.surface.isEmpty else { return issue(.invalidEntry, detail: "表記が空です") }
        let head = entry.pos.split(separator: Character(FakeState.verbSeparator), maxSplits: 1).first.map(String.init) ?? ""
        guard let option = FakeState.posOptions.first(where: { $0.name == head }) else {
            return issue(.invalidEntry, detail: "未知の品詞です")
        }
        let hasType = entry.pos.contains(FakeState.verbSeparator)
        if option.needsConjType != hasType { return issue(.invalidEntry, detail: "活用型の指定が合いません") }
        return nil
    }

    private func learningReply(_ state: FakeState) -> Ikari_Engine_V1_LearningStatusReply {
        var reply = Ikari_Engine_V1_LearningStatusReply()
        reply.usageRows = state.usageRows
        reply.initialRows = state.initialRows
        reply.maxRows = state.maxRows
        reply.backupFile = state.backupFile
        return reply
    }

    private func profileReply(_ state: FakeState) -> Ikari_Engine_V1_ProfileReply {
        var reply = Ikari_Engine_V1_ProfileReply()
        reply.domains = FakeState.domains.map { domain in
            var out = Ikari_Engine_V1_ProfileDomainState()
            out.id = domain.id
            out.name = domain.name
            out.weight = state.profile[domain.id] ?? .profileWeightOff
            return out
        }
        return reply
    }

    // MARK: エクスポートとインポート

    private static func counts(of state: FakeState) -> Ikari_Engine_V1_DataCounts {
        var counts = Ikari_Engine_V1_DataCounts()
        counts.profileDomains = UInt32(state.profile.count)
        counts.userDictEntries = UInt32(state.userDict.count)
        counts.learningRows = state.usageRows + state.initialRows
        return counts
    }

    /// 偽の書き出し: ヘッダの下に、件数を数えられる行だけを置く（原文なし）
    private static func exportContent(_ state: FakeState) -> Data {
        var lines = [FakeState.exportHeader]
        for (id, weight) in state.profile.sorted(by: { $0.key < $1.key }) {
            lines.append("profile\t\(id)\t\(weight.rawValue)")
        }
        for entry in state.userDict { lines.append("dict\t\(entry.reading)\t\(entry.surface)\t\(entry.pos)\t\(entry.comment)") }
        lines.append("usage\t\(state.usageRows)")
        lines.append("initial\t\(state.initialRows)")
        return Data((lines.joined(separator: "\n") + "\n").utf8)
    }

    private func importReply(for request: Ikari_Engine_V1_ImportData, state: inout FakeState) -> Ikari_Engine_V1_ImportReply {
        var reply = Ikari_Engine_V1_ImportReply()
        guard let text = String(data: request.content, encoding: .utf8) else {
            reply.issue = Self.issue(.parse)
            return reply
        }
        let lines = text.split(separator: "\n", omittingEmptySubsequences: false).map(String.init)
        guard let header = lines.first, header.hasPrefix("#ikari-export\t") else {
            reply.issue = Self.issue(.notIkariExport)
            return reply
        }
        guard header == FakeState.exportHeader else {
            var issue = Self.issue(.schemaUnsupported)
            issue.section = "header"
            issue.foundSchema = UInt32(header.split(separator: "=").last.flatMap { Int($0) } ?? 0)
            issue.supportedSchema = 1
            reply.issue = issue
            return reply
        }
        var profile: [String: Ikari_Engine_V1_ProfileWeightKind] = [:]
        var dict: [FakeState.UserEntry] = []
        var usage: UInt32 = 0
        var initial: UInt32 = 0
        for (offset, line) in lines.dropFirst().enumerated() where !line.isEmpty {
            let columns = line.split(separator: "\t", omittingEmptySubsequences: false).map(String.init)
            switch columns.first {
            case "profile" where columns.count == 3:
                profile[columns[1]] = Ikari_Engine_V1_ProfileWeightKind(rawValue: Int(columns[2]) ?? 0) ?? .profileWeightOff
            case "dict" where columns.count == 5:
                var entry = FakeState.UserEntry()
                (entry.reading, entry.surface, entry.pos, entry.comment) = (columns[1], columns[2], columns[3], columns[4])
                dict.append(entry)
            case "usage" where columns.count == 2: usage = UInt32(columns[1]) ?? 0
            case "initial" where columns.count == 2: initial = UInt32(columns[1]) ?? 0
            default:
                var issue = Self.issue(.parse)
                issue.line = UInt32(offset + 2)
                reply.issue = issue
                return reply
            }
        }
        var counts = Ikari_Engine_V1_DataCounts()
        counts.profileDomains = UInt32(profile.count)
        counts.userDictEntries = UInt32(dict.count)
        counts.learningRows = usage + initial
        reply.counts = counts
        guard !request.dryRun else { return reply }
        state.profile = profile
        state.userDict = dict
        state.usageRows = usage
        state.initialRows = initial
        state.backupFile = "before-import-20260101T000000Z.tsv"
        state.lastImportedContent = request.content
        reply.applied = true
        reply.backupFile = state.backupFile
        return reply
    }
}
