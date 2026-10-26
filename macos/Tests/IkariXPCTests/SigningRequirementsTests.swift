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
        #expect(SigningRequirements.settingsApp(mode: .adhoc) == "identifier \"io.github.ikari-zen.inputmethod.ikari.settings\"")
    }
}
