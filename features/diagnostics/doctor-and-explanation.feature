Feature: Diagnostics state exactly what Mosaico can and cannot prove
  Scenario: An unconfigured home produces one unhealthy JSON report
    Given an isolated unconfigured Mosaico home
    When I request diagnostic JSON
    Then the command fails
    And the output is valid JSON
    And diagnostic "config.document" is "error"
    And no daemon was spawned

  @designed @issue-704
  Scenario: Event explanation correlates acceptance, routing, and native delivery
    Given an addressed event has entered the local backend
    When the operator explains that event
    Then the report distinguishes relay materialization from inbox claim
    And it distinguishes harness delivery from native turn outcome
    And every unknown fact remains explicitly unknown

  @wip @issue-701
  Scenario: A durable-store write failure becomes a current health error
    Given the daemon loses the ability to persist accepted work
    When a client asks for current health
    Then doctor reports the backend write-dead
    And no later request is acknowledged as durably accepted
