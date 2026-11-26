.PHONY: all dist check test test-gpu guest-test clean

all: dist

dist:
	cargo xtask dist

check:
	cargo xtask check

test:
	cargo test --workspace

test-gpu:
	cargo test --workspace -- --ignored --nocapture

guest-test:
	$(MAKE) -C guest/bundle host-test

clean:
	cargo clean
	$(MAKE) -C guest/bundle clean
