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
