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
`mold`; macOS and Windows keep their platform linker. Cranelift is the
development-profile codegen backend: the whole suite passes under it on the
pinned `nightly-2026-03-05`. Coverage holds the development profile on LLVM,
because `-Cinstrument-coverage` is LLVM-only and the test profile inherits the
development profile's backend.

Cargo applies a single `rustflags` source rather than merging them, and an
assigned `RUSTFLAGS` replaces every source. So each source repeats the frontend
flag, and the Makefile restates both flags as `STANDARD_RUSTFLAGS` for the
targets that assign `RUSTFLAGS`. Release builds assign an empty inherited
`RUSTFLAGS` and so take neither flag. CI installs `mold` before the first gate
target. `tests/build_standard_contract.rs` holds the configuration and the
Makefile recipes to this.
