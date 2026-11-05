// 「入力」と「変換」のタブ（ADR 0005 §6.2）。
import SwiftUI

struct InputTab: View {
    let model: SettingsModel

    var body: some View {
        Form {
            if let error = model.errorMessage {
                MessageBanner(text: error, kind: .error, dismiss: model.dismissError)
            }
            Section("入力方式") {
                Picker(
                    "入力方式",
                    selection: .asyncSetter(get: { model.values.inputMethod }, set: model.setInputMethod)
                ) {
                    Text("ローマ字").tag(InputMethodKind.romaji)
                    Text("かな（JIS 配列の物理キー）").tag(InputMethodKind.kana)
                }
                .pickerStyle(.radioGroup)
                Text("次の入力から変わります。入力中の文字列は変わりません。").font(.caption).foregroundStyle(.secondary)
            }
            Section("キー割り当て") {
                Picker(
                    "プリセット",
                    selection: .asyncSetter(get: { model.values.keymapPreset }, set: model.setKeymapPreset)
                ) {
                    Text("macOS 風").tag(KeymapPreset.macos)
                    Text("MS-IME 風").tag(KeymapPreset.msIme)
                    Text("ATOK 風").tag(KeymapPreset.atok)
                }
                .pickerStyle(.radioGroup)
            }
            Section("操作ごとのキー") {
                ForEach(model.keymapRows) { row in
                    KeymapRowView(model: model, row: row)
                }
                Text("F6〜F10 はどのプリセットでも使えます（「なし」にした操作を除く）。")
                    .font(.caption).foregroundStyle(.secondary)
            }
        }
        .formStyle(.grouped)
    }
}

private struct KeymapRowView: View {
    let model: SettingsModel
    let row: KeymapRow

    var body: some View {
        LabeledContent(row.title) {
            HStack(spacing: 8) {
                Text(row.keyText)
                    .monospaced()
                    .frame(minWidth: 56, alignment: .trailing)
                    .foregroundStyle(row.isOverridden ? Color.accentColor : Color.primary)
                KeyRecorderButton(title: "キーを記録…") { chord in
                    Task { await model.setKeymap(row.action, to: .chord(KeyChordText.string(for: chord))) }
                }
                Button("なし") { Task { await model.setKeymap(row.action, to: .none) } }
                    .disabled(row.binding == .none)
                Button("プリセットに戻す") { Task { await model.setKeymap(row.action, to: .preset) } }
                    .disabled(!row.isOverridden)
                    .help(row.presetText.map { "プリセットのキー: \($0)" } ?? "プリセットにはこの操作の文字キーがありません")
            }
        }
    }
}

/// アプリの一覧の欄（Bundle ID の一覧を、追加・削除・既定に戻す）。「ショートコードを使わないアプリ」が使う
private struct AppListSection: View {
    let title: String
    let note: String
    let bundleIDs: [String]
    let defaultIDs: [String]
    let add: (String) async -> Void
    let addApp: (URL) async -> Void
    let remove: (String) async -> Void
    let reset: () async -> Void
    @State private var newID = ""

    var body: some View {
        Section(title) {
            Text(note).font(.caption).foregroundStyle(.secondary)
            ForEach(ExcludedApps.rows(for: bundleIDs)) { row in
                LabeledContent {
                    Button("削除") { Task { await remove(row.bundleID) } }
                } label: {
                    if let name = row.name {
                        Text(name)
                        Text(row.bundleID).font(.caption).monospaced().foregroundStyle(.secondary)
                    } else {
                        Text(row.bundleID).monospaced()
                    }
                }
            }
            Button("アプリを選ぶ…", action: pickApp)
            HStack {
                TextField("Bundle ID（例: com.apple.Terminal）", text: $newID)
                    .onSubmit(submit)
                Button("追加", action: submit)
                    .disabled(newID.trimmingCharacters(in: .whitespaces).isEmpty)
            }
            Button("既定の一覧に戻す") { Task { await reset() } }
                .disabled(bundleIDs == defaultIDs)
        }
    }

    private func pickApp() {
        guard let url = AppKitDialogs().chooseApp() else { return }
        Task { await addApp(url) }
    }

    private func submit() {
        let text = newID
        newID = ""
        Task { await add(text) }
    }
}

/// 絵文字のショートコード（ADR 0027）。「:」から始めて、`:eyes:` のように打つと絵文字が候補に出る
private struct EmojiShortcodeSection: View {
    let model: SettingsModel

