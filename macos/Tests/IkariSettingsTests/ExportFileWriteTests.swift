import Foundation
import Testing

@testable import IkariSettings

@Suite struct ExportFileWriteTests {
    private func makeTempDir() throws -> URL {
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("ikari-export-write-tests-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        return url
    }

    @Test func writesPrivateFileAndReplacesExistingFileWith0600() throws {
        let dir = try makeTempDir()
        defer { try? FileManager.default.removeItem(at: dir) }

        let target = dir.appendingPathComponent("export.tsv")
        try Data("old".utf8).write(to: target)
        try FileManager.default.setAttributes([.posixPermissions: 0o644], ofItemAtPath: target.path)

        let payload = Data("#ikari-export\tschema=1\n".utf8)
        try DataFiles.writeExport(payload, to: target)

        #expect(try Data(contentsOf: target) == payload)
        let attrs = try FileManager.default.attributesOfItem(atPath: target.path)
        let perms = (attrs[.posixPermissions] as? NSNumber)?.intValue
        #expect(perms == 0o600)

        // 一時ファイルが残っていないこと
        let items = try FileManager.default.contentsOfDirectory(atPath: dir.path)
        #expect(items == ["export.tsv"])
    }

    @Test func writesEmptyFileWith0600() throws {
        let dir = try makeTempDir()
        defer { try? FileManager.default.removeItem(at: dir) }

        let target = dir.appendingPathComponent("empty.tsv")
        try DataFiles.writeExport(Data(), to: target)

        #expect(try Data(contentsOf: target).isEmpty)
        let attrs = try FileManager.default.attributesOfItem(atPath: target.path)
        let perms = (attrs[.posixPermissions] as? NSNumber)?.intValue
        #expect(perms == 0o600)
    }

    @Test func failsCleanlyWithoutLeavingTempFileWhenDestinationIsDirectory() throws {
        let dir = try makeTempDir()
        defer { try? FileManager.default.removeItem(at: dir) }

        let subdir = dir.appendingPathComponent("existing-dir")
        try FileManager.default.createDirectory(at: subdir, withIntermediateDirectories: true)

        #expect(throws: Error.self) {
            try DataFiles.writeExport(Data("abc".utf8), to: subdir)
        }
        let items = try FileManager.default.contentsOfDirectory(atPath: dir.path)
        #expect(items == ["existing-dir"])
    }
}
