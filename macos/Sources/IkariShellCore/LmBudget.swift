// 変換1回の待ち時間（ADR 0004 §3.4、§7.5）。エンジン側の `LmBudget` と同じ式。
import Foundation

public enum EngineTimeouts {
    public static let base: Duration = .milliseconds(30)
    public static let perCharacter: Duration = .milliseconds(6)
    public static let maximum: Duration = .milliseconds(300)
    /// 殻が `lm_budget` に足す IPC の余裕
    public static let ipcMargin: Duration = .milliseconds(40)

    /// `min(300ms, max(30ms, 6ms × 読みの文字数))`
    public static func lmBudget(readingCharacterCount: Int) -> Duration {
        let count = max(0, readingCharacterCount)
        return min(maximum, max(base, perCharacter * count))
    }

    /// 管理の要求の待ちの上限（ADR 0005 §8.3、docs/phase2-contract.md §5.3）。インポートとエクスポート以外
    public static let admin: Duration = .seconds(3)
    /// インポートとエクスポートの待ちの上限
    public static let adminTransfer: Duration = .seconds(30)

    /// `Convert` と `Page` を同期で待つ上限 = `lm_budget(読み) + 40ms`
    public static func convertTimeout(readingCharacterCount: Int) -> Duration {
        lmBudget(readingCharacterCount: readingCharacterCount) + ipcMargin
    }
}
