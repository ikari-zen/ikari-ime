// XPC 経由の E2E（ADR 0004 §10.1）。ikari.app/Contents/MacOS/ikari-smoke に置き、同梱の XPC サービスにつなぐ。
//
//   ikari-smoke                          E2E を流す
//   ikari-smoke --recovery-limit-ms N    kill -9 からの復帰の上限（既定 1000。測定のときだけ長くする）
//   ikari-smoke --print-requirement engine|shell|settings team|adhoc
//                                        署名要件の文字列を出す（build.sh が XPC サービスの Info.plist に書く。
//                                        settings は build.sh が設定アプリの署名を検査するのに使う）
//
// 終了コード: 0 = FAIL がない、1 = FAIL がある、2 = 使い方の誤り、または実行できない（エンジンのロックが使用中）。
import Foundation
import IkariXPC

// MARK: 引数

enum SmokeConstants {
    static let context = "ユーザー辞書に単語を登録するときは"
    static let reading = "ひんし"
    static let expectedTopCandidate = "品詞"
    static let sessionID: UInt64 = 1
    static let mismatchVersion = "0.0.0-smoke-mismatch"
    /// 契約: kill -9 から 1 秒以内に再接続して変換できる
    static let recoveryLimit: Duration = .seconds(1)
    /// launchd は起動して間もなく（約 10 秒以内）に終わったサービスの再起動を遅らせる（ThrottleInterval）。
    /// 試験の前に、エンジンがこれより長く動いていることを保証する
    static let launchdThrottleWindow: Duration = .milliseconds(10_500)
    static let helloTimeout: Duration = .seconds(10)
    static let firstConvertTimeout: Duration = .seconds(5)
    /// LM の読み込み（Hello の `lm` が ready になる）を待つ上限
    static let lmReadyTimeout: Duration = .seconds(30)
    static let lmReadyPollInterval: Duration = .milliseconds(200)
    static let latencySamples = 50
    /// エンジンがデータのディレクトリで flock するファイル（engine/src/lock.rs の LOCK_FILE_NAME と同じ）
    static let engineLockPath = "Library/Application Support/ikari-ime/engine.lock"
    static let typingInterval: Duration = .milliseconds(120)
    /// NFR-02: 短い読みの p99
    static let latencyBudgetMillis = 16.0
}

let arguments = Array(CommandLine.arguments.dropFirst())

if arguments.first == "--print-requirement" {
    guard arguments.count == 3, let mode = SigningMode(rawValue: arguments[2]) else {
        FileHandle.standardError.write(Data("usage: ikari-smoke --print-requirement engine|shell|settings team|adhoc\n".utf8))
        exit(2)
    }
    switch arguments[1] {
    case "engine": print(SigningRequirements.engine(mode: mode))
    case "shell": print(SigningRequirements.shell(mode: mode))
    case "settings": print(SigningRequirements.settingsApp(mode: mode))
    default:
        FileHandle.standardError.write(Data("unknown target: \(arguments[1])\n".utf8))
        exit(2)
    }
    exit(0)
}

/// kill -9 から戻るまでの上限。契約は 1 秒。長くして実際の所要時間を測るときだけ変える
nonisolated(unsafe) var recoveryLimit = SmokeConstants.recoveryLimit
var remaining = arguments[...]
while let argument = remaining.popFirst() {
    switch argument {
    case "--recovery-limit-ms":
        guard let text = remaining.popFirst(), let value = Int(text), value > 0 else {
            FileHandle.standardError.write(Data("--recovery-limit-ms needs a positive integer\n".utf8))
            exit(2)
        }
        recoveryLimit = .milliseconds(value)
    default:
        FileHandle.standardError.write(Data("unknown argument: \(argument)\n".utf8))
        exit(2)
    }
}

// MARK: エンジンのロックの確認

