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

# Hermetic unit tests only (no external relays). This is what CI runs: the
# integration tests under tests/ require a live `nak` relay and a local NIP-29
# relay (croissant) binary that aren't provisioned on CI runners.
test-unit:
    cargo test --lib

fmt-check:
    cargo fmt --check

loc-check:
    bash scripts/check_loc.sh