Feature: Each turn receives bounded awareness from the shared fabric
  @croissant @bdd-22 @bdd-23
  Scenario: Relay membership warms a peer profile without exposing backend authority
    Given an isolated configured Mosaico home using a fresh NIP-29 relay
    And the backend starts an agent in workspace "warm-demo"
    When a relay-only peer named "alice-peer" joins workspace "warm-demo"
    Then the roster resolves that peer as "alice-peer" without an explicit lookup
    And the backend management identity is absent from the member roster

  @croissant @designed @issue-704
  Scenario: A first turn receives channel context and recent ambient history
    Given an agent joins a channel with recent conversation
    When its first native turn begins
    Then its context names the workspace, channel, members, and recent activity
    And pre-join history is summarized without being presented as new speech

  @croissant @designed @issue-704
  Scenario: Later turns receive only unseen ambient changes
    Given an agent has completed its first turn
    When new channel activity arrives and another turn begins
    Then the new activity appears once
    And previously seen activity is not repeated

  @wip @issue-641
  Scenario: Incomplete stored ancestry cannot erase otherwise valid awareness
    Given a stored channel is missing one workspace ancestor
    When a hook asks for current awareness
    Then Mosaico reports the incomplete ancestry diagnostically
    And still returns every safely resolvable awareness fact
