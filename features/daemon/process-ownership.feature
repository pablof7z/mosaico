Feature: One daemon owns mutable backend state
  Scenario: A normal command starts the configured backend daemon
    Given an isolated configured Mosaico home using a local relay
    And no Mosaico daemon is running
    When I list every visible channel
    Then the command succeeds
    And one daemon owns the backend socket

  @designed @issue-704
  Scenario: Concurrent clients converge on one daemon and one writer
    Given sixteen clients share one configured backend home
    When all clients start write-bearing requests concurrently
    Then every client reaches the same daemon
    And the durable store passes its supported integrity diagnostic

  @designed @issue-704
  Scenario: Version skew replaces only the daemon
    Given a live hosted session and an older daemon protocol
    When a newer client connects
    Then the old daemon exits and the new daemon answers
    And the hosted session process remains alive and adopted
