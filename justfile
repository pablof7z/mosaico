build:
    cargo build --release

install:
    cargo install --path . --bin mosaico --root ~/.local --locked --force
    xattr -cr ~/.local/bin/mosaico
    codesign --force --sign - ~/.local/bin/mosaico

lint:
    cargo clippy --all-targets -- -D warnings

# Hermetic CPU-attribution harness. It uses a disposable redb store and only
# `.invalid` relay names; no live daemon, Mosaico home, or public relay.
stress-nmp *ARGS:
    env -u MOSAICO cargo run --release --features stress-harness --bin nmp-subscription-stress -- {{ARGS}}

stress-nmp-check:
    env -u MOSAICO cargo test --features stress-harness --bin nmp-subscription-stress
    env -u MOSAICO cargo clippy --features stress-harness --bin nmp-subscription-stress -- -D warnings
    env -u MOSAICO cargo run --features stress-harness --bin nmp-subscription-stress -- --scenario router --topology sharded --retained 4 --mailboxes 3 --profile-burst 1 --corpus-rows 4 --iterations 1 --format csv

# Install the repo's git hooks (currently: a pre-commit `cargo fmt --check`,
# matching CI's fmt-check). Symlinked so `git pull` picks up hook updates.
install-hooks:
    ln -sf ../../scripts/git-hooks/pre-commit .git/hooks/pre-commit
    @echo "installed .git/hooks/pre-commit -> scripts/git-hooks/pre-commit"

test: test-all-local

test-all-local: test-dev-scripts test-site test-unit test-hermetic-integration test-local-relay test-local-nip29 test-behavior-contracts

# Test harnesses own exact temporary homes. Do not let the developer's selected
# live instance conflict with those low-level isolation overrides.

test-dev-scripts:
    env -u MOSAICO bash skills/mosaico-dev/tests/scripts.sh
    env -u MOSAICO bash scripts/tests/install-fleet.sh

test-site:
    node site/build.mjs
    node site/test.mjs

# Hermetic unit tests only. This is what CI runs.
test-unit:
    env -u MOSAICO cargo test --lib

# Hermetic real-binary contracts that need no relay or external executable.
test-hermetic-integration:
    env -u MOSAICO cargo test --test help
    env -u MOSAICO cargo test --test install_standalone
    env -u MOSAICO cargo test --test state_reset

# Local plain-Nostr relay tests. Requires `nak` on PATH or at `$HOME/go/bin/nak`.
test-local-relay:
    env -u MOSAICO cargo test --test daemon_mechanics
    env -u MOSAICO cargo test --test e2e_transport

# Local NIP-29 relay tests. Requires an external Croissant executable at
# `$NIP29_RELAY_BIN` or on PATH.
test-local-nip29:
    env -u MOSAICO cargo test --test daemon_integration -- --test-threads=1

# Narrow deterministic product contracts expressed through Cucumber. Croissant
# is an external fixture supplied by exact path; the runner always uses Cargo's
# exact Mosaico binary.
test-behavior-contracts:
    : "${NIP29_RELAY_BIN:?set NIP29_RELAY_BIN to a Croissant executable}"
    env -u MOSAICO cargo test --test bdd

test-live-relay-probe:
    : "${MOSAICO_RELAY:?set MOSAICO_RELAY=wss://relay.tenex.chat}"
    env -u MOSAICO cargo test --test relay_probe -- --ignored --nocapture

test-live-nip29-probe:
    : "${MOSAICO_NIP29_RELAY:?set MOSAICO_NIP29_RELAY=wss://nip29.f7z.io}"
    env -u MOSAICO cargo test --test nip29_probe -- --ignored --nocapture

test-live-seed-validation:
    : "${MOSAICO_NIP29_RELAY:?set MOSAICO_NIP29_RELAY=wss://nip29.f7z.io}"
    env -u MOSAICO cargo test --test seed_validation -- --ignored --nocapture

fmt-check:
    cargo fmt --check

helper-import-check:
    bash scripts/check_integration_helpers.sh

loc-check:
    bash scripts/check_loc.sh
    bash scripts/check_integration_helpers.sh
    bash scripts/check_hosted_open_seam.sh
