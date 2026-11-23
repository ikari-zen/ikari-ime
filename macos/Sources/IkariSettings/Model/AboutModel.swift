// 「このソフトについて」タブのモデル（ADR 0005 §6.2）。版、辞書のラベル、LM の状態、THIRD_PARTY_NOTICES（NFR-43）。
import Foundation
import IkariProto
import IkariShellABI
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
    private(set) var dictSource: Ikari_Engine_V1_DictSource = .unspecified
    /// 辞書を作った日（YYYY-MM-DD）。不明なら nil（画面に出さない）
    private(set) var dictDate: String?
    private(set) var message: String?
    let notices: String

    /// 版と、出してよい機能（Rust が決めた値。ADR 0080 §7.2）。将来のボタンはこの値を見て出す・出さないを決める
    let capabilities: EditionCapabilities

    private let bridge: any SettingsBridge

    init(
        bridge: any SettingsBridge,
        capabilities: EditionCapabilities = ShellSettings.editionCapabilities() ?? .unavailable,
        appVersion: String = AboutModel.bundleVersion(),
        notices: String = AboutModel.bundledNotices()
    ) {
        self.bridge = bridge
        self.capabilities = capabilities
        self.appVersion = appVersion
        self.shellVersion = bridge.shellVersion
        self.notices = notices
    }

    var lmText: String? { lmState.map(LMStatusText.describe) }

    /// 版の名前。オフライン版のときだけ出す（通常版は nil）
    var editionName: String? { capabilities.edition == .offline ? "オフライン版" : nil }

    /// 辞書の出どころ（「同梱の辞書（日付）」「入れた辞書（日付）」）。日付が不明なら日付は出さない。出どころが不明なら nil
    var dictSourceText: String? {
        let name: String
        switch dictSource {
        case .bundled: name = "同梱の辞書"
        case .user: name = "入れた辞書"
        case .unspecified, .UNRECOGNIZED: return nil
        }
        guard let dictDate else { return name }
        return "\(name)（\(dictDate)）"
    }

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
            dictSource = reply.dictSource
            dictDate = reply.dictDate.isEmpty ? nil : reply.dictDate
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
