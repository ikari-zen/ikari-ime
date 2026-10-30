// Swift 版の殻の性能の測り手（ADR 0041 §6.3。P1・P3・P4）。移行の間だけ使い、M5 で消す。
//
// 共通のシナリオ（shell-core/scenarios/*.json）を、替え玉のエンジンで再生し、キー 1 回ごとの時間を集める。
// Rust 版の測り手（shell-core/benches/shell_perf.rs）と同じ手順・同じ出力の形で、`cargo xtask perf shell` が両方を呼んで比べる。
//
// - P1: キー 1 回の `InputStateMachine.handle` の時間（ナノ秒）。同期のエンジン要求（convert、page、segment_page、resize_segment、
//   reconvert、postal_lookup）を送ったキーは除く（ADR 0041 §6.3「Convert を含まない」）。シナリオ全部を `--repeat` 回くり返す
// - P3: 殻の初期化（設定の解釈、キー割り当ての読み込み、ショートコードの表、状態機械の生成、エンジンへの hello）の時間。`--repeat` 回
// - P4: そのプロセスの最大 RSS（`getrusage`）
//
// Doubles.swift と Scenario.swift は、テストの Support への symlink（同じ替え玉と読み手を使う）。`@testable` を使うので、
// `-Xswiftc -enable-testing` を付けてビルドする（`cargo xtask perf shell` が付ける）。
//
// 使い方: ikari-shell-bench --metric p1|p3 --scenarios <dir> --resources <dir> [--repeat 100] [--shortcode-entries 2000]
// 出力: 標準出力に JSON を 1 つ（`{"implementation": "swift", "metric": …, "p50_ns": …, "p99_ns": …, "max_ns": …, …}`）
import Foundation

@testable import IkariShellCore

// MARK: シナリオの読み込み（ディレクトリを指定する。`ScenarioFiles.load()` は #filePath に頼るので使わない）

extension ScenarioFiles {
    static func load(from directory: URL) throws -> [(name: String, file: ScenarioFile)] {
        let urls = try FileManager.default.contentsOfDirectory(at: directory, includingPropertiesForKeys: nil)
            .filter { $0.pathExtension == "json" }
            .sorted { $0.lastPathComponent < $1.lastPathComponent }
        return try urls.map { url in
            do {
                return (url.lastPathComponent, try JSONDecoder().decode(ScenarioFile.self, from: Data(contentsOf: url)))
            } catch {
                throw ScenarioFormatError("\(url.lastPathComponent): \(error)")
            }
        }
    }
}

// MARK: 引数

struct BenchOptions {
    var metric = "p1"
    var scenarios = ""
    var resources = ""
    var repeatCount = 100
    var shortcodeEntries = 2000

    static func parse(_ arguments: [String]) throws -> BenchOptions {
        var options = BenchOptions()
        var index = 0
        func value(_ name: String) throws -> String {
            index += 1
            guard index < arguments.count else { throw ScenarioFormatError("\(name) needs a value") }
            return arguments[index]
        }
        while index < arguments.count {
            switch arguments[index] {
            case "--metric": options.metric = try value("--metric")
            case "--scenarios": options.scenarios = try value("--scenarios")
            case "--resources": options.resources = try value("--resources")
            case "--repeat":
                guard let count = Int(try value("--repeat")), count > 0 else { throw ScenarioFormatError("bad --repeat") }
                options.repeatCount = count
            case "--shortcode-entries":
                guard let count = Int(try value("--shortcode-entries")), count >= 0 else {
                    throw ScenarioFormatError("bad --shortcode-entries")
                }
                options.shortcodeEntries = count
            case let other: throw ScenarioFormatError("unknown argument \(other)")
            }
            index += 1
        }
        guard !options.scenarios.isEmpty, !options.resources.isEmpty else {
            throw ScenarioFormatError("--scenarios and --resources are required")
        }
        return options
    }
}

// MARK: 時計と統計

@inline(__always) func nowNanos() -> UInt64 { clock_gettime_nsec_np(CLOCK_UPTIME_RAW) }

