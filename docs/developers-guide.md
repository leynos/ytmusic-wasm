# Developers' guide

## Spelling policy

The tracked `typos.toml` is generated from the shared estate dictionary and
the repository-specific `typos.local.toml` overlay. Never edit generated
entries by hand. Add only narrow repository terminology to the overlay, then
regenerate and verify the configuration with:

```bash
make spelling-config
```

The focused shared config builder refreshes the dictionary into an untracked
local cache only when the authoritative copy is newer. A valid cache remains
usable when the network is unavailable. Quoted APIs and identifiers retain
their upstream spelling; put them in backticks or fenced code blocks where
practical rather than adding broad word-level exceptions.
