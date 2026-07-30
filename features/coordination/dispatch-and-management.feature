Feature: Coordination requests create visible responsibility without claiming completion
  @croissant @bdd-20
  Scenario: A backend-addressed management command receives one public result
    Given an isolated configured Mosaico home using a fresh NIP-29 relay
    And Claude agent "reviewer" is live in workspace "management-demo"
    When the operator sends management command "list sessions"
    Then the relay records a management reply containing "mgmt ok:"

  @croissant @designed @issue-704
  Scenario: Dispatch starts an available agent in the requested channels
    Given an available agent on a trusted backend
    When a session dispatches work to that agent in two channels
    Then one new public session identity joins both channels
    And the prompt is delivered as addressed work
    And dispatch makes no claim that the work is complete

  @croissant @designed @issue-704
  Scenario: A structured management add launches only the addressed backend capability
    Given two backends advertise different agents
    When the operator adds one backend-qualified agent through a management message
    Then only the addressed backend launches it
    And the management reply identifies the new session

  @designed @issue-383
  Scenario: A work thread closes only through explicit agent-authored settlement
    Given an agent accepted addressed work
    When its native turn ends successfully
    Then Mosaico records the native outcome without claiming semantic completion
    And the thread remains open until an authorized participant explicitly closes it
