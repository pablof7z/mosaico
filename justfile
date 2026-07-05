build:
    cargo build --release

install: build
    rm -f ~/.local/bin/tenex-edge
    cp target/release/tenex-edge ~/.local/bin/tenex-edge
    xattr -cr ~/.local/bin/tenex-edge
    codesign --force --sign - ~/.local/bin/tenex-edge

lint:
    cargo clippy --all-targets -- -D warnings

# Install the repo's git hooks (currently: a pre-commit `cargo fmt --check`,
# matching CI's fmt-check). Symlinked so `git pull` picks up hook updates.
install-hooks:
    ln -sf ../../scripts/git-hooks/pre-commit .git/hooks/pre-commit
    @echo "installed .git/hooks/pre-commit -> scripts/git-hooks/pre-commit"

test: test-all-local

test-all-local: test-unit test-local-relay test-local-nip29

# Hermetic unit tests only. This is what CI runs.
test-unit:
    cargo test --lib

# Local plain-Nostr relay tests. Requires `nak` on PATH or at `$HOME/go/bin/nak`.
test-local-relay:
    cargo test --test daemon_mechanics
    cargo test --test e2e_transport

# Local NIP-29 relay tests. Requires croissant at `$NIP29_RELAY_BIN` or
# `$HOME/Work/croissant/croissant`.
test-local-nip29:
    cargo test --test daemon_integration -- --test-threads=1

test-live-relay-probe:
    : "${TE_RELAY:?set TE_RELAY=wss://relay.tenex.chat}"
    cargo test --test relay_probe -- --ignored --nocapture

test-live-nip29-probe:
    : "${TE_NIP29_RELAY:?set TE_NIP29_RELAY=wss://nip29.f7z.io}"
    cargo test --test nip29_probe -- --ignored --nocapture

test-live-seed-validation:
    : "${TE_NIP29_RELAY:?set TE_NIP29_RELAY=wss://nip29.f7z.io}"
    cargo test --test seed_validation -- --ignored --nocapture

fmt-check:
    cargo fmt --check

helper-import-check:
    bash scripts/check_integration_helpers.sh

loc-check:
    bash scripts/check_loc.sh
    bash scripts/check_integration_helpers.sh
