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

fmt-check:
    cargo fmt --check

loc-check:
    bash scripts/check_loc.sh