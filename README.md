# memcastle-github

[![CI](https://github.com/memcastle/memcastle-github/actions/workflows/ci.yaml/badge.svg?branch=main)](https://github.com/memcastle/memcastle-github/actions/workflows/ci.yaml) [![Codecov](https://codecov.io/gh/memcastle/memcastle-github/graph/badge.svg)](https://codecov.io/gh/memcastle/memcastle-github)

Standalone source-only plugin for mining selected GitHub repositories, issues, pull requests, comments, reviews and
Git-backed wiki history into [MemCastle](https://github.com/memcastle/memcastle).

Install the versioned plugin with `memcastle plugin install github` (review its `curl`, `git`, `rm` and optional token
permissions), then `memcastle source enable github` and configure a miner as described in
[the source guide](sources/github/README.md).
The source retains the existing `github` identity and cursor format.

## Upgrading from the bundled source

Upgrade Core to a release that no longer bundles `github`, then inspect whether `github` was enabled and run
`memcastle source remove github --yes` to clear its old source-package record.
Removal keeps mined documents, source identity and cursor; it does not remove miners.
Run `memcastle plugin install github --adopt-source github`, review and consent to its process and environment grants,
then `memcastle source enable github` if it was previously enabled.
The plugin installer refuses the old source-package record and also refuses to take over previously mined history
without explicit adoption; use `--adopt-source github` only for a palace whose history belongs to this source.
If `github` was never enabled or mined, install the plugin normally without an adoption flag.

## Development

Requires Rust with `wasm32-wasip2`, Node.js and a plugin-capable MemCastle binary for migration checks.

```sh
memcastle plugin inspect --json
# Build sources/github for wasm32-wasip2 first.
memcastle plugin package
memcastle plugin test dist/github-VERSION.tar.gz
PATH="$PWD/sources/github/fixtures/bin:$PATH" memcastle source test sources/github --no-build
node scripts/smoke_adoption.mjs
```

Packaging writes an SCM-versioned archive in `dist/` and its external `.sha256sum`. `plugin.toml` remains an immutable
descriptor; `manifest.toml` is generated only inside the archive. Commit `sources/github/Cargo.lock` before releasing.
CI tests the exact built artifact against Core's shared plugin-validation workflow.
CI also uploads native Cargo coverage and test results to Codecov. gh-ship and git-cliff open a reviewed `release/next`
PR for releasable commits. Configure the `release` environment with `APP_CLIENT_ID` and `APP_PRIVATE_KEY` for an
installed GitHub App before publishing.
