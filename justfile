build:
    cargo build --release

install: build
    rm -f ~/.local/bin/tenex-edge
    cp target/release/tenex-edge ~/.local/bin/tenex-edge
    xattr -cr ~/.local/bin/tenex-edge
    codesign --force --sign - ~/.local/bin/tenex-edge

lint:
    cargo clippy --all-targets -- -D warnings

test:
    cargo test

# Hermetic unit tests only. This is what CI runs. `just test` runs local
# relay-backed integration tests too; ignored public-relay probes require their
# explicit `cargo test --test ... -- --ignored --nocapture` commands.
test-unit:
    cargo test --lib

fmt-check:
    cargo fmt --check

helper-import-check:
    bash scripts/check_integration_helpers.sh

loc-check:
    bash scripts/check_loc.sh
    bash scripts/check_integration_helpers.sh
