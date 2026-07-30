@must-never
Feature: Lifecycle hooks never acquire backend authority
  Scenario: A lifecycle hook never starts backend infrastructure
    Given an isolated configured Mosaico home using a local relay
    And no Mosaico daemon is running
    When a native session-start hook runs
    Then the hook returns successfully within its fail-open deadline
    And no daemon was spawned
