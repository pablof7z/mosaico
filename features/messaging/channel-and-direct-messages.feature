Feature: Addressed work preserves delivery and sender authority
  @croissant
  Scenario: An operator addresses one live PTY session exactly once
    Given an isolated configured Mosaico home using a fresh NIP-29 relay
    And Claude agent "reviewer" is live in workspace "direct-demo"
    When the operator addresses that agent with "review this once"
    Then the native harness receives "review this once" exactly once

  @croissant
  Scenario: An explicit sender session overrides ambient process hints
    Given an isolated configured Mosaico home using a fresh NIP-29 relay
    And Claude agents "reviewer" and "writer" are live in workspace "anchor-demo"
    When I send "explicit anchor wins" with the second session explicitly selected
    Then the relay message "explicit anchor wins" is authored by the explicitly selected session
  @croissant
  Scenario: An agent searches locally cached messages across its backend
    Given an isolated configured Mosaico home using a fresh NIP-29 relay
    And Claude agent "reviewer" is live in workspace "search-demo"
    When the operator addresses that agent with "land the searchable commit"
    And I search all cached channels for "searchable commit"
    Then the search output groups "land the searchable commit" under channel "/search-demo"
