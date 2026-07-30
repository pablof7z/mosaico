Feature: Workspace scope comes from registered roots and explicit context
  @designed @issue-704
  Scenario: A nested working directory resolves to its registered workspace
    Given workspace "/work/mosaico" is registered as "mosaico"
    And an agent starts under "/work/mosaico/src/fabric"
    When the agent asks for its session context
    Then its workspace is "mosaico"
    And its relative working directory is "src/fabric"

  @wip @issue-672
  Scenario: A cross-workspace channel stays anchored to its absolute root
    Given a session belongs to channels in two workspaces
    When it addresses the absolute channel "/other/reviews"
    Then Mosaico selects exactly "/other/reviews"
    And no same-named local child channel is created
