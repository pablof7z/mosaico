Feature: Hosted harness transports preserve one admitted runtime contract
  @croissant @bdd-03 @bdd-21
  Scenario: A named Claude profile activates through the exact native selector
    Given an isolated configured Mosaico home using a fresh NIP-29 relay
    And Claude profile "reviewer" is installed
    And agent "reviewer" selects that profile through bundle "yolo-claude"
    When Mosaico launches agent "reviewer"
    Then the command succeeds
    And the Claude process receives exactly the bundle arguments and profile selector
    And no selector belonging to another harness is present
    And no legacy terminal multiplexer was invoked

  @designed @issue-704
  Scenario Outline: A named profile activates through its harness-native selector
    Given installed <harness> profile "<profile>"
    And agent "<agent>" selects that profile over <transport>
    When Mosaico launches "<agent>"
    Then the admitted runtime uses <harness> over <transport>
    And the native process receives profile "<profile>"
    And no selector belonging to another harness is present

    Examples:
      | harness | profile  | agent    | transport  |
      | Claude  | reviewer | reviewer | PTY        |
      | Codex   | reviewer | reviewer | app-server |
      | Hermes  | reviewer | reviewer | ACP        |

  @live @designed @issue-704
  Scenario Outline: A real provider completes, resumes, and completes another turn
    Given valid host authentication for <harness>
    When Mosaico launches a minimal turn over <transport>
    And the native process restarts and resumes the same session
    Then both turns complete under the same public session identity

    Examples:
      | harness | transport  |
      | Claude  | ACP        |
      | Codex   | app-server |
      | Grok    | PTY        |
      | Goose   | ACP        |
      | Hermes  | ACP        |
      | OpenCode| ACP        |

  @must-never @wip @issue-496
  Scenario: A foreign hook claim cannot reclassify an admitted runtime
    Given Mosaico launched a Grok PTY with an authoritative endpoint
    When that process emits a hook claiming to be Claude
    Then the runtime remains admitted as Grok over PTY
    And an addressed message still reaches the owned Grok endpoint
