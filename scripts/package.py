#!/usr/bin/env python3
"""Build and deterministically package the reviewed GitHub source plugin."""

import gzip
import hashlib
import io
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tomllib

ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "plugin.toml"


def safe_file(name: str) -> Path:
    parts = name.split("/")
    if not name or "\\" in name or ":" in name or any(part in ("", ".", "..") for part in parts) or Path(name).is_absolute():
        raise ValueError(f"unsafe module path {name!r}")
    path = ROOT
    for part in parts:
        path = path / part
        if path.is_symlink():
            raise ValueError(f"module path {name!r} traverses a symlink")
    if not path.is_file():
        raise ValueError(f"missing module file {name!r}")
    return path


def main() -> None:
    text = MANIFEST.read_text()
    plugin = tomllib.loads(text)
    modules = plugin["modules"]
    if len(modules) != 1 or modules[0]["type"] != "source" or modules[0]["id"] != "github":
        raise ValueError("GitHub publishes exactly one source module")
    module = modules[0]
    directory = ROOT / "modules/github"
    if (ROOT / "modules").is_symlink() or directory.is_symlink():
        raise ValueError("source project must not be reached through a symlink")
    if not (directory / "Cargo.lock").is_file():
        raise ValueError("commit modules/github/Cargo.lock before packaging")
    cargo_home = Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo")).resolve()
    sysroot = Path(subprocess.check_output(["rustc", "--print", "sysroot"], text=True).strip())
    version = subprocess.check_output(["rustc", "--version", "--verbose"], text=True)
    commit = next(line.removeprefix("commit-hash: ") for line in version.splitlines() if line.startswith("commit-hash: "))
    env = os.environ.copy()
    env["CARGO_BUILD_RUSTC_WRAPPER"] = ""
    env.pop("RUSTFLAGS", None)
    env["CARGO_ENCODED_RUSTFLAGS"] = "\x1f".join((
        f"--remap-path-prefix={cargo_home}=/cargo",
        f"--remap-path-prefix={sysroot / 'lib/rustlib/src/rust'}=/rustc/{commit}",
        f"--remap-path-prefix={directory}=/module",
    ))
    subprocess.run(["cargo", "build", "--locked", "--release", "--target", "wasm32-wasip2"],
                   cwd=directory, env=env, check=True)
    built = directory / "target/wasm32-wasip2/release/memcastle_source_github.wasm"
    if not built.is_file():
        raise ValueError(f"missing built component: {built}")
    entry = ROOT / module["entry"]
    if entry != directory / "source.wasm" or entry.is_symlink():
        raise ValueError("source component must be a regular file in modules/github")
    shutil.copyfile(built, entry)
    conformance = directory / "dist/source.wasm"
    # A stale link from a previous build must not turn packaging into a write outside this checkout.
    if conformance.parent.is_symlink() or conformance.is_symlink():
        raise ValueError("conformance component destination must not be a symlink")
    conformance.parent.mkdir(exist_ok=True)
    shutil.copyfile(built, conformance)

    files = {name: safe_file(name).read_bytes() for name in (module["manifest"], module["entry"])}
    for key, name in (("manifest_sha256", module["manifest"]), ("sha256", module["entry"])):
        before = f'{key} = "{module[key]}"'
        if text.count(before) != 1:
            raise ValueError(f"ambiguous {key} in plugin.toml")
        text = text.replace(before, f'{key} = "{hashlib.sha256(files[name]).hexdigest()}"', 1)
    MANIFEST.write_text(text)
    files["plugin.toml"] = text.encode()
    destination = ROOT / "dist" / f"github-{plugin['plugin']['version']}.tar.gz"
    destination.parent.mkdir(exist_ok=True)
    with destination.open("wb") as output, gzip.GzipFile(fileobj=output, mode="wb", filename="", mtime=0) as compressed:
        with tarfile.open(fileobj=compressed, mode="w") as archive:
            for name, data in sorted(files.items()):
                info = tarfile.TarInfo(name)
                info.size = len(data)
                info.mode = 0o644
                info.mtime = 0
                archive.addfile(info, io.BytesIO(data))
    print(destination)


if __name__ == "__main__":
    main()
