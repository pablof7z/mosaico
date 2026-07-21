# Adding a Harness

Use this checklist when adding first-class support for an agent harness. Add
only capabilities the harness actually exposes. An unsupported transport,
profile mechanism, hook, or plugin must remain unsupported rather than being
approximated with aliases, wrappers, or compatibility shims.

The ownership boundary is:

- agent JSON selects a role, harness bundle, and optional profile;
- `harnesses.json` owns configurable launch policy; and
- Rust owns the supported `(harness, transport)` capability matrix and maps
  profiles onto each harness's native surface.

## Establish the native contract

- [ ] Choose one canonical harness ID and default agent slug.
- [ ] Confirm the installed executable and current supported version.
- [ ] Identify every native transport Mosaico will support: PTY, ACP, or
  app-server.
- [ ] Record the exact fresh-launch command for each supported transport.
- [ ] Record the harness's authoritative session ID and exact resume mechanism.
- [ ] Determine how turn completion and mid-turn steering work.
- [ ] Identify authentication, configuration, profile, and session-state
  locations.
- [ ] Mark unsupported capabilities explicitly. Do not add alternate names,
  fallback commands, compatibility aliases, or guessed behavior.

## Register the harness

- [ ] Add the harness enum variant, canonical serialization, parser, and default
  agent slug in `src/session.rs`.
- [ ] Add one row per supported `(harness, transport)` pair in
  `src/harness/driver.rs`.
- [ ] For each driver row, define the executable and arguments, required
  environment changes, resume mechanism, steering primitive, turn model, and
  profile mechanism.
- [ ] Add executable detection and inventory presentation.
- [ ] Add interactive and managed transport preferences in
  `src/session_host/launch/source.rs`.
- [ ] Ensure hosted sessions persist the observed harness, admitted bundle,
  admitted transport, native session locator, and runtime endpoint.
- [ ] Add the runtime endpoint and native resume locators needed to reconnect
  after process or daemon restart.

## Integrate hooks and plugins

- [ ] Determine whether the harness natively supports hooks or plugins.
- [ ] Distinguish provider-native plugins or extensions from the integration
  surface Mosaico itself requires.
- [ ] If supported, add and test only the Mosaico integration the harness
  requires.
- [ ] If unsupported, document that hooks and plugins must not be added. Do not
  emulate them with wrappers, injected polling, or compatibility shims.

## Discover native profiles

- [ ] Determine whether the harness supports named profiles, agents, or an
  equivalent native configuration.
- [ ] Classify profile support per transport. Document profiles exposed only by
  unsupported transports, but do not advertise them as launchable.
- [ ] If supported, add discovery for every authoritative global and
  workspace-local profile location and format.
- [ ] Register discovered profiles in Mosaico's agent catalog so they are picked
  up automatically without duplicate Mosaico agent JSON files.
- [ ] Preserve workspace precedence when a local profile overrides a global
  profile from the same harness.
- [ ] Implement native profile activation for every transport that supports it.
- [ ] Do not advertise a profile through a transport that cannot activate it.
- [ ] Keep an unbound role ambiguous when multiple harnesses provide it; never
  silently choose a harness.
- [ ] Test automatic discovery, inventory presentation, selection, launch, and
  resume with a native profile.

## Package the runtime

- [ ] Install a pinned harness version in the development or hosted image.
- [ ] Give the harness an isolated, writable provider home.
- [ ] Stage host authentication without printing secrets or importing unrelated
  host session state.
- [ ] Add runner commands, development-profile mappings, and environment
  overrides needed by the harness.
- [ ] Add doctor checks for the exact executable, transport, configuration, and
  authentication requirements.

## Prove the integration

- [ ] Test canonical parsing and reject removed, legacy, or alternate names.
- [ ] Test every supported and unsupported driver-matrix cell.
- [ ] Test executable detection, inventory, and transport selection.
- [ ] Test profile activation and unsupported-profile failures.
- [ ] Test fresh launch, a completed prompt, process restart, exact resume, and
  another completed prompt.
- [ ] Test hosted launch and message delivery through the daemon.
- [ ] Assert persisted admission facts, runtime endpoints, and native resume
  locators.
- [ ] Run formatting, clippy, line-count checks, unit tests, daemon integration
  tests, and CI.
- [ ] Perform one live end-to-end run using the real harness and real provider
  authentication.

## Document the result

- [ ] Update the README support matrix.
- [ ] Document supported transports, profile behavior, resume behavior,
  authentication boundaries, and known limitations.
- [ ] Include live proof and the commands used in the implementation pull
  request.
- [ ] Use only the canonical harness name everywhere and remove stale names in
  the same change.
