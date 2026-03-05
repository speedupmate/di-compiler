.PHONY: build test check clippy fmt bench clean

build:
	cargo build --workspace

build-release:
	cargo build --workspace --release

test:
	cargo test --workspace

test-ignored:
	cargo test --workspace -- --ignored

check:
	cargo check --workspace

clippy:
	cargo clippy --workspace -- -D warnings

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all -- --check

bench:
	cargo bench --workspace

clean:
	cargo clean

ci: fmt-check clippy test
