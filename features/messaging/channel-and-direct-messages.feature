Feature: Addressed work preserves delivery and sender authority
  @croissant
  Scenario: An operator produces one harness-visible delivery in a controlled execution
    Given an isolated configured Mosaico home using a fresh NIP-29 relay
    And Claude agent "reviewer" is live in workspace "direct-demo"
    When the operator addresses that agent with "review this once"
    Then the native harness observes one user-visible delivery of "review this once" during the controlled execution

  @croissant
  Scenario: An explicit sender session overrides ambient process hints
    Given an isolated configured Mosaico home using a fresh NIP-29 relay
    And Claude agents "reviewer" and "writer" are live in workspace "anchor-demo"
    When I send "explicit anchor wins" with the second session explicitly selected
    Then the relay message "explicit anchor wins" is authored by the explicitly selected session