    var body: some View {
        Section("絵文字のショートコード") {
            Toggle(
                "「:」から始まる名前（:eyes: など）で絵文字を入力する",
                isOn: .asyncSetter(get: { model.values.emojiShortcode }, set: model.setEmojiShortcode))
            Text("かな入力では、Shift を押しながら「け」のキーで始めます。次の入力から変わります。")
                .font(.caption).foregroundStyle(.secondary)
            Toggle(
                "英語のモードで小さい文字（:m2: → m²、:m_2: → m₂、:cm: → ㎝）も候補に出す",
                isOn: .asyncSetter(get: { model.values.emojiSmallLetters }, set: model.setEmojiSmallLetters))
            Text("上のショートコードがオンのときだけ働きます。")
                .font(.caption).foregroundStyle(.secondary)
        }
        AppListSection(
            title: "ショートコードを使わないアプリ",
            note: "ターミナルなど、1 打鍵ずつアプリが意味を持つところでは、ここに挙げたアプリで「:」を今までどおりに入力します。",
            bundleIDs: model.values.emojiShortcodeExclude,
            defaultIDs: SettingsValues.defaults.emojiShortcodeExclude,
            add: model.addEmojiShortcodeExclude(_:),
            addApp: model.addEmojiShortcodeExclude(appAt:),
            remove: model.removeEmojiShortcodeExclude(_:),
            reset: model.resetEmojiShortcodeExclude)
    }
}

/// 住所 ↔ 郵便番号（ADR 0032）
private struct PostalSection: View {
    let model: SettingsModel

    var body: some View {
        Section("郵便番号") {
            Toggle(
                "住所と郵便番号を相互に変換する",
                isOn: .asyncSetter(get: { model.values.postalJapanese }, set: model.setPostalJapanese))
            Text("郵便番号（7 桁）を打つと住所が、住所（市区町村・町名まで）を打つと郵便番号が候補に出ます。切り替えると変換エンジンを起こし直します。")
                .font(.caption).foregroundStyle(.secondary)
            Toggle(
                "「て」+ 郵便番号で〒つきの候補を出す",
                isOn: .asyncSetter(get: { model.values.postalTePrefix }, set: model.setPostalTePrefix))
            Text("「て」「テ」「te」のあとに郵便番号を打つと、〒100-0001 のような候補を先頭に出します。切り替えると変換エンジンを起こし直します。")
                .font(.caption).foregroundStyle(.secondary)
            Toggle(
                "英語のモードでも郵便番号の候補を出す",
                isOn: .asyncSetter(get: { model.values.postalEnglish }, set: model.setPostalEnglish))
            Text("英語のモードの対応はこれからです（今は効きません）。")
                .font(.caption).foregroundStyle(.secondary)
        }
    }
}

/// Unicode 表記（1 文字のかな → U+3042）
private struct UnicodeSection: View {
    let model: SettingsModel

    var body: some View {
        Section("Unicode 表記") {
            Toggle(
                "1 文字のかなに U+3042 のような Unicode 表記の候補を出す",
                isOn: .asyncSetter(get: { model.values.unicodeNotation }, set: model.setUnicodeNotation))
            Text("「あ」を変換すると U+3042 が候補に出ます。切り替えると変換エンジンを起こし直します。")
                .font(.caption).foregroundStyle(.secondary)
        }
    }
}

/// 曜日の読みに、直近の日付の候補
private struct WeekdayDatesSection: View {
    let model: SettingsModel

    var body: some View {
        Section("曜日の日付") {
            Toggle(
                "曜日の読みに、直近の日付の候補を出す",
                isOn: .asyncSetter(get: { model.values.dateWeekdayDates }, set: model.setDateWeekdayDates))
            Text("「げつようび」を変換すると、今日を含めて次の月曜日の日付が候補に出ます。切り替えると変換エンジンを起こし直します。")
                .font(.caption).foregroundStyle(.secondary)
        }
    }
}

struct ConversionTab: View {
    let model: SettingsModel

    /// 「LM を解放するまでの時間」の選択肢（秒）。いまの値がここになければ足す
    private static let idleChoices = [0, 60, 300, 900, 1800, 3600]

