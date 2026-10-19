# Contributing to memcastle-github

Install Rust with `wasm32-wasip2` and Node.js. The source code, offline REST/Git fixtures and manifest
live in `sources/github/`. Keep the `github` source ID, its option identity and cursor format stable;
review WIT changes, process/environment permissions and Core compatibility together.

## Development checks

Point `MEMCASTLE_BIN` to a plugin-capable Core binary. Build the component to
`sources/github/source.wasm` before packaging.

```sh
cargo test --manifest-path sources/github/Cargo.toml --locked
# Build the wasm32-wasip2 component and copy it to sources/github/source.wasm first.
archive="$("$MEMCASTLE_BIN" plugin package --json | jq -r .archive)"
"$MEMCASTLE_BIN" plugin test "$archive"
PATH="$PWD/sources/github/fixtures/bin:$PATH" "$MEMCASTLE_BIN" source test sources/github --no-build
ARCHIVE="$archive" node scripts/smoke_adoption.mjs
ARCHIVE="$archive" node scripts/smoke_scoped_miners.mjs
```

Core creates the hashed `manifest.toml` only inside the archive; `plugin.toml` remains unchanged.
Commit `sources/github/Cargo.lock` with implementation changes and do not commit generated archives.
CI builds and validates the exact release archive against Core's source-conformance and plugin-installation checks.

## Template updates and releases

Template answers and child commits are recorded in `.config/git.tpl.toml`.
To update, run `git tpl update --defaults --json`, review `git tpl diff --json`, then apply with
`git tpl merge --json`. Preserve the source's credential and legacy-adoption guidance when reconciling docs.
If native tests begin producing nonempty LCOV, update the covered-source answer and coverage target.

gh-ship uses git-cliff to propose a reviewed `release/next` PR and publishes `github/v<version>` after CI.
See the [README](README.md) for installation and GitHub App configuration.