/// ikari を使っているあいだは、その IME のエンジンが engine.lock を持つ。smoke が起動したエンジンは open に失敗し、
/// hello が ENGINE_NOT_READY の FAIL になる（設計どおりの挙動）。FAIL と区別するため、先にロックを試して実行を止める。
func engineLockIsHeld() -> Bool {
    let path = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(SmokeConstants.engineLockPath).path
    let descriptor = open(path, O_RDONLY)
    guard descriptor >= 0 else { return false }  // ファイルがなければ誰も持っていない
    defer { close(descriptor) }
    if flock(descriptor, LOCK_EX | LOCK_NB) == 0 {
        flock(descriptor, LOCK_UN)
        return false
    }
    return errno == EWOULDBLOCK
}

if engineLockIsHeld() {
    FileHandle.standardError.write(Data(
        "ikari-smoke: 実行できない: エンジンのロックが使用中（ikari を ABC などに切り替えてから実行する）\n".utf8))
    exit(2)
}

// MARK: 結果の出力

final class Report: @unchecked Sendable {
    private(set) var failures = 0
    private(set) var skips = 0

    func pass(_ name: String, _ detail: String = "") { print("PASS  \(name)\(suffix(detail))") }
    func skip(_ name: String, _ detail: String) {
        skips += 1
        print("SKIP  \(name)\(suffix(detail))")
    }
    func fail(_ name: String, _ detail: String) {
        failures += 1
        print("FAIL  \(name)\(suffix(detail))")
    }
    func info(_ text: String) { print("      \(text)") }
    private func suffix(_ detail: String) -> String { detail.isEmpty ? "" : "  (\(detail))" }
}

let report = Report()
let signing = BuildSigning.current()
let shellVersion =
    (Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String) ?? "unknown"
let client = XPCEngineClient(signing: signing)

func seconds(_ duration: Duration) -> Double {
    let (whole, atto) = duration.components
    return Double(whole) + Double(atto) / 1e18
}

func millis(_ duration: Duration) -> Double {
    seconds(duration) * 1000
}

func isNotReady(_ error: Error) -> Bool {
    if case EngineClientError.engine(.engineNotReady) = error { return true }
    return false
}

func request(_ reading: String, contextRev: UInt64) -> ConversionRequest {
    ConversionRequest(
        sessionID: SmokeConstants.sessionID,
        reading: reading,
        context: SurroundingText(left: SmokeConstants.context, right: "", available: true),
        contextRev: contextRev
    )
}

let session = SessionInfo(
    sessionID: SmokeConstants.sessionID, clientBundleID: IkariIdentifiers.smoke, secure: false)

// MARK: 1. 接続と Hello（署名要件を通ること）

print("ikari-smoke: signing=\(signing.rawValue) version=\(shellVersion)")
nonisolated(unsafe) var engineReady = false
nonisolated(unsafe) var lmEnabled = false
do {
    let hello = try client.hello(shellVersion: shellVersion, timeout: SmokeConstants.helloTimeout)
    engineReady = true
    lmEnabled = hello.lm != .disabled && hello.lm != .unspecified
    report.pass("hello", "engine=\(hello.engineVersion) dict=\(hello.dictLabel) lm=\(hello.lm) pid=\(client.enginePID)")
} catch where isNotReady(error) {
    report.fail("hello", "ENGINE_NOT_READY")
} catch {
    if client.lastEngineVersion != nil {
        report.fail("hello", "version mismatch: engine=\(client.lastEngineVersion ?? "?") shell=\(shellVersion)")
    } else {
        report.fail("hello", "\(error)")
    }
}

// MARK: 2. 文脈つきの変換と確定

