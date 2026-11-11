// 「ユーザー辞書」タブ（ADR 0005 §6.2）。読み・表記・品詞・活用型（動詞のとき）。一覧、検索、削除。
// このウィンドウの入力欄には ikari で日本語を打てる（別のプロセスだから。ADR 0005 §6.1）。
import SwiftUI

struct DictionaryTab: View {
    @Bindable var model: DictionaryModel

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            addForm
            if let message = model.message {
                MessageBanner(text: message, kind: .error, dismiss: model.dismissMessage)
            }
            if let confirmation = model.confirmation {
                MessageBanner(text: confirmation, kind: .info, dismiss: nil)
            }
            Divider()
            list
        }
        .padding()
    }

    private var addForm: some View {
        GroupBox("単語を登録") {
            Grid(alignment: .leadingFirstTextBaseline, horizontalSpacing: 12, verticalSpacing: 8) {
                GridRow {
                    Text("読み").gridColumnAlignment(.trailing)
                    TextField("ひらがな", text: $model.draft.reading)
                        .textFieldStyle(.roundedBorder)
                    Text("表記").gridColumnAlignment(.trailing)
                    TextField("登録する語", text: $model.draft.surface)
                        .textFieldStyle(.roundedBorder)
                }
                GridRow {
                    Text("品詞")
                    Picker("品詞", selection: Binding(get: { model.draft.posName }, set: { model.selectPos($0) })) {
                        ForEach(model.posOptions, id: \.name) { Text($0.name).tag($0.name) }
                    }
                    .labelsHidden()
                    if model.needsConjType {
                        Text("活用型")
                        Picker("活用型", selection: $model.draft.conjType) {
                            ForEach(model.verbConjTypes, id: \.self) { Text($0).tag($0) }
                        }
                        .labelsHidden()
                    } else {
                        Text("")
                        Text("")
                    }
                }
                GridRow {
                    Text("コメント")
                    TextField("任意", text: $model.draft.comment)
                        .textFieldStyle(.roundedBorder)
                        .gridCellColumns(2)
                    Button("登録") { Task { await model.add() } }
                        .keyboardShortcut(.defaultAction)
                        .disabled(!model.canAdd)
                }
            }
            .padding(8)
        }
        .disabled(!model.isLoaded)
    }

    private var list: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                TextField("検索（読み・表記・品詞・コメント）", text: $model.query)
                    .textFieldStyle(.roundedBorder)
                Text("\(model.visibleEntries.count) / \(model.entries.count) 語")
                    .foregroundStyle(.secondary)
                    .monospacedDigit()
            }
            if model.visibleEntries.isEmpty {
                ContentUnavailableView(
                    model.entries.isEmpty ? "登録された単語はありません" : "該当する単語はありません",
                    systemImage: "text.book.closed")
                    .frame(maxHeight: .infinity)
            } else {
                List(model.visibleEntries, id: \.self) { entry in
                    HStack(spacing: 12) {
                        Text(entry.reading).frame(width: 130, alignment: .leading)
                        Text(entry.surface).frame(width: 150, alignment: .leading)
                        Text(DictionaryRules.posDisplay(entry.pos)).frame(width: 130, alignment: .leading)
                        Text(entry.comment).foregroundStyle(.secondary).frame(maxWidth: .infinity, alignment: .leading)
                        Button("削除", systemImage: "trash") { Task { await model.remove(entry) } }
                            .labelStyle(.iconOnly)
                            .buttonStyle(.borderless)
                            .help("この単語を削除")
                    }
                    .lineLimit(1)
                }
            }
        }
    }
}
