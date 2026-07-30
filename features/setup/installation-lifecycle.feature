Feature: Installation begins with one explicit setup surface
  Scenario: An unconfigured home points to setup without starting background state
    Given an isolated unconfigured Mosaico home
    And no Mosaico daemon is running
    When I run Mosaico with no arguments
    Then the command succeeds
    And the output contains "mosaico setup"
    And no daemon was spawned

  @designed @issue-704
  Scenario: Reconfiguration preserves foreign hooks, unknown fields, and identities
    Given Mosaico is installed beside foreign harness hooks
    And its device configuration contains an unknown field and established identities
    When the operator reconfigures the selected harness integrations
    Then only Mosaico-owned integration entries change
    And the unknown field and established identities remain byte-for-byte unchanged

  @designed @issue-704
  Scenario: Scoped uninstall leaves the shared installation intact
    Given Codex and Goose integrations are selected
    When the operator uninstalls only Goose
    Then the Codex integration and packaged runtime skill remain installed
    And Mosaico state and external relay infrastructure remain untouched
