// 画面の共通部品: メッセージの帯、キーの記録ボタン、非同期の更新を受ける Binding。
import AppKit
import SwiftUI

extension Binding {
    /// 値を読んで、変更は非同期の操作（殻に書く）として投げる。反映後の値でモデルが更新され、画面が戻る
    @MainActor
    static func asyncSetter(get: @escaping @MainActor () -> Value, set: @escaping @MainActor (Value) async -> Void) -> Binding<Value> {
        Binding(get: get, set: { value in Task { @MainActor in await set(value) } })
    }
}

struct MessageBanner: View {
    enum Kind { case error, info }

    var text: String
    var kind: Kind
    var dismiss: (() -> Void)?

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Image(systemName: kind == .error ? "exclamationmark.triangle.fill" : "checkmark.circle.fill")
                .foregroundStyle(kind == .error ? Color.orange : Color.green)
            Text(text)
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
            if let dismiss {
                Button("閉じる", systemImage: "xmark", action: dismiss)
                    .labelStyle(.iconOnly)
                    .buttonStyle(.plain)
                    .help("閉じる")
            }
        }
        .padding(8)
        .background(.quaternary, in: RoundedRectangle(cornerRadius: 6))
    }
}

/// キーを押して割り当てを記録するボタン。記録中は次の 1 打鍵を横取りして `KeyChordText.record` に渡す。
/// 受け付けるのは F キーと、Ctrl / Option / Command を伴う 1 文字（状態機械が引くものだけ）。Esc で取り消す
struct KeyRecorderButton: View {
    var title: String
    var onRecord: (KeyChord) -> Void

    @State private var recording = false
    @State private var hint: String?
    @State private var monitor = EventMonitorBox()

    var body: some View {
        HStack(spacing: 6) {
            Button(recording ? "キーを押してください（Esc で取り消し）" : title) {
                recording ? stop() : start()
            }
            if let hint {
                Text(hint).font(.caption).foregroundStyle(.secondary)
            }
        }
        .onDisappear { stop() }
    }

    private func start() {
        hint = nil
        recording = true
        monitor.install { event in
            let result = KeyChordText.record(
                keyCode: event.keyCode, charactersIgnoringModifiers: event.characters, modifiers: event.modifiers)
            switch result {
            case .chord(let chord):
                stop()
                onRecord(chord)
            case .cancel:
                stop()
            case .rejected(.needsModifier):
                hint = "Ctrl・Option・Command のどれかと一緒に押してください"
            case .rejected(.unsupportedKey):
                hint = "このキーは割り当てられません"
            }
        }
    }

    private func stop() {
        monitor.remove()
        recording = false
    }
}

/// 記録に必要な値だけを取り出した打鍵（`NSEvent` は Sendable でないので、境界で値にする）
struct RecordedKeyEvent: Sendable {
    var keyCode: UInt16
    var characters: String
    var modifiers: KeyModifiers

    init(_ event: NSEvent) {
        keyCode = event.keyCode
        characters = event.charactersIgnoringModifiers ?? ""
        var result: KeyModifiers = []
        let flags = event.modifierFlags
        if flags.contains(.shift) { result.insert(.shift) }
        if flags.contains(.control) { result.insert(.control) }
        if flags.contains(.option) { result.insert(.option) }
        if flags.contains(.command) { result.insert(.command) }
        modifiers = result
    }
}

@MainActor
final class EventMonitorBox {
    private var token: Any?

    /// 記録中の打鍵はすべて横取りし、アプリには渡さない
    func install(_ handler: @escaping @MainActor (RecordedKeyEvent) -> Void) {
        remove()
        token = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { event in
            let recorded = RecordedKeyEvent(event)
            MainActor.assumeIsolated { handler(recorded) }
            return nil
        }
    }

    func remove() {
        if let token { NSEvent.removeMonitor(token) }
        token = nil
    }
}
