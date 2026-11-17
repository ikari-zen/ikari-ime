// XPC のプロトコル（ADR 0004 §5.1）。中身は protobuf（Envelope）のバイト列の `handle` が主で、
// もう 1 つ、檻役の匿名の口を渡す `attachLMCage` がある（ADR 0077 §3.2）。
import Foundation
@preconcurrency import XPC

@objc public protocol IkariEngineXPC {
    func handle(_ request: Data, reply: @escaping (Data) -> Void)

    /// 檻役の匿名の口（`xpc_endpoint`）をエンジンに渡す（殻が、`Hello` の前と、檻役との接続を取り直したあとに呼ぶ）。
    /// 中身は見ない輸送。口の先が本物の檻役かの確認と、LM の判断はエンジン（Rust）が持つ。古い殻は呼ばない
    /// （そのときエンジンは「口が無い → LM なし」）。返事の真偽は、エンジンが口を受け取ったか（確かめの結果ではない）
    func attachLMCage(_ endpoint: xpc_endpoint_t, reply: @escaping (Bool) -> Void)
}

public enum IkariEngineXPCInterface {
    /// 口の型（`xpc_endpoint`）を引数に宣言した NSXPCInterface。殻とエンジンの両方がこれと同じ宣言を使う
    public static func make() -> NSXPCInterface {
        let interface = NSXPCInterface(with: IkariEngineXPC.self)
        declareEndpoint(on: interface, selector: #selector(IkariEngineXPC.attachLMCage(_:reply:)))
        return interface
    }

    /// エンジン側のプロトコル（別の宣言。セレクタが同じなら接続できる）にも同じ宣言をするための部品
    public static func declareEndpoint(on interface: NSXPCInterface, selector: Selector) {
        interface.setXPCType(XPC_TYPE_ENDPOINT, for: selector, argumentIndex: 0, ofReply: false)
    }
}
