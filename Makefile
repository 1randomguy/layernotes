.PHONY: help build start install fmt check test

PREFIX ?= /usr
BINDIR ?= $(PREFIX)/bin

help:
	@echo "Usage: make [target]"
	@echo ""
	@echo "Targets:"
	@echo "  help          Show this help message"
	@echo "  build         Build the project"
	@echo "  run           Run the build"
	@echo "  install       Install the build (supports DESTDIR and PREFIX)"
	@echo "  fmt           Format the code"
	@echo "  check         Format, check and lint the code"
	@echo "  test          Run the tests"

build:
	cargo build --release

run: build
	./target/release/layernotes

install: build
	install -Dm755 target/release/layernotes $(DESTDIR)$(BINDIR)/layernotes

fmt:
	cargo fmt

check: fmt
	cargo check
	cargo clippy -- -D warnings

test:
	cargo test
