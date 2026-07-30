Feature: Agent discovery presents launchable capabilities without ambiguous shortcuts
  Scenario: A removed direct agent target is rejected
    Given an isolated unconfigured Mosaico home
    When I invoke the removed agents target
    Then the command fails
    And the output contains "unrecognized subcommand"

  @designed @issue-627
  Scenario: One logical profile installed in several harnesses renders once
    Given profile "writing-partner" is installed for Claude, Codex, and OpenCode
    When the operator lists available agents
    Then "writing-partner" appears as one logical agent
    And its three harness implementations are available as launch variants

  @designed @issue-704
  Scenario: An ambiguous unbound native profile never guesses a harness
    Given the same unbound profile exists in two harnesses
    When an agent tries to launch that profile non-interactively
    Then launch fails with the exact harness choices
    And no persistent binding is written
