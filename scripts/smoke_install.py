#!/usr/bin/env python3
"""Exercise the actual GitHub component against offline commands and a disposable Core daemon."""

import hashlib
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time
import tomllib
from urllib.error import HTTPError, URLError
from urllib.parse import quote
from urllib.request import Request, urlopen

ROOT = Path(__file__).resolve().parents[1]


def request(base, route, *, method="GET", data=None, headers=None):
    with urlopen(Request(base + route, data=data, method=method, headers=headers or {}), timeout=20) as response:
        return json.load(response)


def job_result(base, options):
    payload = {"type": "mine", "source": "github", "locator": "fixture", "requested_by": "test", "options": options}
    job = request(base, "/api/jobs", method="POST", data=json.dumps(payload).encode(),
                  headers={"Content-Type": "application/json"})
    for _ in range(300):
        result = request(base, f"/api/jobs/{job['id']}")
        if result["status"] in ("completed", "failed", "cancelled"):
            return result
        time.sleep(0.1)
    raise AssertionError("GitHub mining did not complete within 30 seconds")


def mine(base, options=None):
    result = job_result(base, options or {"include": "acme/alpha"})
    assert result["status"] == "completed", result
    return result["result"]


def main():
    binary = Path(os.environ["MEMCASTLE_BIN"]).resolve()
    manifest = tomllib.loads((ROOT / "plugin.toml").read_text())
    archive = (ROOT / "dist" / f"github-{manifest['plugin']['version']}.tar.gz").read_bytes()
    with tempfile.TemporaryDirectory(prefix="memcastle-github-smoke-") as scratch:
        root = Path(scratch)
        (root / "assets").mkdir()
        (root / "github-0.1.0.tar.gz").write_bytes(archive)
        registry = root / "plugins.json"
        registry.write_text(json.dumps({"format": 1, "name": "GitHub smoke registry", "plugins": [{
            "id": "github", "description": manifest["plugin"]["description"],
            "repository": "memcastle/memcastle-github", "modules": [{"id": "github", "type": "source"}],
            "versions": [{"version": manifest["plugin"]["version"], "url": "github-0.1.0.tar.gz",
                          "sha256": hashlib.sha256(archive).hexdigest()}],
        }]}))
        config = root / "config.toml"
        config.write_text(f'[plugins]\nregistries = ["{registry}"]\n')
        env = {key: value for key, value in os.environ.items() if not key.startswith("MEMCASTLE_")}
        env.pop("GH_TOKEN", None)
        env.pop("GITHUB_TOKEN", None)
        env.update({"HOME": scratch, "XDG_DATA_HOME": str(root / "data"),
                    "XDG_CONFIG_HOME": str(root / "config"), "XDG_STATE_HOME": str(root / "state"),
                    "MEMCASTLE_CONFIG": str(config), "MEMCASTLE_ASSETS_DIR": str(root / "assets"),
                    "PATH": str(ROOT / "modules/github/fixtures/bin") + os.pathsep + os.environ["PATH"]})
        with socket.socket() as probe:
            probe.bind(("127.0.0.1", 0))
            port = probe.getsockname()[1]
        base = f"http://127.0.0.1:{port}"
        process = subprocess.Popen((str(binary), "--palace", str(root / "palace"), "serve",
                                    "--bind", "127.0.0.1", "--port", str(port)),
                                   cwd=root, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        try:
            for _ in range(120):
                if process.poll() is not None:
                    raise RuntimeError(f"daemon exited: {process.stderr.read().decode()}")
                try:
                    request(base, "/api/health")
                    break
                except (URLError, TimeoutError):
                    time.sleep(0.25)
            else:
                raise AssertionError("temporary daemon did not become healthy")
            listing = request(base, "/api/plugin-registry/search?q=github")
            assert listing["plugins"][0]["id"] == "github" and not listing["warnings"], listing
            indexed = request(base, "/api/plugin-registry/plugins/github")
            assert indexed["archive_digest"] == hashlib.sha256(archive).hexdigest(), indexed
            preview = request(base, "/api/plugins/preview", method="POST", data=archive)
            assert [(item["type"], item["id"]) for item in preview["modules"]] == [("source", "github")]
            consents = {item["source"]: item["digest"] for item in preview["source_consents"]}
            assert set(consents) == {"github"}, preview
            try:
                request(base, "/api/plugins", method="POST", data=archive)
            except HTTPError as error:
                assert error.code == 400, error.read()
            else:
                raise AssertionError("a source with process and environment grants installed without consent")
            installed = request(base, "/api/plugin-registry/install", method="POST",
                                headers={"Content-Type": "application/json"},
                                data=json.dumps({"id": "github", "version": manifest["plugin"]["version"],
                                                 "archive_digest": indexed["archive_digest"], "consents": consents}).encode())
            assert installed["id"] == "github", installed
            assert request(base, "/api/source-packages/github")["state"] == "installed"
            request(base, "/api/source-packages/github/enable", method="POST")
            assert mine(base)["created"] == 3
            hits = request(base, "/api/search?q=" + quote("Example repository"))
            assert any(hit["source"]["origin"]["source"] == "github" for hit in hits), hits
            assert mine(base)["documents"] == 0
            assert mine(base, {"include": "acme/*", "exclude": "acme/other"})["documents"] == 3
            assert mine(base, {"include": "acme/alpha", "comments": "true", "reviews": "true"})["created"] >= 3
            assert request(base, "/api/search?q=" + quote("Inline review")), "review comments must be searchable"
            assert mine(base, {"include": "acme/alpha", "issues": "false", "pulls": "false",
                               "metadata": "false", "wiki_include": "acme/alpha"})["created"] == 3
            assert request(base, "/api/search?q=" + quote("Home v2")), "wiki revisions must be searchable"
            assert mine(base, {"include": "acme/alpha", "wiki_include": "acme/alpha", "issues": "false",
                               "pulls": "false", "metadata": "false", "paths": "Guide.md",
                               "since": "2026-01-04"})["documents"] == 1
            assert mine(base, {"include": "acme/alpha", "wiki_include": "acme/alpha", "comments": "true",
                               "reviews": "true"})["documents"] == 9
            missing = job_result(base, {"metadata": "true"})
            assert missing["status"] == "failed" and "include" in missing["error"], missing
        finally:
            process.terminate()
            try:
                process.communicate(timeout=20)
            except subprocess.TimeoutExpired:
                process.kill()
                process.communicate()
        # A failed authenticated request must not put the daemon's token into a job error.
        env["GH_TOKEN"] = "rejected-token"
        process = subprocess.Popen((str(binary), "--palace", str(root / "palace"), "serve",
                                    "--bind", "127.0.0.1", "--port", str(port)),
                                   cwd=root, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        try:
            for _ in range(120):
                if process.poll() is not None:
                    raise RuntimeError(f"daemon exited: {process.stderr.read().decode()}")
                try:
                    request(base, "/api/health")
                    break
                except (URLError, TimeoutError):
                    time.sleep(0.25)
            else:
                raise AssertionError("temporary daemon did not restart")
            failed = job_result(base, {"include": "acme/alpha", "account": "rejected-credentials"})
            assert failed["status"] == "failed" and "rejected-token" not in failed["error"], failed
        finally:
            process.terminate()
            try:
                process.communicate(timeout=20)
            except subprocess.TimeoutExpired:
                process.kill()
                process.communicate()
    print("GitHub plugin discovery, consent, install, mining and cursor passed")


if __name__ == "__main__":
    main()
