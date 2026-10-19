# GitHub source

The `github` source is the WebAssembly module of the standalone `memcastle-github` plugin.
It reads repository metadata, issues, pull requests, optional conversation comments and reviews from the official REST API.
Repository wikis are separate Git repositories; this source reads their available commit history with Git and files each changed Markdown page at each commit as a separate document.
No semantic inference or GitHub SDK is part of the MemCastle core.

Install the reviewed plugin with `memcastle plugin install github`, then enable its source with
`memcastle source enable github`.
Install `curl` and `git` on the daemon's `PATH` before mining; Git also needs a writable `/tmp` for temporary bare clones.
For private repositories, configure a named source configuration with an env or file credential reference. The token value stays outside `memcastle.toml`.
Existing miners without a credential reference can continue using `GH_TOKEN` or `GITHUB_TOKEN` in the **daemon's** environment (in that precedence order). Legacy miner-level credential references are still supported. A public repository can be mined without a token, subject to GitHub's rate limits.
The source requests the selected-credential permission, optional access to those two legacy environment names, and the four programs `curl`, `git`, `rm` and `sh`; it requests no WASI filesystem or ambient network access.
Git runs only against `github.com` wiki clone URLs; a fixed shell wrapper receives the selected token on stdin and passes it to Git's credential helper through its child environment, not an argument or a file.

```toml
[[source_configs]]
id = "github-work"
source = "github"
credential = { type = "env", name = "GH_TOKEN" }

[[miners]]
name = "acme-issues"
source = "github"
source_config = "github-work"
wing = "engineering"
room = "tickets"
max_documents = 100

[miners.options]
include = ["acme/app", "acme/docs"]
topics = ["docs"]
pulls = false
wiki = false
metadata = false

[[miners]]
name = "acme-wiki"
source = "github"
source_config = "github-work"
wing = "knowledge"
mapping = { room_from = "kind" }

[miners.options]
include = ["acme/docs"]
wiki_include = ["acme/docs"]
wiki = true
issues = false
pulls = false
metadata = false
paths = ["guides/*"]

[miners.schedule]
every = "1d"
enabled = false
```

`include` is required: there is no implicit account-wide scope.
An exact `owner/repo` names one repository; `org/*` enumerates an organization's repositories and `owner/repo-*` selects a subset.
An explicit owner wildcard (`*/repo`) enumerates accessible repositories and needs a token.
`exclude` takes precedence over `include`.
`wiki_include` is a separate set of patterns **within** the selected repositories, and `wiki_exclude` takes precedence over it.
`wiki_include` opts in to wiki mining by default; set `wiki = false` to turn off an inherited wiki selection and its path/exclusion filters for one miner, or `wiki = true` with `wiki_include` to explicitly enable it.
To mine only wikis, set `issues = false`, `pulls = false` and `metadata = false`, as well as `wiki_include`.
The `labels` filter selects issues and PRs with at least one named label; `topics` selects repositories, and `paths` selects wiki page paths.
`labels` requires issues or pulls enabled; `paths` requires `wiki_include` (and takes effect when wiki mining is enabled). Unknown filters and ineffective combinations are rejected.
`comments` and `reviews` default to false, `issues`, `pulls` and `metadata` to true.

Source configuration options supply defaults; miner options replace matching keys. Put repository and content selections on individual miners for independent scopes. The selected credential is used for REST and private wikis; GitHub enforces the account's effective repository access. A missing referenced credential on an enabled miner makes it unavailable rather than using a different account. A miner's Wing, fixed Room or metadata mapping, document cap and schedule are managed by Core; schedules stay disabled until explicitly enabled. This plugin contains only the `github` source module and does not ingest Gists.

The adapter validates and canonicalizes scope before creating a job.
Changing a scope selector creates an independent source identity and cursor; `since` narrows a run without changing identity, so use `memcastle miner run acme-issues --full` to move the bootstrap boundary backward.
REST and Git pagination/history are bounded; narrow your selectors if a list exceeds 10,000 entries.
Git wiki commit history is discovered in Git's history order, and a rewritten history requires a full mine.
A changed issue, comment or review is discovered by its `updated_at` timestamp; changes whose upstream timestamps move backward require a full mine.
Polling and scheduled triggers can run the miner, but no trigger is enabled by defining it and webhook bodies never enter the source.
