// 画面に出す文言（エラー、データの問題、品詞）。純関数にしてテストする。
// ここに出るのは画面のためだけで、ログには書かない（NFR-19。`DataIssue.detail` は利用者自身のデータを含みうる）。
import Foundation
import IkariBridge
import IkariProto

enum Messages {
    static func describe(_ error: Error) -> String {
        guard let error = error as? BridgeClientError else { return "予期しないエラーが起きました" }
        switch error {
        case .disconnected:
            return "ikari との接続が切れました。ikari を入力ソースに選び直してから、再接続してください"
        case .versionMismatch:
            return "ikari の版が設定アプリと合いません。入力ソースを選び直すか、ログインし直してください"
        case .timeout:
            return "ikari から返事がありません。しばらくしてからもう一度お試しください"
        case .tooLarge:
            return "データが大きすぎて送れません"
        case .unexpectedReply:
            return "ikari から予期しない返事が届きました。入力ソースを選び直してください"
        case .bridge(let code, let key):
            return describe(bridge: code, key: key)
        case .engine(let code):
            return describe(engine: code)
        }
    }

    private static func describe(bridge code: BridgeErrorCode, key: String) -> String {
        switch code {
        case .versionMismatch:
            return describe(BridgeClientError.versionMismatch)
        case .invalidSetting:
            return key.isEmpty ? "設定の値が正しくありません" : "設定「\(key)」の値が正しくありません"
        case .notAllowed:
            return "この操作は許可されていません"
        case .engineUnavailable:
            return "変換エンジンにつながりません。ikari を使うと自動で起動します。しばらくしてからもう一度お試しください"
        case .tooLarge:
            return describe(BridgeClientError.tooLarge)
        case .writeFailed:
            return "設定ファイルを書き込めませんでした"
        case .bridgeErrorUnspecified, .UNRECOGNIZED:
            return "予期しないエラーが起きました"
        }
    }

    private static func describe(engine code: Ikari_Engine_V1_ErrorCode) -> String {
        switch code {
        case .badRequest:
            // 古いエンジンは新しい管理の要求を知らず、BAD_REQUEST を返す（ADR 0005 §8.6）
            return "変換エンジンがこの操作に対応していません。エンジンが古い可能性があります。入力ソースを選び直してください"
        case .engineNotReady:
            return "変換エンジンを準備中です。しばらくしてからもう一度お試しください"
        default:
            return "変換エンジンがエラーを返しました"
        }
    }

    /// `DataIssue` を画面の文言にする。`changedNothing` は、インポートのように「何も変えていない」と添えたいとき
    static func describe(_ issue: Ikari_Engine_V1_DataIssue, changedNothing: Bool = false) -> String {
        let tail = changedNothing ? "。何も変更していません" : ""
        switch issue.kind {
        case .notIkariExport:
            return "ikari の書き出しファイルではありません" + tail
        case .schemaUnsupported:
            let section = issue.section.isEmpty ? "" : "（\(issue.section)）"
            return "このファイルの形式の版 \(issue.foundSchema)\(section)には対応していません。対応しているのは版 \(issue.supportedSchema) です"
                + tail
        case .parse:
            let place = issue.line > 0 ? "\(issue.line) 行目" : "ファイル"
            let section = issue.section.isEmpty ? "" : "（\(issue.section)）"
            return "\(place)\(section)を読めません" + tail
        case .invalidEntry:
            return issue.detail.isEmpty ? "登録できない項目です" + tail : issue.detail + tail
        case .notFound:
            return "該当する項目がありません" + tail
        case .tooLarge:
            return "データが大きすぎます" + tail
        case .io:
            return "ファイルの読み書きに失敗しました" + tail
        case .dataIssueNone, .UNRECOGNIZED:
            return "不明な問題が起きました" + tail
        }
    }

    static func hasIssue(_ issue: Ikari_Engine_V1_DataIssue) -> Bool {
        issue.kind != .dataIssueNone
    }

    /// 「辞書 N 語、学習 M 行、分野 K、取り込み元 L」（ADR 0005 §8.4）
    static func describe(_ counts: Ikari_Engine_V1_DataCounts) -> String {
        "辞書 \(counts.userDictEntries) 語、学習 \(counts.learningRows) 行、分野 \(counts.profileDomains)、取り込み元 \(counts.ingestSources)"
    }
}
