#!/usr/bin/env python3
"""Prove a previously enabled GitHub source keeps its mined cursor on explicit adoption."""

import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import tarfile
import tempfile
import time
from urllib.error import HTTPError, URLError

from smoke_install import ROOT, request


def main() -> None:
    binary = Path(os.environ["MEMCASTLE_BIN"]).resolve()
    plugin = (ROOT / "dist/github-0.1.0.tar.gz").read_bytes()
    # The legacy package supplies exactly the files a former Core release bundled, without a second WASM build.
    subprocess.run((str(binary), "source", "package", str(ROOT / "modules/github"), "--no-build"),
                   check=True, capture_output=True)
    with tempfile.TemporaryDirectory(prefix="memcastle-github-adopt-") as scratch:
        root = Path(scratch)
        (root / "assets").mkdir()
        bundled = root / "bundled/github"
        bundled.mkdir(parents=True)
        with tarfile.open(ROOT / "modules/github/dist/github-0.1.0.tar.gz", "r:gz") as archive:
            archive.extractall(bundled, filter="data")
        config = root / "config.toml"
        config.write_text(f'[mining]\nbundled_dir = "{bundled.parent}"\n')
        env = {key: value for key, value in os.environ.items() if not key.startswith("MEMCASTLE_")}
        env.pop("GH_TOKEN", None)
        env.pop("GITHUB_TOKEN", None)
        env.update({"HOME": scratch, "XDG_DATA_HOME": str(root / "data"),
                    "XDG_CONFIG_HOME": str(root / "config"), "XDG_STATE_HOME": str(root / "state"),
                    "MEMCASTLE_ASSETS_DIR": str(root / "assets"), "MEMCASTLE_CONFIG": str(config),
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
                raise AssertionError("disposable daemon did not become healthy")
            installed = request(base, "/api/source-packages/github/enable", method="POST")
            assert installed["origin"] == "bundled" and installed["state"] == "enabled", installed
            miner = json.dumps({"source": "github", "options": {"include": "acme/alpha"}}).encode()
            request(base, "/api/miners/github", method="PUT", data=miner,
                    headers={"Content-Type": "application/json"})
            def mine():
                job = request(base, "/api/jobs", method="POST", data=json.dumps({
                    "type": "mine", "source": "github", "locator": "fixture", "requested_by": "test",
                    "options": {"include": "acme/alpha"},
                }).encode(), headers={"Content-Type": "application/json"})
                for _ in range(300):
                    result = request(base, f"/api/jobs/{job['id']}")
                    if result["status"] in ("completed", "failed", "cancelled"):
                        assert result["status"] == "completed", result
                        return result["result"]
                    time.sleep(0.1)
                raise AssertionError("GitHub mining did not complete within 30 seconds")
            assert mine()["created"] == 3
            before = next(item for item in request(base, "/api/sources")["sources"] if item["source"] == "github")
            # Simulate the Core upgrade removing this bundle before the explicit source-record cleanup.
            shutil.rmtree(bundled)
            try:
                request(base, "/api/plugins/preview", method="POST", data=plugin)
            except HTTPError as error:
                assert error.code == 409, error.read()
            else:
                raise AssertionError("the plugin replaced a bundled source row without its removal")
            request(base, "/api/source-packages/github", method="DELETE")
            preview = request(base, "/api/plugins/preview", method="POST", data=plugin)
            assert preview["adoptions"] == ["github"], preview
            consents = {item["source"]: item["digest"] for item in preview["source_consents"]}
            try:
                request(base, "/api/plugins", method="POST", data=plugin,
                        headers={"X-MemCastle-Consents": json.dumps(consents)})
            except HTTPError as error:
                assert error.code == 409, error.read()
            else:
                raise AssertionError("legacy mined data was adopted without explicit permission")
            request(base, "/api/plugins", method="POST", data=plugin, headers={
                "X-MemCastle-Consents": json.dumps(consents),
                "X-MemCastle-Adopt-Sources": json.dumps(["github"]),
            })
            after = next(item for item in request(base, "/api/sources")["sources"] if item["source"] == "github")
            assert before["cursor"] == after["cursor"] and before["documents"] == after["documents"]
            assert request(base, "/api/miners/github")["source"] == "github"
            assert request(base, "/api/source-packages/github")["state"] == "installed"
            request(base, "/api/source-packages/github/enable", method="POST")
            assert mine()["documents"] == 0, "adoption must not rewind the source cursor"
        finally:
            process.terminate()
            try:
                process.communicate(timeout=20)
            except subprocess.TimeoutExpired:
                process.kill()
                process.communicate()
    print("legacy GitHub miner, cursor and mined data survived explicit plugin adoption")


if __name__ == "__main__":
    main()
