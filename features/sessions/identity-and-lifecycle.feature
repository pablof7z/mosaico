Feature: A session public key is its sole durable identity
  @croissant @bdd-17
  Scenario: Addressed work recovers one stopped session under the same identity
    Given an isolated configured Mosaico home using a fresh NIP-29 relay
    And Claude agent "reviewer" is live in workspace "resume-demo"
    When the operator stops that exact session
    And the operator addresses that agent with "resume this identity"
    Then agent "reviewer" is live under the same public identity with no sibling
    And the native harness receives "resume this identity" exactly once

  @croissant @bdd-18
  Scenario: Addressed work starts an offline stable agent under its configured identity
    Given an isolated configured Mosaico home using a fresh NIP-29 relay
    And stable Claude agent "durable-reviewer" is configured but offline in workspace "stable-demo"
    When the operator addresses that configured identity with "start the stable identity"
    Then agent "durable-reviewer" is live under the same public identity with no sibling
    And the native harness receives "start the stable identity" exactly once

  @croissant @designed @issue-704
  Scenario: Resuming a native session preserves the same public identity
    Given a hosted session has a native resume locator
    And the session is offline
    When the operator resumes that native session
    Then the same public session identity becomes live
    And its signer, agent, workspace, and channel remain unchanged

  @croissant @designed @issue-704
  Scenario: A duplicate agent in one channel receives a distinct transient identity
    Given agent "reviewer" is already live in "/mosaico/reviews"
    When another "reviewer" session starts in the same channel
    Then the new session has a different public key and codename
    And messages can address each session without ambiguity

  @wip @issue-647
  Scenario: A durable agent is not evicted by ordinary headless-idle policy
    Given a durable-key agent is running headlessly
    When the ordinary idle deadline passes
    Then the durable agent remains available until explicitly stopped