/// 順位の最も近い値（最近傍順位法）。`sorted` は昇順で空でない
func percentile(_ sorted: [UInt64], _ fraction: Double) -> UInt64 {
    let rank = Int((fraction * Double(sorted.count)).rounded(.up))
    return sorted[min(max(rank, 1), sorted.count) - 1]
}

func maxRSSBytes() -> UInt64? {
    var usage = rusage()
    guard getrusage(RUSAGE_SELF, &usage) == 0 else { return nil }
    return UInt64(usage.ru_maxrss)  // macOS はバイト
}

// MARK: 台本つきの替え玉のエンジン

/// 台本の失敗を、要求の種類ごとの回数で引く（ScenarioRunnerTests と同じ）
final class FailureCounter: @unchecked Sendable {
    private let lock = NSLock()
    private let script: ScenarioEngineScript
    private var counts: [String: Int] = [:]

    init(_ script: ScenarioEngineScript) { self.script = script }

    func next(_ request: String) -> EngineClientError? {
        lock.lock()
        defer { lock.unlock() }
        counts[request, default: 0] += 1
        return script.failure(for: request, nth: counts[request]!)
    }
}

func makeEngine(_ setup: ResolvedScenarioSetup) -> MockEngine {
    let engine = MockEngine()
    let script = FailureCounter(setup.engine)
    let pages = setup.engine.pages
    engine.onConvert = { _ in
        if let error = script.next("convert") { throw error }
        guard let page = pages.first else { throw EngineClientError.engine(.badRequest) }
        return page.conversionPage
    }
    engine.onPage = { _, index in
        if let error = script.next("page") { throw error }
        guard pages.indices.contains(index) else { throw EngineClientError.engine(.badRequest) }
        return pages[index].conversionPage
    }
    engine.onSegmentPage = { _, _, _ in throw script.next("segment_page") ?? EngineClientError.engine(.badRequest) }
    engine.onResize = { _, _, _ in throw script.next("resize_segment") ?? EngineClientError.engine(.badRequest) }
    engine.onReconvert = { _ in throw script.next("reconvert") ?? EngineClientError.engine(.badRequest) }
    engine.onPostalLookup = { _ in
        if let error = script.next("postal_lookup") { throw error }
        return []
    }
    return engine
}

/// 返事を待つ（同期の）要求か。P1 では、これを送ったキーを除く
func isSynchronousRequest(_ call: MockEngine.Call) -> Bool {
    switch call {
    case .convert, .page, .segmentPage, .resize, .reconvert, .postalLookup: true
    default: false
    }
}

func loadKeymap(_ settings: Settings, resources: String) throws -> Keymap {
    let url = URL(fileURLWithPath: resources).appendingPathComponent("keymap/\(settings.keymapPreset.rawValue).tsv")
    return try Keymap(presetTSV: String(contentsOf: url, encoding: .utf8), settings: settings)
}

// MARK: P1

struct P1Result {
    var samples: [UInt64] = []
    var excluded = 0
}

func measureKeys(_ options: BenchOptions) throws -> P1Result {
    let files = try ScenarioFiles.load(from: URL(fileURLWithPath: options.scenarios))
    var result = P1Result()
    var sink = 0
    // 同じ設定のキー割り当てを使い回すと、読み込みの時間がキーの時間に混ざらない（読み込みは P3）
    var keymaps: [String: (Settings, Keymap)] = [:]
    for _ in 0..<options.repeatCount {
        for (_, file) in files {
            let defaults = file.defaults ?? ScenarioSetup()
            for scenario in file.scenarios {
                let setup = defaults.resolve(scenario.setup)
                let tsv = setup.settings.joined(separator: "\n")
                if keymaps[tsv] == nil {
                    let parsed = Settings.parse(tsv: tsv)
                    keymaps[tsv] = (parsed.settings, try loadKeymap(parsed.settings, resources: options.resources))
                }
                let (settings, keymap) = keymaps[tsv]!
                let engine = makeEngine(setup)
                let composer = FakeComposer()
                let machine = InputStateMachine(
                    engine: engine, composer: composer, scripts: FakeScripts(), keymap: keymap, settings: settings,
                    session: SessionInfo(sessionID: 42, clientBundleID: setup.client.bundleID, secure: setup.client.secure))
                var surrounding = setup.surrounding.machineValue
                for step in scenario.steps {
                    if let given = step.surrounding { surrounding = given.machineValue }
                    let selection = step.selection?.machineValue
                    for key in try step.keys() {
                        let before = engine.calls.count
                        let started = nowNanos()
                        let actions = machine.handle(key, surrounding: surrounding, selection: selection, isRepeat: false)
                        let elapsed = nowNanos() - started
                        sink &+= actions.count
                        if engine.calls.dropFirst(before).contains(where: isSynchronousRequest) {
                            result.excluded += 1
                        } else {
                            result.samples.append(elapsed)
                        }
                    }
                }
            }
        }
    }
    FileHandle.standardError.write(Data("sink=\(sink)\n".utf8))
    return result
}

