# vitrify-core

The core library of [vitrify](https://crates.io/crates/vitrify-cli): the SDD instance model, trace hashing, the journal, the ledger, audit, claim planning, and the content — briefs, schemas, templates, and the default configuration — that the `vitrify` binary serves.

It exists to be built into `vitrify-cli` and holds no API across releases: each `vitrify-cli` release depends on exactly the `vitrify-core` released with it. To use vitrify, install the CLI:

```sh
cargo install vitrify-cli --locked
```

Source, documentation, and issues: <https://github.com/bitshock-src/vitrify>. Licensed under MIT — see [`LICENSE`](https://github.com/bitshock-src/vitrify/blob/main/LICENSE).
