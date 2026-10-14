# github

Scoped GitHub repositories, issues, pull requests and wiki history.

This provider plugin owns the versioned package, while each module has its own lifecycle.
Installing the plugin makes its modules available; enable sources and install integrations separately.

```sh
python3 scripts/package.py
python3 scripts/check_modules.py
python3 -m unittest discover -s tests
MEMCASTLE_BIN=/path/to/memcastle python3 scripts/smoke_install.py
```

The archive in `dist/` is independently releasable from MemCastle Core.
The root `plugin.toml` declares stable module identities; the package script fills artifact hashes after building.
Commit each source module's generated `Cargo.lock` before publishing a release.
The source module carries the versioned public WIT contract and its own fixtures; the agent integration uses MCP/HTTP.
No module needs a private Core checkout or a sibling provider to build or test.
The smoke check starts an isolated daemon and uses a temporary agent configuration to verify the archive install path.
Provider CI builds and packages locally, then calls Core's shared artifact-validation workflow for a `[main, versions...]`
matrix, with additional exact plugin-capable
stable versions configured in `core-versions` and the `main` prerelease always included.
The reusable validation workflow follows Core's `main` branch without a revision pin.
It runs on every pull request and every push to `main`, including merges, without a second push run on PR branches.
Every version exercises source fixtures and archive installation/activation; incompatible versions fail the workflow.
The development release body records the exact Core source SHA, and the workflow summary reports it for each run.
Core CI separately tests a freshly composed project against its current install contract.
The provider's own release workflow accepts only a `github/v<plugin.toml version>` tag on `main`, then builds
the reviewed artifact, runs manifest and compatibility-matrix tests, and refuses an existing release.
Keep IDs stable when moving this project from `plugins/github/` to its own repository.

Releases use gh-ship: merging a releasable Conventional Commit into `main` opens a `release/next` PR.
Merging that PR publishes a validated `github/vVERSION` archive as a GitHub Release.
Set up the `release` environment with variable `APP_CLIENT_ID` and secret `APP_PRIVATE_KEY`
for an App installed on this repository.
Grant contents, actions and pull requests write permission; issues write is needed to create the release label.
Set squash merge title to PR title and body to blank, and enable this repository's release environment.
The `prepare-release` workflow honors its `dry_run` input without pushing or committing.

