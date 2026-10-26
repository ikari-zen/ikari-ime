// settings.tsv の書き換え（ADR 0005 §7.1）。契約。
import Foundation

public enum SettingsFile {
    private static let separator: Character = "\t"
    private static let commentPrefix = "#"

    /// 今のファイルの文字列から、該当するキーの行の値だけを差し替えた文字列を返す。純関数。
    ///
    /// - `set` のキーが既にあれば、その行の値だけを差し替える（同じキーが複数あれば最後の行）。ないキーは末尾に足す（`set` の順）。
    /// - `reset` のキーの行は消す（既定に戻す）。同じキーが複数あれば、すべて消す（古い行の値が表に出ないように）。
    ///   同じキーが `set` と `reset` の両方にあるときは、`reset` を先に行う（結果は `set` の値の 1 行）。
    /// - 注釈、空行、未知のキー、ほかのキーの行は、中身も順もそのまま残す（`engine.*` を含む）。
    /// - 区切りの tab や改行を含むキー・値は、別の行を混ぜられるので無視する（ファイルに書かない）。
    /// - 結果の検証（`Settings.parse` の警告が増えたら書かない）は呼び出す側が行う。
    public static func update(text: String, set: [(key: String, value: String)], reset: [String]) -> String {
        // 改行の流儀を保つ。最後の要素は、末尾が改行で終わるときだけ空文字列になる
        var lines = text.components(separatedBy: "\n")

        for key in reset where isWritable(key: key) {
            lines.removeAll { keyOf(line: $0) == key }
        }

        var appended: [String] = []
        for (key, value) in set where isWritable(key: key) && isWritable(value: value) {
            if let index = lines.lastIndex(where: { keyOf(line: $0) == key }) {
                // 行末の CR（CRLF のファイル）は残す
                let carriage = lines[index].hasSuffix("\r") ? "\r" : ""
                lines[index] = "\(key)\(separator)\(value)\(carriage)"
            } else if let index = appended.lastIndex(where: { keyOf(line: $0) == key }) {
                appended[index] = "\(key)\(separator)\(value)"
            } else {
                appended.append("\(key)\(separator)\(value)")
            }
        }
        guard !appended.isEmpty else { return lines.joined(separator: "\n") }

        // 末尾が改行で終わっていれば最後の要素は空。足す前に取り除き、足したあとで改行を付ける
        if lines.last == "" { lines.removeLast() }
        lines += appended
        return lines.joined(separator: "\n") + "\n"
    }

    /// `Settings.parse` と同じ規則で、行のキーを取り出す。注釈、空行、tab のない行は nil
    private static func keyOf(line rawLine: String) -> String? {
        let line = rawLine.trimmingCharacters(in: CharacterSet(charactersIn: "\r"))
        if line.trimmingCharacters(in: .whitespaces).isEmpty || line.hasPrefix(commentPrefix) { return nil }
        let columns = line.split(separator: separator, maxSplits: 1, omittingEmptySubsequences: false)
        guard columns.count == 2 else { return nil }
        return columns[0].trimmingCharacters(in: .whitespaces)
    }

    private static let forbidden = CharacterSet.newlines.union(.controlCharacters)

    private static func isWritable(key: String) -> Bool {
        !key.isEmpty && !key.hasPrefix(commentPrefix) && key.rangeOfCharacter(from: forbidden) == nil
            && key == key.trimmingCharacters(in: .whitespaces)
    }

    private static func isWritable(value: String) -> Bool {
        value.rangeOfCharacter(from: forbidden) == nil
    }
}
