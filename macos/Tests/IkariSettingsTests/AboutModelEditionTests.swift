// 「このソフトについて」の版の名前と辞書の出どころ（ADR 0080 §3.2、§7.2）。判断は Rust（Capabilities はエンジン側が決めた値を注入する）で、
// ここは値を表示に直すところだけを確かめる
import Foundation
import IkariProto
import IkariShellABI
import Testing

@testable import IkariSettings

@Suite @MainActor struct AboutModelEditionTests {
    private static let normal = EditionCapabilities(edition: .normal, urlModelFetch: true, versionCheck: true, fileImport: true)
    private static let offline = EditionCapabilities(edition: .offline, urlModelFetch: false, versionCheck: false, fileImport: true)

    private func loaded(
        _ capabilities: EditionCapabilities, source: Ikari_Engine_V1_DictSource = .bundled, date: String = "2026-11-22"
    ) async throws -> AboutModel {
        var state = FakeState()
        state.dictSource = source
        state.dictDate = date
        let harness = try await Harness.make(state: state)
        defer { harness.close() }
        let model = AboutModel(bridge: harness.client, capabilities: capabilities, appVersion: "1.0.0", notices: "")
        await model.load()
        return model
    }

    @Test func theNormalEditionShowsNoEditionName() async throws {
        let model = try await loaded(Self.normal)
        #expect(model.editionName == nil)
    }

    @Test func theOfflineEditionIsNamed() async throws {
        let model = try await loaded(Self.offline)
        #expect(model.editionName == "オフライン版")
    }

    @Test func holdsTheCapabilitiesForFutureButtons() async throws {
        let model = try await loaded(Self.offline)
        #expect(model.capabilities.urlModelFetch == false)
        #expect(model.capabilities.versionCheck == false)
        #expect(model.capabilities.fileImport == true)
        let normalModel = try await loaded(Self.normal)
        #expect(normalModel.capabilities.urlModelFetch == true)
        #expect(normalModel.capabilities.versionCheck == true)
    }

    @Test func theBundledDictionaryShowsItsDate() async throws {
        let model = try await loaded(Self.normal, source: .bundled, date: "2026-11-22")
        #expect(model.dictSourceText == "同梱の辞書（2026-11-22）")
    }

    @Test func aDictionaryTheUserPutInIsNamedSo() async throws {
        let model = try await loaded(Self.normal, source: .user, date: "2026-12-01")
        #expect(model.dictSourceText == "入れた辞書（2026-12-01）")
    }

    @Test func anUnknownDateIsLeftOut() async throws {
        let model = try await loaded(Self.normal, source: .bundled, date: "")
        #expect(model.dictSourceText == "同梱の辞書")
        #expect(model.dictDate == nil)
    }

    @Test func anUnknownSourceShowsNothing() async throws {
        let model = try await loaded(Self.normal, source: .unspecified, date: "")
        #expect(model.dictSourceText == nil)
    }

    @Test func beforeLoadingNothingIsShown() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        let model = AboutModel(bridge: harness.client, capabilities: Self.normal, appVersion: "x", notices: "")
        #expect(model.dictSourceText == nil)
    }
}

#if IKARI_HAS_SHELL_FFI
    @Suite struct EditionCapabilitiesABITests {
        /// 殻の FFI から値が取れる。macOS の製品では、自由指定の取得は出さない（エンジンと同じ値。ADR 0080 §7.2）
        @Test func theABIReturnsValues() throws {
            let c = try #require(ShellSettings.editionCapabilities())
            #expect(c.fileImport == true)
            #expect(c.urlModelFetch == false)
        }
    }
#endif
