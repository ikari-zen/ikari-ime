// 「学習とデータ」と「このソフトについて」のタブ（ADR 0005 §6.2）。
import SwiftUI

struct LearningTab: View {
    let model: LearningModel

    var body: some View {
        Form {
            if let message = model.message {
                MessageBanner(text: message, kind: .error, dismiss: model.dismissMessages)
            }
            if let notice = model.notice {
                MessageBanner(text: notice, kind: .info, dismiss: model.dismissMessages)
            }
            Section("学習の件数") {
                if let status = model.status {
                    LabeledContent("使用の学習（変換の選択から）") { Text("\(status.usageRows) 行").monospacedDigit() }
                    LabeledContent("初期学習（取り込んだ文章から）") { Text("\(status.initialRows) 行").monospacedDigit() }
                    LabeledContent("上限") { Text("\(status.maxRows) 行").monospacedDigit() }
                } else {
                    Text("読み込み中…").foregroundStyle(.secondary)
                }
                Text("学習の中身は特徴量だけで、書いた文章そのものは保存されません。画面には件数だけを出します。")
                    .font(.caption).foregroundStyle(.secondary)
            }
            Section("消去") {
                ForEach(ResetTarget.allCases, id: \.self) { target in
                    LabeledContent(target.summary) {
                        Button(target.title, role: .destructive) { Task { await model.reset(target) } }
                    }
                }
                Text("消す前に自動でバックアップを取ります（最新の 3 つ。中身のある最新の 1 つは必ず残し、空の状態や直前と同じ内容のときは取りません）。確定済みの文字には影響しません。")
                    .font(.caption).foregroundStyle(.secondary)
            }
            Section("エクスポートとインポート") {
                LabeledContent("ユーザー辞書、学習、分野を 1 つのファイルに") {
                    Button("エクスポート…") { Task { await model.export() } }
                }
                LabeledContent("書き出したファイルで置き換える（マージしない）") {
                    Button("インポート…") { Task { await model.importFile() } }
                }
                LabeledContent("インポートと消去の前の状態") {
                    Button("バックアップの場所を開く") { model.openBackupFolder() }
                }
                Text("バックアップはエクスポートと同じ形式です。元に戻すときは、そのファイルをインポートします。")
                    .font(.caption).foregroundStyle(.secondary)
            }
            .disabled(model.isBusy)
        }
        .formStyle(.grouped)
    }
}

struct AboutTab: View {
    let model: AboutModel

    var body: some View {
        Form {
            if let message = model.message {
                MessageBanner(text: message, kind: .error, dismiss: nil)
            }
            Section("ikari") {
                LabeledContent("設定アプリの版") { Text(model.appVersion) }
                LabeledContent("入力メソッド（殻）の版") { Text(model.shellVersion) }
                LabeledContent("変換エンジンの版") { Text(model.engineVersion ?? "—") }
                LabeledContent("辞書") { Text(model.dictLabel ?? "—").textSelection(.enabled) }
                LabeledContent("言語モデル（zenz）") {
                    HStack {
                        Text(model.lmText ?? "—")
                        Button("更新", systemImage: "arrow.clockwise") { Task { await model.load() } }
                            .labelStyle(.iconOnly)
                            .buttonStyle(.borderless)
                            .help("読み込みの状態を取り直す")
                    }
                }
            }
            Section("ライセンス") {
                ScrollView {
                    Text(model.notices)
                        .font(.system(.footnote, design: .monospaced))
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                .frame(minHeight: 180)
            }
        }
        .formStyle(.grouped)
    }
}
