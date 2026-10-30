// 文節と再変換の E2E（ADR 0006 §10.1「E2E」）。XPC 経由で、実エンジンに次を流す。
//
//   Convert（2 文節になる読み）→ SegmentPage（文節 1）→ ResizeSegment（区切り直し）→ CommitSegments
//   Reconvert（secure のセッションでは BAD_REQUEST で断られる）
//
// **学習を汚さないよう、確定はすべて secure のセッションで行う**（エンジンは secure のセッションの確定を学習しない。
// 学習しないことも、エンジンが確定を受け付けて変換を閉じること（UNKNOWN_CONVERSION）で確かめる）。
// 読みや表記の中身はエンジンの辞書に依存するので、確かめるのは「区切りの読みが元の読みに戻る」
// 「文節の候補の 0 番が表示中の表記と一致する」「区切り直しで境界が動く」といった、辞書によらない性質に絞る。
import Foundation
import IkariXPC

enum SegmentSmoke {
    /// 殻の通常のセッション（ID 1）とは別の、secure なセッション
    static let sessionID: UInt64 = 2
    /// 文節に分かれるはずの読み（初めて｜知った）
    static let multiSegmentReading = "はじめてしった"
    /// 複合名詞。1 文節で出て、Shift+← を 2 回で りぽじとり｜めい に区切り直す
    static let compoundReading = "りぽじとりめい"
    static let compoundHeadReading = "りぽじとり"
    /// 再変換の見張りの文字列（辞書にある語）
    static let reconvertSurface = "品詞"
    static let staleLayoutRev: UInt32 = 9_999
    static let unknownConversionID: UInt64 = 0xDEAD_BEEF
    /// 区切り直しの試行の上限（読みの長さ）
    static let maxResizeSteps = 16
    static let firstContextRev: UInt64 = 200
}

struct SegmentE2E {
    let client: XPCEngineClient
    let report: Report
    let context: SurroundingText
    let timeout: Duration

    private func request(_ reading: String, rev: UInt64) -> ConversionRequest {
        ConversionRequest(sessionID: SegmentSmoke.sessionID, reading: reading, context: context, contextRev: rev)
    }

    private func expect(_ name: String, _ body: () throws -> String) {
        do {
            report.pass(name, try body())
        } catch let failure as SegmentCheckFailure {
            report.fail(name, failure.message)
        } catch {
            report.fail(name, "\(error)")
        }
    }

    func run() {
        client.beginSession(SessionInfo(sessionID: SegmentSmoke.sessionID, clientBundleID: IkariIdentifiers.smoke, secure: true))
        defer { client.endSession(sessionID: SegmentSmoke.sessionID) }

        var layout: SegmentLayoutInfo?
        expect("segment-convert") {
            let page = try client.convert(request(SegmentSmoke.multiSegmentReading, rev: SegmentSmoke.firstContextRev), timeout: timeout)
            guard let found = page.layout else { throw SegmentCheckFailure("ConvertReply.layout is missing") }
            try Self.requireConsistent(found, reading: SegmentSmoke.multiSegmentReading)
            guard found.segments.count >= 2 else {
                throw SegmentCheckFailure("expected 2+ segments, got \(found.segments.count)")
            }
            // 表示は今と同じ: 文節の最初の表記をつないだものが、全体の第一候補
            let joined = found.segments.map(\.surface).joined()
            guard joined == page.candidates.first?.surface else {
                throw SegmentCheckFailure("joined segment surfaces differ from the first candidate")
            }
            guard found.focusedPage?.candidates.first?.surface == found.segments[0].surface else {
                throw SegmentCheckFailure("candidate 0 of the focused segment is not the displayed surface")
            }
            layout = found
            return "segments=\(found.segments.count) layoutRev=\(found.layoutRev) engine=\(found.engineMicros)us"
        }

        if let layout {
            segmentPageAndCommit(layout)
        } else {
            report.skip("segment-page", "no layout")
            report.skip("segment-commit", "no layout")
        }
        resize()
        reconvert()
    }

    // MARK: SegmentPage と CommitSegments

    private func segmentPageAndCommit(_ layout: SegmentLayoutInfo) {
        expect("segment-page") {
            let left: [SegmentChoiceValue] = [.candidate(0)]
            let page = try client.segmentPage(
                conversionID: layout.conversionID, layoutRev: layout.layoutRev, segment: 1, index: 0, left: left,
                timeout: timeout)
            guard !page.candidates.isEmpty else { throw SegmentCheckFailure("segment 1 has no candidates") }
            // 文節に移っても、表示中の表記が変わらない（候補の 0 番に固定）
            guard page.candidates[0].surface == layout.segments[1].surface else {
                throw SegmentCheckFailure("candidate 0 of segment 1 is not the displayed surface")
            }
            var detail = "candidates=\(page.candidates.count) hasNext=\(page.hasNext)"
            if page.hasNext {
                let next = try client.segmentPage(
                    conversionID: layout.conversionID, layoutRev: layout.layoutRev, segment: 1, index: 1, left: left,
                    timeout: timeout)
                guard next.pageIndex == 1, !next.candidates.isEmpty else {
                    throw SegmentCheckFailure("segment 1 page 1 is empty or has the wrong index")
                }
                detail += " page1=\(next.candidates.count)"
            }
            // 古い版と知らない変換は断られる
            try Self.requireError(.staleLayout) {
                _ = try client.segmentPage(
                    conversionID: layout.conversionID, layoutRev: SegmentSmoke.staleLayoutRev, segment: 1, index: 0,
                    left: left, timeout: timeout)
            }
            try Self.requireError(.unknownConversion) {
                _ = try client.segmentPage(
                    conversionID: SegmentSmoke.unknownConversionID, layoutRev: 0, segment: 0, index: 0, left: [],
                    timeout: timeout)
            }
            return detail
        }

        expect("segment-commit") {
            // 確定は通知。secure のセッションなので学習されない。変換が閉じたことは UNKNOWN_CONVERSION で確かめる
            client.commitSegments(
                conversionID: layout.conversionID, layoutRev: layout.layoutRev,
                choices: layout.segments.map { _ in .candidate(0) })
            try Self.requireError(.unknownConversion) {
                _ = try client.segmentPage(
                    conversionID: layout.conversionID, layoutRev: layout.layoutRev, segment: 0, index: 0, left: [],
                    timeout: timeout)
            }
            return "conversion=\(layout.conversionID) closed"
        }
    }

