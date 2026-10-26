// 管理の E2E（ADR 0005 §10.1「E2E」）。XPC の `relay` で、管理のメッセージ（engine.proto の 40〜53）をエンジンへ送る。
//
// **本物のユーザーデータに触れる**ので、書き込むのは見張りの語（`いかりすもーく` → `IKARI_SMOKE`）の追加と削除だけ。
// 学習のリセットと、インポートの実行（`dry_run = false`）はしない。インポートは `dry_run` だけ。
// 途中で失敗しても、見張りの語は最後に必ず消す。
import Foundation
import IkariProto
import IkariShellCore

enum AdminSmoke {
    static let watchReading = "いかりすもーく"
    static let watchSurface = "IKARI_SMOKE"
    static let watchPos = "名詞"
    static let watchComment = "ikari-smoke の見張りの語（消えていなければ削除してよい）"
    static let sessionID: UInt64 = 1
    static let requestID: UInt64 = 1
    static let schemaVersion: UInt32 = 1
    static let exportHeaderPrefix = "#ikari-export"
    static let exportSchema: UInt32 = 1
    static let garbageImport = Data("not an ikari export\n".utf8)
}

typealias AdminEnvelope = Ikari_Engine_V1_Envelope
typealias AdminBody = AdminEnvelope.OneOf_Body

enum AdminSmokeError: Error, CustomStringConvertible {
    case engine(Ikari_Engine_V1_ErrorCode)
    case unexpectedReply(String)
    case invalidReply

    var description: String {
        switch self {
        case .engine(let code): "engine error \(code)"
        case .unexpectedReply(let expected): "unexpected reply (expected \(expected))"
        case .invalidReply: "reply is not an Envelope"
        }
    }
}

/// 管理の要求を送って、返事の本体を返す（`ErrorReply` は投げる）
func relayAdmin(_ client: any EngineClient, _ body: AdminBody, timeout: Duration) throws -> AdminBody {
    var envelope = AdminEnvelope()
    envelope.schemaVersion = AdminSmoke.schemaVersion
    envelope.requestID = AdminSmoke.requestID
    envelope.body = body
    let reply = try client.relay(try envelope.serializedData(), timeout: timeout)
    guard let decoded = try? AdminEnvelope(serializedBytes: reply), let answer = decoded.body else {
        throw AdminSmokeError.invalidReply
    }
    if case .error(let error) = answer { throw AdminSmokeError.engine(error.code) }
    return answer
}

struct AdminE2E {
    let client: any EngineClient
    let report: Report
    let convertContext: SurroundingText
    let convertTimeout: Duration

    func run() {
        // 前の実行が途中で止まって残った見張りの語を先に消す（失敗しても続ける）
        _ = try? removeWatchWord()
        defer { _ = try? removeWatchWord() }

        userDictionaryRoundTrip()
        learningStatus()
        profile()
        exportAndDryRunImport()
    }

    // MARK: ユーザー辞書

    private func userDictionaryRoundTrip() {
        let name = "admin-user-dict"
        do {
            let initial = try listUserDict()
            guard initial.posOptions.contains(where: { $0.name == AdminSmoke.watchPos }) else {
                report.fail(name, "pos options do not contain \(AdminSmoke.watchPos)")
                return
            }
            guard !initial.entries.contains(where: { $0.surface == AdminSmoke.watchSurface }) else {
                report.fail(name, "the watch word is still registered")
                return
            }

            var entry = Ikari_Engine_V1_UserDictEntry()
            entry.reading = AdminSmoke.watchReading
            entry.surface = AdminSmoke.watchSurface
            entry.pos = AdminSmoke.watchPos
            entry.comment = AdminSmoke.watchComment
            var add = Ikari_Engine_V1_UserDictAdd()
            add.entry = entry
            let added = try userDictReply(try relayAdmin(client, .addUserDict(add), timeout: EngineTimeouts.admin))
            guard added.issue.kind == .dataIssueNone,
                added.entries.contains(where: { $0.reading == entry.reading && $0.surface == entry.surface })
            else {
                report.fail(name, "add: issue=\(added.issue.kind) entries=\(added.entries.count)")
                return
            }
            report.pass("admin-add", "entries=\(added.entries.count)")

            // 1 秒の監視を待たず、その場で変換に出る（ADR 0005 §8.3）
            let surfaces = try convertSurfaces()
            if surfaces.contains(AdminSmoke.watchSurface) {
                report.pass("admin-add-converts", "candidates=\(surfaces.count)")
            } else {
                report.fail("admin-add-converts", "\(AdminSmoke.watchSurface) is not among \(surfaces.count) candidates")
            }

            let removed = try removeWatchWord()
            if removed.issue.kind == .dataIssueNone,
                !removed.entries.contains(where: { $0.surface == AdminSmoke.watchSurface })
            {
                report.pass("admin-remove", "entries=\(removed.entries.count)")
            } else {
                report.fail("admin-remove", "issue=\(removed.issue.kind)")
            }
            let after = try convertSurfaces()
            if after.contains(AdminSmoke.watchSurface) {
                report.fail("admin-remove-converts", "\(AdminSmoke.watchSurface) still converts")
            } else {
                report.pass("admin-remove-converts")
            }
        } catch {
            report.fail(name, "\(error)")
        }
    }

    private func listUserDict() throws -> Ikari_Engine_V1_UserDictListReply {
        try userDictReply(try relayAdmin(client, .listUserDict(Ikari_Engine_V1_UserDictList()), timeout: EngineTimeouts.admin))
    }

