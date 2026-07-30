@croissant
Feature: Independent backends coordinate only through the relay
  Scenario: A workspace opened on one backend appears on another
    Given a fresh NIP-29 relay
    And backends "laptop" and "server" have isolated homes
    And both backends trust the same operator
    When "laptop" starts an agent in workspace "mosaico"
    Then the relay holds the root channel for "mosaico"
    When "server" lists every visible workspace
    Then "server" shows workspace "mosaico"
    And no filesystem state is shared between the backends