/// LM が有効なエンジンについて、読み込みが終わる（Hello の lm が ready）まで待つ。上限つき。
/// 戻り値は ready になったか。LM が無効なら true（待つ必要がない）
func waitForLM(_ name: String) -> Bool {
    guard lmEnabled else { return true }
    let deadline = ContinuousClock.now + SmokeConstants.lmReadyTimeout
    var last = "?"
    while ContinuousClock.now < deadline {
        if let hello = try? client.hello(shellVersion: shellVersion, timeout: SmokeConstants.helloTimeout) {
            last = "\(hello.lm)"
            if hello.lm == .ready { return true }
            if hello.lm == .failed { break }
        }
        Thread.sleep(forTimeInterval: seconds(SmokeConstants.lmReadyPollInterval))
    }
    report.fail(name, "LM did not become ready within \(SmokeConstants.lmReadyTimeout) (lm=\(last))")
    return false
}

func contextConversion() {
    client.beginSession(session)
    if lmEnabled, waitForLM("lm-ready") { report.pass("lm-ready", "lm=ready") }
    client.prefetch(request(SmokeConstants.reading, contextRev: 1))
    Thread.sleep(forTimeInterval: 0.2)
    do {
        var page = try client.convert(
            request(SmokeConstants.reading, contextRev: 1), timeout: SmokeConstants.firstConvertTimeout)
        // 読み込み直後の最初の先行計算は、LM の初回の採点（Metal のパイプラインの準備）で遅れることがある。
        // その間の Convert は LM を待たずに返るので、先行計算が終わるまで同じ要求で繰り返す（上限つき）
        if lmEnabled {
            let deadline = ContinuousClock.now + SmokeConstants.lmReadyTimeout
            while !page.lmApplied, ContinuousClock.now < deadline {
                Thread.sleep(forTimeInterval: seconds(SmokeConstants.lmReadyPollInterval))
                page = try client.convert(
                    request(SmokeConstants.reading, contextRev: 1), timeout: SmokeConstants.firstConvertTimeout)
            }
        }
        let top = page.candidates.first?.surface ?? "(none)"
        if top == SmokeConstants.expectedTopCandidate, lmEnabled, !page.lmApplied {
            report.fail("convert-with-context", "top=\(top) but lm=false although the LM is ready")
        } else if top == SmokeConstants.expectedTopCandidate {
            report.pass(
                "convert-with-context",
                "top=\(top) candidates=\(page.candidates.count) lm=\(page.lmApplied) engine=\(page.engineMicros)us")
        } else {
            report.fail(
                "convert-with-context",
                "top=\(top) expected=\(SmokeConstants.expectedTopCandidate) lm=\(page.lmApplied)")
        }
        // 次のページを取ってから確定する（Commit で変換は閉じるので、確定後のページ取得は unknownConversion）
        if page.hasNext {
            let next = try client.page(
                conversionID: page.conversionID, index: 1, timeout: SmokeConstants.firstConvertTimeout)
            report.pass("page", "page=\(next.pageIndex) candidates=\(next.candidates.count)")
        }
        // 確定。返事を待たない通知なので、直後の変換などで続けて通ることで確かめる
        client.commit(conversionID: page.conversionID, candidateIndex: 0)
        client.commitText(sessionID: SmokeConstants.sessionID, reading: "かな", surface: "かな")
        report.pass("commit", "conversion=\(page.conversionID)")
    } catch {
        report.fail("convert-with-context", "\(error)")
    }
}

if engineReady {
    contextConversion()
} else {
    report.skip("convert-with-context", "engine not ready")
}

// MARK: 2b. 管理の E2E（ADR 0005 §10.1。見張りの語の追加と削除だけを書き、インポートは dry_run だけ）

if engineReady {
    AdminE2E(
        client: client, report: report,
        convertContext: SurroundingText(left: SmokeConstants.context, right: "", available: true),
        convertTimeout: SmokeConstants.firstConvertTimeout
    ).run()
} else {
    report.skip("admin", "engine not ready")
}

// MARK: 2c. 文節と再変換の E2E（ADR 0006 §10.1。確定は secure のセッションで行い、学習を汚さない）

