// エンジン（Rust）の寿命の門（#51）。FFI に依存しない形にして、テストで確かめられるようにする。
//
// 使う側は `enter` で処理中の数を増やしてから資源を使い、終わったら `leave` で減らす。`close` は closed を
// 立てて新しい `enter` を断り、処理中の数が 0 になるのを待ってから資源を返す（呼び手が解放する）。
// 待ちが上限を超えたら資源を返さずに残す。解放後の使用（UAF）より、リークのほうが安全なため。
import Foundation

/// `EngineGate.close` の結果
enum EngineGateCloseOutcome<Resource> {
    /// 処理中がなくなった。呼び手が資源を解放する
    case release(Resource)
    /// 処理中が上限の時間までに終わらなかった。解放せずに残す（リーク）
    case leaked
    /// 解放する資源がない（開いていない、すでに閉じた）
    case nothing
}

final class EngineGate<Resource>: @unchecked Sendable {
    private let condition = NSCondition()
    private var resource: Resource?
    private var inFlight = 0
    private var closed = false

    init() {}

    /// 処理中の数を増やして資源を返す。資源がなければ `open` で開く（ロックの中で呼ぶ）。
    /// closed のあと、または開けなかったときは nil を返し、数は増やさない
    func enter(open: () -> Resource?) -> Resource? {
        condition.lock()
        defer { condition.unlock() }
        if closed { return nil }
        if resource == nil { resource = open() }
        guard let resource else { return nil }
        inFlight += 1
        return resource
    }

    /// `enter` が資源を返したときにだけ、1回呼ぶ
    func leave() {
        condition.lock()
        defer { condition.unlock() }
        precondition(inFlight > 0, "EngineGate.leave without enter")
        inFlight -= 1
        if inFlight == 0 { condition.broadcast() }
    }

    /// `enter` から `leave` までを `body` の前後で行う。断られたときは nil
    func withResource<T>(open: () -> Resource?, _ body: (Resource) -> T) -> T? {
        guard let resource = enter(open: open) else { return nil }
        defer { leave() }
        return body(resource)
    }

    /// closed を立て、処理中が 0 になるのを `timeout` まで待つ。すでに解放を返したあとは `.nothing`。
    /// `.leaked` のあとに呼び直せば、そのときに処理中がなければ解放を返す
    func close(timeout: TimeInterval) -> EngineGateCloseOutcome<Resource> {
        condition.lock()
        defer { condition.unlock() }
        closed = true
        let deadline = Date(timeIntervalSinceNow: timeout)
        while inFlight > 0 {
            // 偽の起床もあるので、時刻で判断する
            if !condition.wait(until: deadline), inFlight > 0 { return .leaked }
        }
        guard let closing = resource else { return .nothing }
        resource = nil
        return .release(closing)
    }

    var isClosed: Bool {
        condition.lock()
        defer { condition.unlock() }
        return closed
    }
}
