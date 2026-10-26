import Foundation
import IkariProto
import Testing

@testable import IkariBridge

private func exportFrame(id: UInt64, size: Int) -> Frame {
    var reply = Ikari_Engine_V1_ExportReply()
    reply.content = Data(repeating: UInt8(id & 0xFF), count: size)
    var envelope = EngineEnvelope()
    envelope.requestID = id
    envelope.exportReply = reply
    var frame = Frame()
    frame.requestID = id
    frame.engine = envelope
    return frame
}

private func eventFrame() -> Frame {
    var open = Ikari_Bridge_V1_OpenTab()
    open.tab = "dictionary"
    var event = Ikari_Bridge_V1_Event()
    event.openTab = open
    var frame = Frame()
    frame.event = event
    return frame
}

@Suite struct BridgeTransportTests {
    @Test func sendsAndReceivesFramesInOrder() throws {
        let (a, b) = try FileDescriptorTransport.socketPair()
        for id in 1...5 { try a.send(exportFrame(id: UInt64(id), size: 10)) }
        for id in 1...5 { #expect(try b.receive().requestID == UInt64(id)) }
        try b.send(eventFrame())
        #expect(try a.receive() == eventFrame())
    }

    @Test func carriesAFrameLargerThanTheSocketBuffer() throws {
        let (a, b) = try FileDescriptorTransport.socketPair()
        let big = exportFrame(id: 7, size: 3 * 1024 * 1024)
        // 送る側は受ける側が読むまで止まるので、別のスレッドで送る
        let sender = Thread { try? a.send(big) }
        sender.start()
        #expect(try b.receive() == big)
    }

    @Test func concurrentSendersDoNotInterleaveFrames() throws {
        let (a, b) = try FileDescriptorTransport.socketPair()
        let senders = 8
        let perSender = 40
        let group = DispatchGroup()
        for sender in 0..<senders {
            group.enter()
            Thread {
                for index in 0..<perSender {
                    try? a.send(exportFrame(id: UInt64(sender * 1000 + index), size: 20_000))
                }
                group.leave()
            }.start()
        }
        var received = Set<UInt64>()
        for _ in 0..<(senders * perSender) {
            let frame = try b.receive()
            guard case .engine(let envelope)? = frame.body, case .exportReply(let reply)? = envelope.body else {
                Issue.record("unexpected frame")
                return
            }
            #expect(reply.content.count == 20_000)
            #expect(reply.content.allSatisfy { $0 == UInt8(frame.requestID & 0xFF) })
            received.insert(frame.requestID)
        }
        group.wait()
        #expect(received.count == senders * perSender)
    }

    @Test func reportsClosedWhenThePeerCloses() throws {
        let (a, b) = try FileDescriptorTransport.socketPair()
        try a.send(eventFrame())
        a.close()
        #expect(try b.receive() == eventFrame())  // 閉じる前に送ったものは読める
        #expect(throws: BridgeTransportError.closed) { try b.receive() }
    }

    @Test func closeWakesAReceiverThatIsWaiting() throws {
        let (a, _) = try FileDescriptorTransport.socketPair()
        let result = ResultBox()
        let done = DispatchSemaphore(value: 0)
        Thread {
            do { _ = try a.receive() } catch { result.set(error) }
            done.signal()
        }.start()
        Thread.sleep(forTimeInterval: 0.05)
        a.close()
        #expect(done.wait(timeout: .now() + 2) == .success)
        #expect(result.get() as? BridgeTransportError == .closed)
    }

    @Test func sendAfterCloseFails() throws {
        let (a, _) = try FileDescriptorTransport.socketPair()
        a.close()
        #expect(throws: BridgeTransportError.closed) { try a.send(eventFrame()) }
        a.close()  // 二度閉じてもよい
    }

    @Test func aBrokenLengthEndsTheConnectionWithAFrameError() throws {
        var fds: [Int32] = [0, 0]
        #expect(socketpair(AF_UNIX, SOCK_STREAM, 0, &fds) == 0)
        let receiver = FileDescriptorTransport(descriptor: fds[0])
        defer { Darwin.close(fds[1]) }
        let bytes: [UInt8] = [0xFF, 0xFF, 0xFF, 0xFF]
        #expect(write(fds[1], bytes, bytes.count) == bytes.count)
        #expect(throws: BridgeTransportError.frame(.tooLarge(length: UInt64(UInt32.max)))) { try receiver.receive() }
    }

    @Test func refusesToSendAnOversizedFrame() throws {
        let (a, _) = try FileDescriptorTransport.socketPair()
        let huge = exportFrame(id: 1, size: BridgeLimits.maxFrameBytes + 1)
        #expect(throws: BridgeTransportError.self) { try a.send(huge) }
    }
}

private final class ResultBox: @unchecked Sendable {
    private let lock = NSLock()
    private var error: Error?
    func set(_ value: Error) { lock.withLock { error = value } }
    func get() -> Error? { lock.withLock { error } }
}
