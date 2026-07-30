Feature: Channel speech and addressed work have distinct delivery semantics
  @croissant @bdd-16
  Scenario: An operator addresses one live PTY session exactly once
    Given an isolated configured Mosaico home using a fresh NIP-29 relay
    And Claude agent "reviewer" is live in workspace "direct-demo"
    When the operator addresses that agent with "review this once"
    Then the native harness receives "review this once" exactly once

  @croissant @bdd-19
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

  @croissant @designed @issue-704
  Scenario: An addressed message reaches one live session exactly once
    Given two agents are live in the same channel
    When one sends an addressed message to the other
    Then the target harness receives the message exactly once
    And the other channel members see ordinary channel history without direct delivery

  @croissant @designed @issue-704
  Scenario: Reply preserves its event relationship and channel
    Given a visible message in "/mosaico/reviews"
    When an agent replies to that message
    Then the reply remains in "/mosaico/reviews"
    And its public event relationship names the original message

  @croissant @designed @issue-704
  Scenario: An offline resumable session receives the message under the same identity
    Given an addressed session is offline and resumable
    When a message targets its exact public key
    Then Mosaico resumes the same session identity
    And its native harness receives the message exactly once

  @designed @issue-704
  Scenario: An attachment is uploaded and referenced without leaking local paths
    Given a readable local attachment
    When an agent sends it with a channel message
    Then the channel event contains the uploaded public reference
    And no recipient receives the sender's local filesystem path
