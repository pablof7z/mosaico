Feature: Addressed activation preserves one durable public identity
  @croissant
  Scenario: Addressed work recovers one stopped session under the same identity
    Given an isolated configured Mosaico home using a fresh NIP-29 relay
    And Claude agent "reviewer" is live in workspace "resume-demo"
    When the operator stops that exact session
    And the operator addresses that agent with "resume this identity"
    Then agent "reviewer" is live under the same public identity with no sibling
    And the native harness observes one user-visible delivery of "resume this identity" during the controlled execution

  @croissant
  Scenario: Addressed work starts an offline stable agent under its configured identity
    Given an isolated configured Mosaico home using a fresh NIP-29 relay
    And stable Claude agent "durable-reviewer" is configured but offline in workspace "stable-demo"
    When the operator addresses that configured identity with "start the stable identity"
    Then agent "durable-reviewer" is live under the same public identity with no sibling
    And the native harness observes one user-visible delivery of "start the stable identity" during the controlled execution
