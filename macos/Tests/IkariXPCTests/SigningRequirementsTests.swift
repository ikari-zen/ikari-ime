import Foundation
import Testing

@testable import IkariXPC

@Suite struct SigningRequirementsTests {
    @Test func teamRequirementsPinTheTeamID() {
        #expect(
            SigningRequirements.engine(mode: .team)
                == "identifier \"io.github.ikari-zen.inputmethod.ikari.engine\" and anchor apple generic and certificate leaf[subject.OU] = \"L9Z8SK775X\""
        )
        #expect(
            SigningRequirements.shell(mode: .team)
                == "(identifier \"io.github.ikari-zen.inputmethod.ikari\" or identifier \"io.github.ikari-zen.inputmethod.ikari.smoke\") and anchor apple generic and certificate leaf[subject.OU] = \"L9Z8SK775X\""
        )
    }

    @Test func adhocRequirementsOnlyCheckTheIdentifier() {
        #expect(SigningRequirements.engine(mode: .adhoc) == "identifier \"io.github.ikari-zen.inputmethod.ikari.engine\"")
        #expect(
            SigningRequirements.shell(mode: .adhoc)
                == "identifier \"io.github.ikari-zen.inputmethod.ikari\" or identifier \"io.github.ikari-zen.inputmethod.ikari.smoke\""
        )
    }

    @Test func settingsAppRequirementPinsTheIdentifierAndTheTeam() {
        #expect(
            SigningRequirements.settingsApp(mode: .team)
                == "identifier \"io.github.ikari-zen.inputmethod.ikari.settings\" and anchor apple generic and certificate leaf[subject.OU] = \"L9Z8SK775X\""
        )
    }

    @Test func bridgeShellRequirementPinsTheShellIdentifierAndTheTeam() {
        #expect(
            SigningRequirements.bridgeShell(mode: .team)
                == "identifier \"io.github.ikari-zen.inputmethod.ikari\" and anchor apple generic and certificate leaf[subject.OU] = \"L9Z8SK775X\""
        )
    }

    @Test func adhocBridgeRequirementsPinTheCodeHash() {
        let hash = Data([0x00, 0xab, 0xff])
        #expect(
            SigningRequirements.settingsApp(mode: .adhoc, cdhash: hash)
                == "identifier \"io.github.ikari-zen.inputmethod.ikari.settings\" and cdhash H\"00abff\"")
        #expect(
            SigningRequirements.bridgeShell(mode: .adhoc, cdhash: hash)
                == "identifier \"io.github.ikari-zen.inputmethod.ikari\" and cdhash H\"00abff\"")
    }

    @Test func adhocBridgeRequirementsNeedACodeHash() {
        #expect(SigningRequirements.settingsApp(mode: .adhoc) == nil)
        #expect(SigningRequirements.bridgeShell(mode: .adhoc) == nil)
        #expect(SigningRequirements.settingsApp(mode: .adhoc, cdhash: Data()) == nil)
    }

    @Test func teamBridgeRequirementsIgnoreTheCodeHash() {
        #expect(
            SigningRequirements.settingsApp(mode: .team, cdhash: Data([1]))
                == SigningRequirements.settingsApp(mode: .team))
    }
}
