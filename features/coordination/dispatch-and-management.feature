Feature: Backend management requests produce public fabric results
  @croissant
  Scenario: A backend-addressed management command receives one public result
    Given an isolated configured Mosaico home using a fresh NIP-29 relay
    And Claude agent "reviewer" is live in workspace "management-demo"
    When the operator sends management command "list sessions"
    Then the relay records a management reply containing "mgmt ok:"