    var body: some View {
        Form {
            if let error = model.errorMessage {
                MessageBanner(text: error, kind: .error, dismiss: model.dismissError)
            }
            Section("候補") {
                Toggle(
                    "最後のページで Space を押したら先頭に戻る",
                    isOn: .asyncSetter(get: { model.values.pagingWrap }, set: model.setPagingWrap))
                Picker(
                    "1 ページの件数",
                    selection: .asyncSetter(get: { model.values.pageSize }, set: model.setPageSize)
                ) {
                    ForEach(Array(SettingsValues.pageSizeRange), id: \.self) { Text("\($0) 件").tag($0) }
                }
                .pickerStyle(.menu)
                Toggle(
                    "確信が高い変換では候補一覧を出さない",
                    isOn: .asyncSetter(
                        get: { model.values.candidateHideWhenConfident }, set: model.setCandidateHideWhenConfident))
                Text("最初の Space で 1 位だけを出します。もう一度 Space か ↓ で一覧を出します。")
                    .font(.caption).foregroundStyle(.secondary)
            }
            Section("文節ごとの削除") {
                Stepper(
                    "最小の文字数: \(model.values.clauseMinChars) 文字",
                    value: .asyncSetter(get: { model.values.clauseMinChars }, set: model.setClauseMinChars),
                    in: SettingsValues.clauseMinCharsRange
                )
                Text(
                    "消す文字がこの数に満たないときは、前の文節まで広げて消します（既定は \(SettingsValues.defaults.clauseMinChars) 文字。\(SettingsValues.clauseMinCharsRange.lowerBound) は下限なし）。キーは「操作ごとのキー」で変えられます。"
                )
                .font(.caption).foregroundStyle(.secondary)
            }
            Section("言語モデル（zenz）") {
                Toggle(
                    "言語モデルで候補を並べ替える",
                    isOn: .asyncSetter(get: { model.values.lmEnabled }, set: model.setLMEnabled))
                Text("切り替えると変換エンジンを起こし直します。読み込み中の数秒は、辞書だけで変換します。")
                    .font(.caption).foregroundStyle(.secondary)
                Picker(
                    "使っていないとき解放するまでの時間",
                    selection: .asyncSetter(get: { model.values.idleUnloadSecs }, set: model.setIdleUnloadSecs)
                ) {
                    ForEach(idleOptions, id: \.self) { Text(Self.idleLabel($0)).tag($0) }
                }
                .pickerStyle(.menu)
                .disabled(!model.values.lmEnabled)
                Stepper(
                    "LM の先行計算の締め切り: \(model.values.lmDeadlineMs) ミリ秒",
                    value: .asyncSetter(get: { model.values.lmDeadlineMs }, set: model.setLMDeadlineMs),
                    in: SettingsValues.lmDeadlineRange, step: 50
                )
                .disabled(!model.values.lmEnabled)
                Stepper(
                    "Space で先行計算を待つ上限: \(model.values.convertWaitMs) ミリ秒",
                    value: .asyncSetter(get: { model.values.convertWaitMs }, set: model.setConvertWaitMs),
                    in: SettingsValues.convertWaitRange, step: 10
                )
                .disabled(!model.values.lmEnabled)
                Text(
                    "どちらも変えるとエンジンを起こし直します（既定は \(SettingsValues.defaults.lmDeadlineMs) ミリ秒と \(SettingsValues.defaults.convertWaitMs) ミリ秒）。締め切りは \(SettingsValues.lmDeadlineRange.lowerBound)〜\(SettingsValues.lmDeadlineRange.upperBound)、待つ上限は \(SettingsValues.convertWaitRange.lowerBound)〜\(SettingsValues.convertWaitRange.upperBound)（0 は待たない）。"
                )
                .font(.caption).foregroundStyle(.secondary)
            }
            EmojiShortcodeSection(model: model)
            PostalSection(model: model)
            UnicodeSection(model: model)
            WeekdayDatesSection(model: model)
            Section("属性プロファイル（よく書く分野）") {
                Text("文脈がまだ少ないときの手がかりとして、分野の語を少しだけ上位に出します。文法で決まる箇所は変えません。")
                    .font(.caption).foregroundStyle(.secondary)
                if let message = model.profileMessage {
                    MessageBanner(text: message, kind: .error, dismiss: nil)
                }
                ForEach(model.profile) { domain in
                    Picker(
                        domain.name,
                        selection: .asyncSetter(
                            get: { domain.weight }, set: { await model.setWeight($0, forDomain: domain.id) })
                    ) {
                        Text("使わない").tag(ProfileWeight.off)
                        Text("たまに").tag(ProfileWeight.sometimes)
                        Text("よく").tag(ProfileWeight.often)
                    }
                    .pickerStyle(.segmented)
                }
            }
        }
        .formStyle(.grouped)
    }

    private var idleOptions: [Int] {
        Self.idleChoices.contains(model.values.idleUnloadSecs)
            ? Self.idleChoices : (Self.idleChoices + [model.values.idleUnloadSecs]).sorted()
    }

    static func idleLabel(_ seconds: Int) -> String {
        switch seconds {
        case 0: "解放しない"
        case 3600: "1 時間"
        case let value where value % 60 == 0: "\(value / 60) 分"
        default: "\(seconds) 秒"
        }
    }
}
