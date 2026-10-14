"""Keep the standalone source identity and artifact declarations aligned."""

import hashlib
from pathlib import Path
import tarfile
import tomllib
import unittest

ROOT = Path(__file__).resolve().parents[1]


class PluginManifestTests(unittest.TestCase):
    def test_github_is_the_only_module_and_its_compatibility_matches_the_parent(self):
        plugin = tomllib.loads((ROOT / "plugin.toml").read_text())
        self.assertEqual(plugin["plugin"]["id"], "github")
        self.assertEqual(plugin["plugin"]["repository"], "https://github.com/memcastle/memcastle-github")
        module, = plugin["modules"]
        self.assertEqual((module["type"], module["id"]), ("source", "github"))
        source = tomllib.loads((ROOT / module["manifest"]).read_text())
        self.assertEqual((source["source"]["name"], source["source"]["version"]),
                         (module["id"], module["version"]))
        self.assertEqual(source["compatibility"],
                         {"contract": module["contract"], "memcastle": plugin["memcastle"]})

    def test_archive_contains_only_the_reviewed_source_artifacts(self):
        plugin = tomllib.loads((ROOT / "plugin.toml").read_text())
        module, = plugin["modules"]
        archive = ROOT / "dist" / f"github-{plugin['plugin']['version']}.tar.gz"
        with tarfile.open(archive, "r:gz") as package:
            self.assertEqual(package.getnames(), sorted(("plugin.toml", module["manifest"], module["entry"])))
            for key, name in (("manifest_sha256", module["manifest"]), ("sha256", module["entry"])):
                self.assertEqual(hashlib.sha256(package.extractfile(name).read()).hexdigest(), module[key])


if __name__ == "__main__":
    unittest.main()
