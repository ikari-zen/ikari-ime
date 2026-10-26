// XPC のプロトコル（ADR 0004 §5.1）。メソッドは1つだけで、中身は protobuf（Envelope）のバイト列。
import Foundation

@objc public protocol IkariEngineXPC {
    func handle(_ request: Data, reply: @escaping (Data) -> Void)
}