    // MARK: ResizeSegment

    private func resize() {
        expect("segment-resize") {
            let page = try client.convert(
                request(SegmentSmoke.compoundReading, rev: SegmentSmoke.firstContextRev + 1), timeout: timeout)
            guard var layout = page.layout else { throw SegmentCheckFailure("ConvertReply.layout is missing") }
            try Self.requireConsistent(layout, reading: SegmentSmoke.compoundReading)

            // 先頭の文節の読みを りぽじとり にする（1 文字ずつ縮める／伸ばす）
            let target = SegmentSmoke.compoundHeadReading.count
            var steps = 0
            while layout.segments[0].reading.count != target {
                let delta = layout.segments[0].reading.count > target ? -1 : 1
                steps += 1
                guard steps <= SegmentSmoke.maxResizeSteps else { throw SegmentCheckFailure("too many resize steps") }
                let before = layout
                layout = try client.resize(
                    conversionID: before.conversionID, layoutRev: before.layoutRev, segment: 0, delta: delta, left: [],
                    timeout: timeout)
                try Self.requireConsistent(layout, reading: SegmentSmoke.compoundReading)
                guard layout.layoutRev > before.layoutRev else { throw SegmentCheckFailure("layoutRev did not advance") }
                guard layout.segments[0].reading.count == before.segments[0].reading.count + delta else {
                    throw SegmentCheckFailure("the focused segment did not move by \(delta)")
                }
                guard layout.focusedPage?.candidates.first?.surface == layout.segments[layout.focused].surface else {
                    throw SegmentCheckFailure("candidate 0 of the resized segment is not the displayed surface")
                }
            }
            guard layout.segments.count >= 2, layout.segments[0].reading == SegmentSmoke.compoundHeadReading else {
                throw SegmentCheckFailure("the head segment is not \(SegmentSmoke.compoundHeadReading)")
            }
            // 古い版の区切り直しは断られる
            try Self.requireError(.staleLayout) {
                _ = try client.resize(
                    conversionID: layout.conversionID, layoutRev: layout.layoutRev &- 1, segment: 0, delta: 1, left: [],
                    timeout: timeout)
            }
            // 新しい版で確定できる（secure）
            client.commitSegments(
                conversionID: layout.conversionID, layoutRev: layout.layoutRev, choices: layout.segments.map { _ in .candidate(0) })
            return "steps=\(steps) segments=\(layout.segments.map(\.reading).joined(separator: "|"))"
        }
    }

    // MARK: Reconvert（secure では断られる）

    private func reconvert() {
        // 再変換は secure のセッションでは始めない（NFR-18。ADR 0006 §9）。エンジンは BAD_REQUEST で拒否するのが正しい。
        // 通常のセッションでの再変換は、学習や履歴に触れないよう、この smoke では流さない（エンジンの単体テストが担う）
        expect("reconvert-refused-in-secure") {
            try Self.requireError(.badRequest) {
                _ = try client.reconvert(
                    ReconvertRequest(sessionID: SegmentSmoke.sessionID, surface: SegmentSmoke.reconvertSurface, context: context),
                    timeout: timeout)
            }
            return "secure session refused the reconversion with BAD_REQUEST"
        }
    }

    // MARK: 検査の部品

    private struct SegmentCheckFailure: Error {
        let message: String
        init(_ message: String) { self.message = message }
    }

    /// 区切りの読みをつなぐと元の読みになり、フォーカスが範囲内で、空の文節がない
    private static func requireConsistent(_ layout: SegmentLayoutInfo, reading: String) throws {
        guard layout.segments.map(\.reading).joined() == reading else {
            throw SegmentCheckFailure("segment readings do not add up to the typed reading")
        }
        guard layout.segments.allSatisfy({ !$0.reading.isEmpty && !$0.surface.isEmpty }) else {
            throw SegmentCheckFailure("an empty segment")
        }
        guard layout.segments.indices.contains(layout.focused) else { throw SegmentCheckFailure("focus is out of range") }
    }

    /// `body` が指定のエンジンのエラーで失敗すること
    private static func requireError(_ expected: EngineErrorCode, _ body: () throws -> Void) throws {
        do {
            try body()
        } catch EngineClientError.engine(let code) where code == expected {
            return
        } catch {
            throw SegmentCheckFailure("expected engine error \(expected), got \(error)")
        }
        throw SegmentCheckFailure("expected engine error \(expected), but the request succeeded")
    }
}