// MARK: P3

func measureStartup(_ options: BenchOptions) throws -> [UInt64] {
    // 設定のファイルは、代表的な数行（既定と違う値を含む）。解釈の時間を測る
    let settingsTSV = "paging.size\t3\nkeymap.preset\tmacos\nengine.lm\ttrue\nemoji.shortcode\ttrue\n"
    // ショートコードの表は、エンジンの返事から作る。生成は測らず、`ShortcodeTable` の構築と状態機械への受け渡しを測る
    let entries = (0..<options.shortcodeEntries).map {
        ShortcodeEntry(name: String(format: "name%06d", $0), emoji: "😀")
    }
    var samples: [UInt64] = []
    var sink = 0
    for _ in 0..<options.repeatCount {
        let engine = MockEngine()
        let started = nowNanos()
        let parsed = Settings.parse(tsv: settingsTSV)
        let keymap = try loadKeymap(parsed.settings, resources: options.resources)
        let table = ShortcodeTable(entries: entries)
        let machine = InputStateMachine(
            engine: engine, composer: FakeComposer(), scripts: FakeScripts(), keymap: keymap, settings: parsed.settings,
            session: SessionInfo(sessionID: 42, clientBundleID: "com.apple.TextEdit", secure: false))
        machine.setShortcodeTable(table)
        let hello = try engine.hello(shellVersion: "bench", timeout: .seconds(1))
        let elapsed = nowNanos() - started
        sink &+= hello.engineVersion.count + table.entries.count
        samples.append(elapsed)
    }
    FileHandle.standardError.write(Data("sink=\(sink)\n".utf8))
    return samples
}

// MARK: 出力

func report(metric: String, samples: [UInt64], excluded: Int, options: BenchOptions) throws {
    guard !samples.isEmpty else { throw ScenarioFormatError("no samples") }
    let sorted = samples.sorted()
    var object: [String: Any] = [
        "implementation": "swift",
        "metric": metric,
        "samples": sorted.count,
        "excluded_samples": excluded,
        "repeat": options.repeatCount,
        "p50_ns": percentile(sorted, 0.5),
        "p99_ns": percentile(sorted, 0.99),
        "max_ns": sorted.last!,
    ]
    if metric == "p3" { object["shortcode_entries"] = options.shortcodeEntries }
    if let rss = maxRSSBytes() { object["max_rss_bytes"] = rss }
    let data = try JSONSerialization.data(withJSONObject: object, options: [.sortedKeys])
    FileHandle.standardOutput.write(data)
    FileHandle.standardOutput.write(Data("\n".utf8))
}

do {
    let options = try BenchOptions.parse(Array(CommandLine.arguments.dropFirst()))
    switch options.metric {
    case "p1":
        let result = try measureKeys(options)
        try report(metric: "p1", samples: result.samples, excluded: result.excluded, options: options)
    case "p3":
        try report(metric: "p3", samples: try measureStartup(options), excluded: 0, options: options)
    default:
        throw ScenarioFormatError("unknown metric \(options.metric)")
    }
} catch {
    FileHandle.standardError.write(Data("error: \(error)\n".utf8))
    exit(1)
}
