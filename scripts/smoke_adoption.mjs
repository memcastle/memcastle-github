// A legacy GitHub miner keeps its cursor and mined documents after explicit adoption.
import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { join } from "node:path"
import { ROOT, cli, isolated, mine, pluginArchive } from "./smoke_common.mjs"

await isolated("memcastle-github-adopt-", ({ root, env }) => {
  env.PATH = `${join(ROOT, "sources/github/fixtures/bin")}:${process.env.PATH}`
  env.MEMCASTLE_CONFIG = join(root, "config.toml")
  const bundled = join(root, "bundled/github")
  mkdirSync(bundled, { recursive: true })
  cli(["source", "package", join(ROOT, "sources/github"), "--no-build"], env)
  const archive = join(ROOT, "sources/github/dist/github-0.1.0.tar.gz")
  const unpack = spawnSync("tar", ["-xzf", archive, "-C", bundled], { encoding: "utf8" })
  assert.equal(unpack.status, 0, unpack.stderr)
  writeFileSync(env.MEMCASTLE_CONFIG, `[mining]\nbundled_dir = "${join(root, "bundled")}"\n`)
}, async ({ root, request }) => {
  const plugin = readFileSync(pluginArchive())
  const enabled = await request("/api/source-packages/github/enable", { method: "POST" })
  assert.equal(enabled.state, "enabled")
  await request("/api/miners/github", { method: "PUT", json: { source: "github", options: { include: "acme/alpha" } } })
  const run = async () => {
    const job = await mine(request, "github", "fixture", { include: "acme/alpha" })
    assert.equal(job.status, "completed", JSON.stringify(job))
    return job.result
  }
  assert.equal((await run()).created, 3)
  const before = (await request("/api/sources")).sources.find((item) => item.source === "github")
  rmSync(join(root, "bundled/github"), { recursive: true })
  await assert.rejects(request("/api/plugins/preview", { method: "POST", data: plugin }), { status: 409 })
  await request("/api/source-packages/github", { method: "DELETE" })
  const preview = await request("/api/plugins/preview", { method: "POST", data: plugin })
  assert.deepEqual(preview.adoptions, ["github"])
  const consents = Object.fromEntries(preview.source_consents.map(({ source, digest }) => [source, digest]))
  await assert.rejects(request("/api/plugins", { method: "POST", data: plugin,
    headers: { "X-MemCastle-Consents": JSON.stringify(consents) } }), { status: 409 })
  await request("/api/plugins", { method: "POST", data: plugin, headers: {
    "X-MemCastle-Consents": JSON.stringify(consents), "X-MemCastle-Adopt-Sources": '["github"]',
  } })
  const after = (await request("/api/sources")).sources.find((item) => item.source === "github")
  assert.deepEqual([after.cursor, after.documents], [before.cursor, before.documents])
  assert.equal((await request("/api/miners/github")).source, "github")
  assert.equal((await request("/api/source-packages/github")).state, "installed")
  await request("/api/source-packages/github/enable", { method: "POST" })
  assert.equal((await run()).documents, 0, "adoption rewound the source cursor")
})
console.log("legacy GitHub miner, cursor and mined data survived explicit plugin adoption")