    private func userDictReply(_ body: AdminBody) throws -> Ikari_Engine_V1_UserDictListReply {
        guard case .userDictListReply(let reply) = body else { throw AdminSmokeError.unexpectedReply("UserDictListReply") }
        return reply
    }

    private func removeWatchWord() throws -> Ikari_Engine_V1_UserDictListReply {
        var remove = Ikari_Engine_V1_UserDictRemove()
        remove.reading = AdminSmoke.watchReading
        remove.surface = AdminSmoke.watchSurface
        return try userDictReply(try relayAdmin(client, .removeUserDict(remove), timeout: EngineTimeouts.admin))
    }

    private func convertSurfaces() throws -> [String] {
        let request = ConversionRequest(
            sessionID: AdminSmoke.sessionID, reading: AdminSmoke.watchReading, context: convertContext, contextRev: 1000)
        var surfaces: [String] = []
        var page = try client.convert(request, timeout: convertTimeout)
        surfaces += page.candidates.map(\.surface)
        // 見張りの語は最初のページに出るとは限らない。次のページも見る（上限つき）
        var index = 0
        while page.hasNext, index < 5 {
            index += 1
            page = try client.page(conversionID: page.conversionID, index: index, timeout: convertTimeout)
            surfaces += page.candidates.map(\.surface)
        }
        client.cancel(conversionID: page.conversionID)
        return surfaces
    }

    // MARK: 学習とプロファイル（読むだけ）

    private func learningStatus() {
        do {
            let body = try relayAdmin(client, .learningStatus(Ikari_Engine_V1_LearningStatus()), timeout: EngineTimeouts.admin)
            guard case .learningStatusReply(let status) = body else { throw AdminSmokeError.unexpectedReply("LearningStatusReply") }
            if status.maxRows > 0, status.usageRows <= status.maxRows, status.initialRows <= status.maxRows {
                report.pass("admin-learning-status", "usage=\(status.usageRows) initial=\(status.initialRows) max=\(status.maxRows)")
            } else {
                report.fail("admin-learning-status", "usage=\(status.usageRows) initial=\(status.initialRows) max=\(status.maxRows)")
            }
        } catch {
            report.fail("admin-learning-status", "\(error)")
        }
    }

    private func profile() {
        do {
            let body = try relayAdmin(client, .getProfile(Ikari_Engine_V1_ProfileGet()), timeout: EngineTimeouts.admin)
            guard case .profileReply(let reply) = body else { throw AdminSmokeError.unexpectedReply("ProfileReply") }
            if reply.domains.isEmpty {
                report.fail("admin-profile", "no domains")
            } else {
                report.pass("admin-profile", "domains=\(reply.domains.count)")
            }
        } catch {
            report.fail("admin-profile", "\(error)")
        }
    }

    // MARK: エクスポートとインポート（dry_run だけ）

    private func exportAndDryRunImport() {
        let name = "admin-export-import-dry-run"
        do {
            let body = try relayAdmin(client, .exportData(Ikari_Engine_V1_ExportData()), timeout: EngineTimeouts.adminTransfer)
            guard case .exportReply(let export) = body else { throw AdminSmokeError.unexpectedReply("ExportReply") }
            guard export.schema == AdminSmoke.exportSchema,
                String(decoding: export.content.prefix(AdminSmoke.exportHeaderPrefix.utf8.count), as: UTF8.self)
                    == AdminSmoke.exportHeaderPrefix
            else {
                report.fail("admin-export", "schema=\(export.schema) bytes=\(export.content.count)")
                return
            }
            report.pass("admin-export", "bytes=\(export.content.count) \(describe(export.counts))")

            // 原文を含まない: 見張りの語はもう消してあり、書き出しにも出ない
            if String(decoding: export.content, as: UTF8.self).contains(AdminSmoke.watchSurface) {
                report.fail("admin-export-no-watch-word", "the removed watch word is in the export")
            }

            var dry = Ikari_Engine_V1_ImportData()
            dry.content = export.content
            dry.dryRun = true
            let result = try relayAdmin(client, .importData(dry), timeout: EngineTimeouts.adminTransfer)
            guard case .importReply(let reply) = result else { throw AdminSmokeError.unexpectedReply("ImportReply") }
            if !reply.applied, reply.issue.kind == .dataIssueNone, reply.counts == export.counts {
                report.pass(name, "counts match: \(describe(reply.counts))")
            } else {
                report.fail(
                    name,
                    "applied=\(reply.applied) issue=\(reply.issue.kind) import=\(describe(reply.counts)) export=\(describe(export.counts))")
            }

            // 壊れた入力は何も変えずに理由を返す（dry_run のまま）
            var bad = Ikari_Engine_V1_ImportData()
            bad.content = AdminSmoke.garbageImport
            bad.dryRun = true
            let badResult = try relayAdmin(client, .importData(bad), timeout: EngineTimeouts.adminTransfer)
            guard case .importReply(let badReply) = badResult else { throw AdminSmokeError.unexpectedReply("ImportReply") }
            if !badReply.applied, badReply.issue.kind == .notIkariExport {
                report.pass("admin-import-rejects-garbage")
            } else {
                report.fail("admin-import-rejects-garbage", "applied=\(badReply.applied) issue=\(badReply.issue.kind)")
            }
        } catch {
            report.fail(name, "\(error)")
        }
    }

    private func describe(_ counts: Ikari_Engine_V1_DataCounts) -> String {
        "profile=\(counts.profileDomains) dict=\(counts.userDictEntries) learning=\(counts.learningRows) ingest=\(counts.ingestSources)"
    }
}
