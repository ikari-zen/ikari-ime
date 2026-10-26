// エンジンとの通信の事象の数（ADR 0034 §4.2 の `sh_engine`）。メモリ上の整数だけ。キーは engine の許可リスト（diag_schema.rs）に合わせる
import Foundation
import IkariShellCore

struct DiagEngineCounts: Equatable {
    static let kind = "sh_engine"

    /// 返事が時間内に来なかった
    var timeout = 0
    /// 相手のプロセスが落ちた（接続は有効なまま）
    var interrupted = 0
    /// 接続が無効になった
    var invalidated = 0
    /// 変換が失敗して、殻がエンジンなしの候補で補った
    var fallbackConvs = 0
    /// 連続して落ちて、エンジンを呼ばずに休み始めた
    var breakerOpen = 0
    /// 接続を作り直した
    var reconnect = 0
    /// 落ちたあとに、`Hello` とセッションを送り直した
    var resync = 0

    var isEmpty: Bool { self == DiagEngineCounts() }

    /// 送った分を引く（送っている間に増えた分は残す）
    mutating func subtract(_ sent: DiagEngineCounts) {
        timeout -= sent.timeout
        interrupted -= sent.interrupted
        invalidated -= sent.invalidated
        fallbackConvs -= sent.fallbackConvs
        breakerOpen -= sent.breakerOpen
        reconnect -= sent.reconnect
        resync -= sent.resync
    }

    /// 0 でないキーだけを数として持つ
    func event(sessionID: UInt64) -> DiagEventValue {
        let all: [String: Int] = [
            "timeout": timeout, "interrupted": interrupted, "invalidated": invalidated,
            "fallback_convs": fallbackConvs, "breaker_open": breakerOpen, "reconnect": reconnect, "resync": resync,
        ]
        return DiagEventValue(
            sessionID: sessionID, kind: Self.kind, nums: all.filter { $0.value != 0 }.mapValues { Int64($0) })
    }
}
