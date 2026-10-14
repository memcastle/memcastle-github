# Contributing to memcastle-github

Install Rust with the `wasm32-wasip2` target and Python 3.11+.
The source code, offline REST/Git fixtures and manifest live in `modules/github/`.
Keep the source ID `github`, its option identity and cursor format stable across releases.
Review changes to the WIT contract, process/environment permissions and Core compatibility together.

With `MEMCASTLE_BIN` pointing to a plugin-capable Core binary:

```sh
mise run package
mise run test
python3 -m unittest discover -s tests
PATH="$PWD/modules/github/fixtures/bin:$PATH" "$MEMCASTLE_BIN" source test modules/github --no-build
python3 scripts/smoke_install.py
python3 scripts/smoke_adoption.py
```

Review and commit `plugin.toml`'s generated digests and `modules/github/Cargo.lock` together with implementation changes.
CI builds and validates the exact `dist/github-<version>.tar.gz` against Core's shared source conformance and plugin
installation workflow.
Release tags are `github/v<version>` on `main`; publish only after the validation workflow passes.
