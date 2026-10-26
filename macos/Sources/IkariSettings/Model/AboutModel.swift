// 「このソフトについて」タブのモデル（ADR 0005 §6.2）。版、辞書のラベル、LM の状態、THIRD_PARTY_NOTICES（NFR-43）。
import Foundation
import IkariProto
import Observation

enum LMStatusText {
    static func describe(_ state: Ikari_Engine_V1_LmState) -> String {
        switch state {
        case .lmDisabled: "オフ"
        case .lmLoading: "読み込み中"
        case .lmReady: "使用中"
        case .lmFailed: "使えません（読み込みに失敗しました）"
        case .unspecified, .UNRECOGNIZED: "不明"
        }
    }
}

@MainActor @Observable
final class AboutModel {
    /// ライセンス表示の本文が見つからないときの文言
    static let noticesMissing = "ライセンス表示のファイル（THIRD_PARTY_NOTICES）が見つかりません"

    let appVersion: String
    private(set) var shellVersion: String
    private(set) var engineVersion: String?
    private(set) var dictLabel: String?
    private(set) var lmState: Ikari_Engine_V1_LmState?
    private(set) var message: String?
    let notices: String

    private let bridge: any SettingsBridge

    init(bridge: any SettingsBridge, appVersion: String = AboutModel.bundleVersion(), notices: String = AboutModel.bundledNotices()) {
        self.bridge = bridge
        self.appVersion = appVersion
        self.shellVersion = bridge.shellVersion
        self.notices = notices
    }

    var lmText: String? { lmState.map(LMStatusText.describe) }

    /// エンジンの `Hello` で版・辞書・LM の状態を取る。LM の読み込み中は、開き直す（または更新する）と変わる
    func load() async {
        shellVersion = bridge.shellVersion
        do {
            var hello = Ikari_Engine_V1_Hello()
            hello.shellVersion = shellVersion
            guard case .helloReply(let reply) = try await bridge.engine(.hello(hello)) else {
                throw BridgeClientError.unexpectedReply
            }
            engineVersion = reply.engineVersion
            dictLabel = reply.dictLabel
            lmState = reply.lm
            message = nil
        } catch {
            message = Messages.describe(error)
        }
    }

    static func bundleVersion(_ bundle: Bundle = .main) -> String {
        (bundle.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String) ?? "unknown"
    }

    /// build.sh が設定アプリの Resources に置く（リポジトリの THIRD_PARTY_NOTICES.md）
    static func bundledNotices(_ bundle: Bundle = .main) -> String {
        guard let url = bundle.url(forResource: "THIRD_PARTY_NOTICES", withExtension: "md"),
            let text = try? String(contentsOf: url, encoding: .utf8)
        else { return noticesMissing }
        return text
    }
}
