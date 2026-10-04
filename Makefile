.PHONY: help all clean test build release lint fmt check-fmt markdownlint nixie spelling

TARGET ?= libytmusic_wasm.rlib

CARGO ?= cargo
BUILD_JOBS ?=
RUST_FLAGS ?= -D warnings
# The build standard: every `rustflags` source in `.cargo/config.toml` carries
# the parallel frontend, and the Linux source adds `mold`. Assigning `RUSTFLAGS`
# replaces those sources outright, so the targets that assign it restate the
# flags here. The recipes add them to any inherited `RUSTFLAGS` (setup-rust
# exports one in CI) instead of replacing it. `make release` adds neither standard flag;
# coverage takes neither only when its caller exports `RUSTFLAGS`, as setup-rust
# does in CI.
STANDARD_THREADS_FLAG ?= -Zthreads=8
STANDARD_MOLD_FLAG ?= -Clink-arg=-fuse-ld=mold
BUILD_HOST_OS ?= $(shell uname -s)
# mold is added only when the machine doing the build is Linux (only Make can
# tell whether it has mold) and the compilation target is Linux too, which is
# the host unless `CARGO_BUILD_TARGET` names another triple. Android triples
# contain `-linux-` but report `target_os = "android"`, so they are not Linux.
STANDARD_TARGET_IS_LINUX = $(if $(CARGO_BUILD_TARGET),$(or $(filter host-tuple,$(CARGO_BUILD_TARGET)),$(and $(findstring -linux-,$(CARGO_BUILD_TARGET)),$(if $(findstring -android,$(CARGO_BUILD_TARGET)),,yes))),yes)
STANDARD_RUSTFLAGS = $(STANDARD_THREADS_FLAG)$(if $(filter Linux,$(BUILD_HOST_OS)),$(if $(STANDARD_TARGET_IS_LINUX), $(STANDARD_MOLD_FLAG)))
# Release builds add neither standard flag: assigning `RUSTFLAGS`, even to an
# empty inherited value, displaces every `rustflags` source in the
# configuration, and a caller's own value passes through untouched.
RELEASE_RUSTFLAGS = RUSTFLAGS="$${RUSTFLAGS-}"
# Debug builds keep a caller's exported flags and add the standard ones,
# since an inherited `RUSTFLAGS` would otherwise displace the configuration.
# Cranelift has no WebAssembly target, so a debug build for one takes LLVM.
WASM_CODEGEN_BACKEND = $(if $(filter wasm32% wasm64%,$(CARGO_BUILD_TARGET)),CARGO_PROFILE_DEV_CODEGEN_BACKEND=llvm CARGO_UNSTABLE_CODEGEN_BACKEND=true)
DEBUG_RUSTFLAGS = RUSTFLAGS="$${RUSTFLAGS:+$$RUSTFLAGS }$(STANDARD_RUSTFLAGS)"
# Gate targets also deny warnings, so they compose the caller's flags, the
# warning policy and the standard flags in one place.
GATE_RUSTFLAGS = RUSTFLAGS="$${RUSTFLAGS:+$$RUSTFLAGS }$(RUST_FLAGS) $(STANDARD_RUSTFLAGS)"
# Whitaker's Dylint driver runs on its own pinned toolchain, which need not
# carry the Cranelift component the development profile selects, so its
# check builds take LLVM. Dylint builds its driver in a crate outside this
# repository, which the `[unstable]` table does not reach, so the override
# also enables the unstable key there.
WHITAKER_CODEGEN_BACKEND ?= llvm
CARGO_FLAGS ?= --all-targets --all-features
CLIPPY_FLAGS ?= $(CARGO_FLAGS) -- $(RUST_FLAGS)
TEST_FLAGS ?= $(CARGO_FLAGS)
MDLINT ?= markdownlint-cli2
# `make fmt` and `make check-fmt` call mdtablefix directly. `--git` selects the
# Markdown files Git tracks and `--include-untracked` adds the untracked files
# Git does not ignore, so a new document is formatted before it is staged.
# Both modes need mdtablefix 0.6.0 or later; CI pins the version at the
# install-mdtablefix step.
MDTABLEFIX ?= mdtablefix
MDTABLEFIX_SELECT = --git --include-untracked
MDTABLEFIX_RULES = --wrap --renumber --breaks --ellipsis --fences
NIXIE ?= nixie
WHITAKER ?= whitaker
UV ?= uv
UV_ENV = UV_CACHE_DIR=.uv-cache UV_TOOL_DIR=.uv-tools
TYPOS_CONFIG_BUILDER_VERSION ?= v0.1.1
TYPOS_CONFIG_BUILDER = $(UV_ENV) $(UV) tool run --python 3.14 --from \
	"git+https://github.com/leynos/typos-config-builder.git@$(TYPOS_CONFIG_BUILDER_VERSION)" \
	typos-config-builder

build: target/debug/$(TARGET) ## Build debug binary
release: target/release/$(TARGET) ## Build release binary

all: check-fmt lint test spelling ## Perform a comprehensive check of code

clean: ## Remove build artefacts
	$(CARGO) clean

test: ## Run tests with warnings treated as errors
	$(GATE_RUSTFLAGS) $(CARGO) test $(TEST_FLAGS) $(BUILD_JOBS)

target/%/$(TARGET): ## Build binary in debug or release mode
	$(if $(findstring release,$(@)),$(RELEASE_RUSTFLAGS),$(DEBUG_RUSTFLAGS) $(WASM_CODEGEN_BACKEND)) $(CARGO) build $(BUILD_JOBS) $(if $(findstring release,$(@)),--release)

lint: ## Run Clippy and the Whitaker Dylint suite with warnings denied
	$(GATE_RUSTFLAGS) RUSTDOCFLAGS="$(RUSTDOC_FLAGS)" $(CARGO) doc --no-deps
	$(GATE_RUSTFLAGS) $(CARGO) clippy $(CLIPPY_FLAGS)
	@# `if` rather than `&& ... ||`, so a failing Whitaker run fails the target
	@# instead of falling through to the not-installed message.
	@if command -v $(WHITAKER) >/dev/null 2>&1; then \
		CARGO_UNSTABLE_CODEGEN_BACKEND=true CARGO_PROFILE_DEV_CODEGEN_BACKEND=$(WHITAKER_CODEGEN_BACKEND) $(GATE_RUSTFLAGS) $(WHITAKER) --all -- $(CARGO_FLAGS); \
	else \
		echo "whitaker not found on PATH; skipping whitaker lint. Install whitaker to run this check."; \
	fi

fmt: ## Format Rust and Markdown sources
	$(CARGO) fmt --all
	$(MDTABLEFIX) --in-place $(MDTABLEFIX_SELECT) $(MDTABLEFIX_RULES)
	$(MDLINT) --fix "**/*.md"

check-fmt: ## Verify formatting
	$(CARGO) fmt --all -- --check
	$(MDTABLEFIX) --check $(MDTABLEFIX_SELECT) $(MDTABLEFIX_RULES)

markdownlint: spelling ## Lint Markdown files and enforce spelling
	$(MDLINT) '**/*.md'

spelling: ## Enforce en-GB-oxendict spelling
	$(TYPOS_CONFIG_BUILDER) gate --repository .

nixie: ## Validate Mermaid diagrams
	$(NIXIE) --no-sandbox

help: ## Show available targets
	@grep -E '^[a-zA-Z_-]+:.*?##' $(MAKEFILE_LIST) | \
	awk 'BEGIN {FS=":"; printf "Available targets:\n"} {printf "  %-20s %s\n", $$1, $$2}'
