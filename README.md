# memcastle-github

[![CI][ci-badge]][ci-workflow] [![Codecov][codecov-badge]][codecov-project]

[ci-badge]: https://github.com/memcastle/memcastle-github/actions/workflows/ci.yaml/badge.svg?branch=main
[ci-workflow]: https://github.com/memcastle/memcastle-github/actions/workflows/ci.yaml
[codecov-badge]: https://codecov.io/gh/memcastle/memcastle-github/graph/badge.svg
[codecov-project]: https://codecov.io/gh/memcastle/memcastle-github

Source-only plugin for mining selected GitHub repositories, issues, pull requests, comments, reviews and
Git-backed wiki history into [MemCastle](https://github.com/memcastle/memcastle).

The source retains the stable `github` identity and cursor format. Use MemCastle Core with the optional
selected-credential contract from [Core #341](https://github.com/memcastle/memcastle/issues/341).
See [the source guide](sources/github/README.md) for GitHub credentials, scopes, multiple accounts and miners.

## Install

When a release is available in the [MemCastle plugin catalogue](https://memcastle.github.io/plugins.json),
install it by ID with a plugin-capable Core binary:

```sh
memcastle plugin install github
memcastle source enable github
```

Core downloads and verifies the release archive. Review the source's selected-credential, optional legacy token
environment and `curl`, `git`, `rm`, `sh` process permissions when prompted. Installing the plugin does not
activate its source. If there is no published release, build the archive below and install
`dist/github-VERSION.tar.gz` instead.

### Upgrading from the bundled source

Upgrade Core to a release that no longer bundles `github`, then check whether `github` was enabled and remove
its old source-package record with `memcastle source remove github --yes`. Removal preserves mined documents,
source identity, cursor and miners.

Run `memcastle plugin install github --adopt-source github`, review and consent to its permissions, then
`memcastle source enable github` if it was previously enabled. The installer refuses an old source-package
record or a takeover of mined history without explicit adoption. If it was never enabled or mined, install
the plugin normally without the adoption flag.

## Build and test locally

Install Rust with `wasm32-wasip2`, Node.js and a plugin-capable MemCastle binary.
Build the source component to `sources/github/source.wasm` before packaging; see CI for the reproducible
build and copy commands. Commit `sources/github/Cargo.lock` before release.

```sh
memcastle plugin inspect --json
memcastle plugin package
memcastle plugin test dist/github-VERSION.tar.gz
PATH="$PWD/sources/github/fixtures/bin:$PATH" memcastle source test sources/github --no-build
node scripts/smoke_adoption.mjs
```

Replace `VERSION` with the archive version in `dist/`. Core generates `manifest.toml` inside the archive
and a matching external `.sha256sum`, without modifying `plugin.toml`.

## CI and releases

CI validates the built artifact against Core `development` and `0.5` using the shared source-conformance
and installation workflow. The Core setup action verifies the binary against `SHA256SUMS`.
Native Cargo coverage and test results upload to Codecov with fixed 81% project and `source-github` floors
and an 80% patch floor; source coverage is not carried over from previous commits.

gh-ship and git-cliff open a reviewed `release/next` PR for releasable commits. After it merges,
the release workflow publishes the validated `github/vVERSION` archive. Configure `APP_CLIENT_ID` and
`APP_PRIVATE_KEY` in the `release` environment for an installed GitHub App.
The publish workflow uses this credential to check the draft release and upload its validated archive.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for project checks and [AGENTS.md](AGENTS.md) for agent guidance.
