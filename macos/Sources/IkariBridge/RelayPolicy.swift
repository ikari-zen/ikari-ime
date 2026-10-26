// 殻がエンジンへ中継してよい要求の許可リスト（ADR 0005 §6.3「中継の許可リスト」）。
// 殻は `Frame.engine`（Envelope）の oneof の種類だけを見る。中身は解釈せず、ログにも書かない。
// 設定アプリから `Commit` などを送って学習を汚す経路を作らないための関所。
import Foundation
import IkariProto
import IkariShellCore

public enum RelayPolicy {
    /// 中継してよい要求か。**許可するのは `Hello` と ADR 0005 §8.2 の管理の要求だけ。**
    /// `switch` は `default` を持たない: 要求の種類が増えたら、ここでコンパイルが通らなくなり、許可するかを決めさせる。
    public static func isAllowed(_ body: EngineEnvelope.OneOf_Body?) -> Bool {
        switch body {
        case .hello,
            .listUserDict, .addUserDict, .removeUserDict,
            .learningStatus, .resetLearning,
            .exportData, .importData,
            .getProfile, .setProfile:
            return true
        case .helloReply, .beginSession, .ack, .endSession, .prefetch, .convert, .convertReply, .page, .pageReply,
            .commit, .commitText, .cancel, .shutdown, .error,
            // 文節の編集と再変換（ADR 0006）は殻の入力だけが送る。設定アプリから中継しない
            .segmentPage, .segmentPageReply, .resizeSegment, .segmentLayout, .commitSegments, .reconvert,
            .userDictListReply, .learningStatusReply, .exportReply, .importReply, .profileReply,
            // 絵文字の表（ADR 0027）は殻の入力だけが頼む。設定アプリから中継しない
            .emojiTable, .emojiTableReply,
            // 診断ログ（ADR 0034）は殻の入力だけが送る。設定アプリから中継しない
            .diag,
            nil:
            return false
        }
    }

    /// 設定アプリから届いた枠が、中継できないとき `NOT_ALLOWED` の枠を返す。中継してよい（または `engine` の枠でない）なら nil
    public static func rejection(for frame: Frame) -> Frame? {
        guard case .engine(let envelope)? = frame.body else { return nil }
        guard isAllowed(envelope.body) else {
            return .error(requestID: frame.requestID, code: .notAllowed)
        }
        return nil
    }

    /// 中継の待ちの上限。インポートとエクスポートは 30 秒、ほかは 3 秒（ADR 0005 §8.3）
    public static func timeout(for body: EngineEnvelope.OneOf_Body?) -> Duration {
        switch body {
        case .importData, .exportData: EngineTimeouts.adminTransfer
        default: EngineTimeouts.admin
        }
    }
}
