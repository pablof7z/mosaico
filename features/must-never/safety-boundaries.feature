@must-never
Feature: Integration failure never captures authority or silently loses work
  Scenario: A lifecycle hook never starts backend infrastructure
    Given an isolated configured Mosaico home using a local relay
    And no Mosaico daemon is running
    When a native session-start hook runs
    Then the hook returns successfully within its fail-open deadline
    And no daemon was spawned

  @designed @issue-704
  Scenario: Peer text is rendered as data and never promoted to host authority
    Given an untrusted peer message contains command-like text
    When that message enters an agent's awareness context
    Then it is attributed to the peer as fabric data
    And it cannot alter setup, authorization, or harness execution policy

  @wip @issue-291
  Scenario: Work sent immediately after adding an agent never disappears
    Given an operator added "chief-of-staff" to channel "/everything"
    And the new session is live and addressable
    When the operator sends that session an addressed work item
    Then the event is durably visible as accepted, pending, delivered, or failed
    And event explanation says why it did or did not reach the harness

  @designed @issue-704
  Scenario: Secret-bearing configuration never appears in public output
    Given configured operator and backend secret keys
    When every human and JSON diagnostic surface is rendered
    Then neither secret appears in stdout, stderr, logs, context, or receipts
