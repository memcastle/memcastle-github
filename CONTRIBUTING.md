# Contributing to memcastle-github

Install Rust with the `wasm32-wasip2` target and Python 3.11+.
The source code, offline REST/Git fixtures and manifest live in `modules/github/`.
Keep the source ID `github`, its option identity and cursor format stable across releases.
Review changes to the WIT contract, process/environment permissions and Core compatibility together.

With `MEMCASTLE_BIN` pointing to a plugin-capable Core binary:

```sh
cargo test --manifest-path modules/github/Cargo.toml --locked
# Build the wasm32-wasip2 component and copy it to modules/github/source.wasm first.
mise run package
"$MEMCASTLE_BIN" plugin test dist/github-0.1.0.tar.gz
PATH="$PWD/modules/github/fixtures/bin:$PATH" "$MEMCASTLE_BIN" source test modules/github --no-build
python3 scripts/smoke_install.py
python3 scripts/smoke_adoption.py
```

Review and commit the generated `manifest.toml` digests and `modules/github/Cargo.lock` together with implementation changes.
CI builds and validates the exact `dist/github-<version>.tar.gz` against Core's shared source conformance and plugin
installation workflow.
gh-ship uses git-cliff to propose a `release/next` PR and publishes `github/v<version>` after review and CI.
