// 候補ウィンドウの高さ。先頭の行（文節の下線。#292）がないときは、これまでと同じ。
import Testing

@testable import IkariIME

@MainActor
@Suite struct CandidateViewHeightTests {
    @Test func withoutTheHeaderTheHeightIsRowsOnly() {
        let three: Double = CandidateView.height(rows: 3)
        let zero: Double = CandidateView.height(rows: 0)
        #expect(three == Double(3 * CandidateView.rowHeight))
        #expect(zero == Double(CandidateView.rowHeight))
    }

    @Test func theHeaderAddsOneHeaderRow() {
        let actual: Double = CandidateView.height(rows: 3, hasHeader: true)
        #expect(actual == Double(3 * CandidateView.rowHeight + CandidateView.headerHeight))
    }
}
