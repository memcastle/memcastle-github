// One selected GitHub credential, independently scoped miners and destinations.
import assert from "node:assert/strict"
import { join } from "node:path"
import { ROOT, isolated, pluginArchive } from "./smoke_common.mjs"
import { readFileSync } from "node:fs"

async function install(request) {
  const archive = readFileSync(pluginArchive())
  const preview = await request("/api/plugins/preview", { method: "POST", data: archive })
  const consents = Object.fromEntries(preview.source_consents.map(({ source, digest }) => [source, digest]))
  await request("/api/plugins", { method: "POST", data: archive,
    headers: { "X-MemCastle-Consents": JSON.stringify(consents) } })
  await request("/api/source-packages/github/enable", { method: "POST" })
}

await isolated("memcastle-github-scopes-", ({ env }) => {
  env.PATH = `${join(ROOT, "sources/github/fixtures/bin")}:${process.env.PATH}`
  env.GH_TOKEN = "fixture-selected-token"
  env.GITHUB_TOKEN = "rejected-token"
}, async ({ request }) => {
  await install(request)

  await request("/api/source-configs/github-team", { method: "PUT", json: {
    source: "github", credential: { type: "env", name: "GH_TOKEN" },
    options: { include: ["acme/*"] },
  } })
  await request("/api/source-configs/github-denied", { method: "PUT", json: {
    source: "github", credential: { type: "env", name: "GITHUB_TOKEN" },
    options: { include: ["acme/alpha"] },
  } })
  const definitions = [
    ["docs-issues", { source: "github", source_config: "github-team", wing: "docs",
      options: { include: ["acme/alpha"], topics: ["docs"], pulls: false, metadata: false } }],
    ["other-metadata", { source: "github", source_config: "github-team", wing: "other",
      max_documents: 1, options: { include: ["acme/other"], topics: ["work"], issues: false, pulls: false } }],
    ["archive-issues", { source: "github", source_config: "github-team", wing: "archive",
      mapping: { room_from: "kind" },
      options: { include: ["acme/alpha"], topics: ["docs"], pulls: false, metadata: false } }],
    ["docs-wiki", { source: "github", source_config: "github-team", wing: "wikis",
      options: { include: ["acme/alpha"], wiki_include: ["acme/alpha"], wiki: true,
        issues: false, pulls: false, metadata: false } }],
  ]
  for (const [name, definition] of definitions) {
    await request(`/api/miners/${name}`, { method: "PUT", json: definition })
  }
  await request("/api/miners/denied", { method: "PUT", json: {
    source: "github", source_config: "github-denied", wing: "denied",
  } })
  async function run(name) {
    const submitted = await request(`/api/miners/${name}/run`, { method: "POST" })
    for (let attempt = 0; attempt < 300; attempt++) {
      const job = await request(`/api/jobs/${submitted.id}`)
      if (job.status === "completed") return job.result
      assert.notEqual(job.status, "failed", JSON.stringify(job))
      await new Promise((resolve) => setTimeout(resolve, 100))
    }
    throw new Error(`${name} did not finish within 30 seconds`)
  }
  const first = await Promise.all(definitions.map(([name]) => run(name)))
  assert.deepEqual(first.map((result) => result.created), [1, 1, 1, 3])
  assert.equal(new Set(first.map((result) => result.source_id)).size, definitions.length,
    "scope and destination must have independent source identities")
  for (const [wing, room, query] of [
    ["docs", "issues", "Item"],
    ["archive", "issue", "Item"],
    ["other", "repositories", "Example"],
    ["wikis", "wikis", "Home"],
  ]) {
    const matches = await request(`/api/search?q=${encodeURIComponent(query)}&wing=${wing}&room=${room}`)
    assert.ok(matches.length > 0, `missing ${wing}/${room} documents`)
  }
  for (const [name] of definitions) {
    const resumed = await run(name)
    assert.equal(resumed.documents, 0, `${name} must resume its own cursor`)
  }
  const refused = await request("/api/miners/denied/run", { method: "POST" })
  let denied
  for (let attempt = 0; attempt < 300; attempt++) {
    const job = await request(`/api/jobs/${refused.id}`)
    if (job.status === "failed") { denied = job; break }
    assert.notEqual(job.status, "completed", "denied account must not mine any documents")
    await new Promise((resolve) => setTimeout(resolve, 100))
  }
  assert.ok(denied?.error?.includes("refused"), JSON.stringify(denied))
  assert.ok(!JSON.stringify(denied).includes("rejected-token"), "credential must never be reported")
  const sources = (await request("/api/sources")).sources
  for (const { source_id } of first) {
    assert.ok(sources.some((source) => source.id === source_id || source.source_id === source_id),
      `missing source for ${source_id}`)
  }
})

await isolated("memcastle-github-public-", ({ env }) => {
  env.PATH = `${join(ROOT, "sources/github/fixtures/bin")}:${process.env.PATH}`
  delete env.GH_TOKEN
  delete env.GITHUB_TOKEN
}, async ({ request }) => {
  await install(request)
  await request("/api/miners/public", { method: "PUT", json: {
    source: "github", options: { include: ["acme/alpha"] },
  } })
  const queued = await request("/api/miners/public/run", { method: "POST" })
  let completed
  for (let attempt = 0; attempt < 300; attempt++) {
    const job = await request(`/api/jobs/${queued.id}`)
    if (job.status === "completed") { completed = job; break }
    assert.notEqual(job.status, "failed", JSON.stringify(job))
    await new Promise((resolve) => setTimeout(resolve, 100))
  }
  assert.equal(completed?.result?.created, 3, "public repositories need no credential reference")
})
console.log("shared GitHub credential, independent scopes, and credential-free public mining verified")
