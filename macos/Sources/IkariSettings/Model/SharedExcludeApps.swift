// 「文脈を共有しないアプリ」の欄（ADR 0015 D6）で、アプリ（.app）と Bundle ID を行き来する部分。
// SwiftUI や NSOpenPanel には触れない。システムへの問い合わせは引数で差し替えられる。
import AppKit
import Foundation

enum SharedExcludeApps {
    /// 一覧に出す 1 行
    struct Row: Equatable, Identifiable {
        var bundleID: String
        /// アプリ名。見つからなければ nil（Bundle ID だけを出す）
        var name: String?
        var id: String { bundleID }
    }

    /// 選んだ `.app` の Bundle ID。`.app` でない、または Info.plist に Bundle ID がなければ nil
    static func bundleID(ofAppAt url: URL) -> String? {
        guard url.pathExtension.lowercased() == "app" else { return nil }
        guard let id = Bundle(url: url)?.bundleIdentifier?.trimmingCharacters(in: .whitespacesAndNewlines),
            !id.isEmpty
        else { return nil }
        return id
    }

    /// Bundle ID から、アプリの置き場所を引く。インストールされていなければ nil
    static func systemLocator(_ bundleID: String) -> URL? {
        NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundleID)
    }

    static func systemDisplayName(_ path: String) -> String {
        FileManager.default.displayName(atPath: path)
    }

    /// 表示名。置き場所が分からない、または名前が空なら nil。Finder の表示（ローカライズ済み）から `.app` を除く
    static func displayName(
        forBundleID bundleID: String,
        locate: (String) -> URL? = systemLocator,
        displayNameAtPath: (String) -> String = systemDisplayName
    ) -> String? {
        guard let url = locate(bundleID) else { return nil }
        var name = displayNameAtPath(url.path)
        if name.lowercased().hasSuffix(".app") { name = String(name.dropLast(".app".count)) }
        name = name.trimmingCharacters(in: .whitespacesAndNewlines)
        return name.isEmpty ? nil : name
    }

    static func rows(
        for bundleIDs: [String],
        locate: (String) -> URL? = systemLocator,
        displayNameAtPath: (String) -> String = systemDisplayName
    ) -> [Row] {
        bundleIDs.map {
            Row(
                bundleID: $0,
                name: displayName(forBundleID: $0, locate: locate, displayNameAtPath: displayNameAtPath))
        }
    }
}
