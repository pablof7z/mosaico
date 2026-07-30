Feature: Channels are relay-backed paths with durable membership
  @croissant @designed @issue-704
  Scenario: Creating sibling channels preserves both public paths
    Given an agent is live in workspace "mosaico"
    When it creates channels "/mosaico/reviews" and "/mosaico/runtime"
    Then both channels appear under "/mosaico"
    And neither channel replaces the other

  @croissant @designed @issue-704
  Scenario: Stopping a session preserves standing until explicit leave
    Given a session is a member of "/mosaico/reviews"
    When its hosted runtime stops cleanly
    Then it is offline but remains a member of "/mosaico/reviews"
    When it explicitly leaves "/mosaico/reviews"
    Then its membership is absent

  @must-never @wip @issue-658
  Scenario: Path resolution never mints phantom same-name channels
    Given channels with the same child name exist under different roots
    When a session addresses one by absolute path
    Then Mosaico resolves only that path
    And no channel is auto-created under the current workspace
