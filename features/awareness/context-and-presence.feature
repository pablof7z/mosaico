Feature: Relay-backed awareness preserves identity and authority boundaries
  @croissant
  Scenario: Relay membership warms a peer profile without exposing backend authority
    Given an isolated configured Mosaico home using a fresh NIP-29 relay
    And the backend starts an agent in workspace "warm-demo"
    When a relay-only peer named "alice-peer" joins workspace "warm-demo"
    Then the roster resolves that peer as "alice-peer" without an explicit lookup
    And the backend management identity is absent from the member roster