if engineReady {
    SegmentE2E(
        client: client, report: report,
        context: SurroundingText(left: SmokeConstants.context, right: "", available: true),
        timeout: SmokeConstants.firstConvertTimeout
    ).run()
} else {
    report.skip("segments", "engine not ready")
}

// MARK: 3. kill -9 からの復帰（1 秒以内）

/// プロセスが動き始めてからの時間（取れなければ nil）
func processAge(_ pid: Int32) -> Duration? {
    var info = kinfo_proc()
    var size = MemoryLayout<kinfo_proc>.stride
    var mib: [Int32] = [CTL_KERN, KERN_PROC, KERN_PROC_PID, pid]
    guard sysctl(&mib, UInt32(mib.count), &info, &size, nil, 0) == 0, size > 0 else { return nil }
    let started = info.kp_proc.p_un.__p_starttime
    let startDate = Date(timeIntervalSince1970: TimeInterval(started.tv_sec) + TimeInterval(started.tv_usec) / 1e6)
    return .seconds(Date().timeIntervalSince(startDate))
}

/// エンジンが十分長く動いていることを待つ。起動直後の kill や終了は launchd に再起動を遅らされ、
/// 「1 秒以内の復帰」を測れない（実際の利用ではエンジンはもっと長く動いている）
func waitUntilEngineIsOld(_ pid: Int32) {
    guard let age = processAge(pid), age < SmokeConstants.launchdThrottleWindow else { return }
    let wait = SmokeConstants.launchdThrottleWindow - age
    report.info(String(format: "waiting %.1fs so that launchd's restart throttle does not apply", millis(wait) / 1000))
    Thread.sleep(forTimeInterval: seconds(wait))
}

func recoveryAfterKill() {
    let oldPID = client.enginePID
    waitUntilEngineIsOld(oldPID)
    guard oldPID > 0 else {
        report.fail("kill-9-recovery", "no engine pid")
        return
    }
    let started = ContinuousClock.now
    if kill(oldPID, SIGKILL) != 0 {
        report.fail("kill-9-recovery", "kill(\(oldPID)) failed errno=\(errno)")
        return
    }
    var recovered = false
    var lastError = "none"
    while ContinuousClock.now - started < recoveryLimit {
        do {
            if engineReady {
                // 変換できるところまで戻ること（セッションの送り直しも含めて）
                let page = try client.convert(
                    request(SmokeConstants.reading, contextRev: 2), timeout: .milliseconds(300))
                recovered = !page.candidates.isEmpty
            } else {
                _ = try client.hello(shellVersion: shellVersion, timeout: .milliseconds(300))
                recovered = true
            }
        } catch where !engineReady && isNotReady(error) {
            recovered = true  // スタブでも、新しいプロセスが返事をした
        } catch {
            lastError = "\(error)"
            Thread.sleep(forTimeInterval: 0.02)
        }
        if recovered { break }
    }
    let elapsed = millis(ContinuousClock.now - started)
    let newPID = client.enginePID
    let detail = String(format: "%.0fms old=%d new=%d", elapsed, oldPID, newPID)
    if recovered, newPID != oldPID, newPID > 0 {
        report.pass("kill-9-recovery", detail)
    } else {
        report.fail("kill-9-recovery", "\(detail) lastError=\(lastError)")
    }
}

recoveryAfterKill()

// MARK: 4. 版の不一致（Shutdown が働き、新しいプロセスに替わる）

