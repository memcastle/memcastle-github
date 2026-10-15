# memcastle-github

Release delivery uses gh-ship: releasable Conventional Commits open a `release/next` PR on pushes to `main`.
Review and merge that PR to publish a validated `github/vVERSION` archive. Configure the `release` environment
with `APP_CLIENT_ID` (variable) and `APP_PRIVATE_KEY` (secret) for a GitHub App installed on this repository with
contents, actions and pull requests write permissions (issues write for creating labels). Set squash title to PR title
and squash message to blank. `prepare-release` accepts `dry_run` without pushing.

Standalone source-only plugin for mining selected GitHub repositories, issues, pull requests, comments, reviews and
Git-backed wiki history into [MemCastle](https://github.com/memcastle/memcastle).

Install the versioned plugin with `memcastle plugin install github` (review its `curl`, `git`, `rm` and optional token
permissions), then `memcastle source enable github` and configure a miner as described in
[the source guide](modules/github/README.md).
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

Requires Rust with `wasm32-wasip2`, Python 3.11+, and a plugin-capable MemCastle binary for integration checks.

```sh
python3 scripts/package.py
python3 scripts/check_modules.py
python3 -m unittest discover -s tests
PATH="$PWD/modules/github/fixtures/bin:$PATH" memcastle source test modules/github --no-build
MEMCASTLE_BIN="$(command -v memcastle)" python3 scripts/smoke_install.py
MEMCASTLE_BIN="$(command -v memcastle)" python3 scripts/smoke_adoption.py
```

Packaging writes `dist/github-0.1.0.tar.gz` and the SHA-256 digests in `plugin.toml`.
Review and commit the manifest digests and `modules/github/Cargo.lock` before releasing.
CI tests the exact built artifact against Core's shared plugin-validation workflow.
