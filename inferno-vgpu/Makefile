.PHONY: all dist check test test-gpu guest-test firmware sdk-check clean

all: dist firmware

dist:
	cargo xtask dist

check:
	cargo xtask check

test:
	cargo test --workspace

test-gpu:
	cargo test --workspace -- --ignored --nocapture

guest-test:
	$(MAKE) -C guest all

firmware:
	$(MAKE) -C firmware all

sdk-check:
	$(MAKE) -C guest sdk-check

clean:
	cargo clean
	$(MAKE) -C guest clean
	$(MAKE) -C firmware clean