func versionMismatch() {
    let oldPID = client.enginePID
    waitUntilEngineIsOld(oldPID)
    do {
        _ = try client.hello(shellVersion: SmokeConstants.mismatchVersion, timeout: SmokeConstants.helloTimeout)
        report.fail("version-mismatch", "a mismatched hello was accepted")
        return
    } catch EngineClientError.unavailable {
        // 期待どおり: Shutdown して起動し直したが、版は合わないまま
    } catch {
        if isNotReady(error) {
            report.skip("version-mismatch (hello)", "engine is a contract stub; the version cannot be compared")
            versionShutdownOnly(oldPID: oldPID)
        } else {
            report.fail("version-mismatch", "\(error)")
        }
        return
    }
    let newPID = client.enginePID
    guard newPID > 0, newPID != oldPID else {
        report.fail("version-mismatch", "engine was not restarted old=\(oldPID) new=\(newPID)")
        return
    }
    report.pass("version-mismatch", "restarted old=\(oldPID) new=\(newPID)")

    // 実際の版で照合し直せば通る。バンドルの版とエンジンの版が違うなら、それ自体が問題なので知らせる
    let actual = client.lastEngineVersion ?? shellVersion
    do {
        _ = try client.hello(shellVersion: actual, timeout: SmokeConstants.helloTimeout)
        report.pass("hello-after-restart", "engine=\(actual)")
        if actual != shellVersion {
            report.info("note: engine_version (\(actual)) differs from the bundle version (\(shellVersion))")
        }
    } catch {
        report.fail("hello-after-restart", "\(error)")
    }
}

/// スタブのときも、Shutdown がエンジンのプロセスを替えることだけは確かめる
func versionShutdownOnly(oldPID: Int32) {
    waitUntilEngineIsOld(oldPID)
    client.shutdown(reason: .versionMismatch)
    Thread.sleep(forTimeInterval: 0.3)
    do {
        _ = try client.hello(shellVersion: shellVersion, timeout: SmokeConstants.helloTimeout)
    } catch {
        // ENGINE_NOT_READY でよい。新しいプロセスが返事をすれば足りる
    }
    let newPID = client.enginePID
    if newPID > 0, newPID != oldPID {
        report.pass("shutdown-restarts-engine", "old=\(oldPID) new=\(newPID)")
    } else {
        report.fail("shutdown-restarts-engine", "old=\(oldPID) new=\(newPID)")
    }
}

versionMismatch()

// MARK: 5. 打鍵のシミュレーションでの Convert の時間（XPC の往復を含む）

func latency() {
    client.beginSession(session)
    // kill -9 のあとの再起動でモデルが読み直されるので、LM ありの時間を測るために待つ
    guard waitForLM("latency") else { return }
    var samples: [Double] = []
    var failures = 0
    for index in 0..<SmokeConstants.latencySamples {
        let rev = UInt64(100 + index)
        client.prefetch(request(SmokeConstants.reading, contextRev: rev))
        Thread.sleep(forTimeInterval: seconds(SmokeConstants.typingInterval))
        let started = ContinuousClock.now
        do {
            _ = try client.convert(
                request(SmokeConstants.reading, contextRev: rev),
                timeout: EngineTimeouts.convertTimeout(readingCharacterCount: SmokeConstants.reading.count))
            samples.append(millis(ContinuousClock.now - started))
        } catch {
            failures += 1
        }
    }
    guard !samples.isEmpty else {
        report.fail("latency", "no successful samples (failures=\(failures))")
        return
    }
    samples.sort()
    let p50 = samples[samples.count / 2]
    let p99 = samples[min(samples.count - 1, Int(Double(samples.count) * 0.99))]
    let detail = String(
        format: "n=%d p50=%.2fms p99=%.2fms max=%.2fms failures=%d", samples.count, p50, p99, samples.last ?? 0, failures)
    // p99 の予算は完了条件。ここでは計測値を報告し、予算超過と失敗は FAIL にする
    if failures == 0, p99 <= SmokeConstants.latencyBudgetMillis {
        report.pass("latency", detail)
    } else {
        report.fail("latency", "\(detail) budget=\(SmokeConstants.latencyBudgetMillis)ms")
    }
}

if engineReady {
    latency()
} else {
    report.skip("latency", "engine not ready")
}

client.endSession(sessionID: SmokeConstants.sessionID)
print("ikari-smoke: \(report.failures) failed, \(report.skips) skipped")
exit(report.failures == 0 ? 0 : 1)
