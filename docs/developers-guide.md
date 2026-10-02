# Developers' guide

## Spelling policy

Run the spelling gate with:

```bash
make spelling
```

The tracked `typos.toml` is regenerated on every run from the live shared
dictionary and the repository-specific `typos.local.toml` overlay. Never edit
generated entries by hand; add only narrow repository terminology to the
overlay. Because the dictionary is live, `typos.toml` must never be drift
checked in continuous integration.

The focused shared config builder refreshes the dictionary into an untracked
local cache only when the authoritative copy is newer. A valid cache remains
usable when the network is unavailable. Quoted APIs and identifiers retain
their upstream spelling; put them in backticks or fenced code blocks where
practical rather than adding broad word-level exceptions.

## Build standard

Development builds follow the estate's Rust build standard, which
`.cargo/config.toml` sets and Cargo auto-discovers, so a bare `cargo build`
gets it. Every `rustflags` source enables the parallel `rustc` frontend with
`-Zthreads=8`, and the `cfg(target_os = "linux")` source also links with
`mold`; macOS and Windows keep their platform linker. A debug build for a
WebAssembly target (`CARGO_BUILD_TARGET` of `wasm32` or `wasm64`) takes LLVM,
because Cranelift has no WebAssembly target. A Linux host therefore needs
`mold` installed before any `cargo` or `make` build, build scripts included.
Cranelift is the development-profile codegen backend: the whole suite passes
under it on the pinned `nightly-2026-03-05`. Coverage holds the development
profile on LLVM, because `-Cinstrument-coverage` is LLVM-only and the test
profile inherits the development profile's backend.

Cargo applies a single `rustflags` source rather than merging them, and an
assigned `RUSTFLAGS` replaces every source. So each source repeats the frontend
flag, and the Makefile restates both flags as `STANDARD_RUSTFLAGS` for the
targets that assign `RUSTFLAGS`, adding them to any `RUSTFLAGS` the recipe
inherits (setup-rust exports one in CI) rather than replacing it; every `lint`
command assigns it too. The Makefile adds `mold` only when both the host and
the compilation target (`CARGO_BUILD_TARGET`, when set) are Linux; an Android
triple contains `-linux-` but is not Linux to Cargo's `target_os`, so it does
not get `mold`. `make release` assigns the inherited `RUSTFLAGS`, which is
empty when the caller exports none, so it takes neither flag. A bare
`cargo build --release` still takes both flags, because Cargo does not select
`rustflags` by profile. The target is read from `CARGO_BUILD_TARGET` alone,
which is the supported way to cross-compile here: a `--target` passed through
`TEST_FLAGS` or `CARGO_FLAGS` is invisible to Make, so set the variable as
well. CI installs `mold` before the first gate target.

`make lint` runs Whitaker on LLVM, because its Dylint driver builds outside
this workspace on a toolchain that need not carry the Cranelift component. The
recipe sets `CARGO_UNSTABLE_CODEGEN_BACKEND=true` (the driver crate is outside
the `[unstable]` table) and `CARGO_PROFILE_DEV_CODEGEN_BACKEND` from
`WHITAKER_CODEGEN_BACKEND`, which defaults to `llvm`. A failing Whitaker run
fails the target; a missing `whitaker` binary skips the check with a message.
The coverage step in `ci.yml` sets the same two variables for the same reason.

`tests/build_standard_contract.rs` (readers in
`tests/build_standard/support.rs`) holds the configuration and the Makefile
recipes to the flag rules. `tests/build_standard_backend.rs` holds the
Cranelift configuration, the coverage step's LLVM overrides and the Whitaker
boundary.
