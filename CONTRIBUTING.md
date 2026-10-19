# Contributing to memcastle-github

Install Rust with the `wasm32-wasip2` target and Node.js.
The source code, offline REST/Git fixtures and manifest live in `sources/github/`.
Keep the source ID `github`, its option identity and cursor format stable across releases.
Review changes to the WIT contract, process/environment permissions and Core compatibility together.

With `MEMCASTLE_BIN` pointing to a plugin-capable Core binary:

```sh
cargo test --manifest-path sources/github/Cargo.toml --locked
# Build the wasm32-wasip2 component and copy it to sources/github/source.wasm first.
archive="$("$MEMCASTLE_BIN" plugin package --json | jq -r .archive)"
"$MEMCASTLE_BIN" plugin test "$archive"
PATH="$PWD/sources/github/fixtures/bin:$PATH" "$MEMCASTLE_BIN" source test sources/github --no-build
ARCHIVE="$archive" node scripts/smoke_adoption.mjs
ARCHIVE="$archive" node scripts/smoke_scoped_miners.mjs
```

Core generates the hashed `manifest.toml` only inside the archive; commit `sources/github/Cargo.lock` with implementation changes.
CI builds and validates the exact `dist/github-<version>.tar.gz` against Core's shared source conformance and plugin
installation workflow.
gh-ship uses git-cliff to propose a `release/next` PR and publishes `github/v<version>` after review and CI.
