// 候補ウィンドウの高さ。
import Testing

@testable import IkariIME

@MainActor
@Suite struct CandidateViewHeightTests {
    @Test func heightIsRowsOnly() {
        let three: Double = CandidateView.height(rows: 3)
        let zero: Double = CandidateView.height(rows: 0)
        #expect(three == Double(3 * CandidateView.rowHeight))
        #expect(zero == Double(CandidateView.rowHeight))
    }
}
