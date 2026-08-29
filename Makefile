.PHONY: validate deps fmt clippy test test-release doc verify qualify contract doctor demo bundle

validate:
	python3 scripts/validate_repo.py

deps:
	python3 scripts/check_dependency_policy.py

fmt:
	cargo fmt --all -- --check

clippy:
	cargo clippy --locked --workspace --all-targets --all-features -- -D warnings

test:
	cargo test --locked --workspace --all-targets --all-features

test-release:
	cargo test --locked --release --workspace --all-targets --all-features

doc:
	RUSTDOCFLAGS="-D warnings" cargo doc --locked --workspace --all-features --no-deps

verify:
	./scripts/verify.sh

qualify:
	./scripts/qualify_local.sh

contract:
	cargo run --locked --quiet --bin dwarf-fortress-mcp -- contract

doctor:
	cargo run --locked --quiet --bin dwarf-fortress-mcp -- doctor

demo:
	cargo run --locked --quiet --bin dwarf-fortress-mcp -- demo

bundle:
	./scripts/create_source_bundle.sh
