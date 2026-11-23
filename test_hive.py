import json
import os
import tempfile
import time
import unittest
from pathlib import Path

import hive

NOTE = "---\ntype: {type}\nproject: demo\nagent: claude\nstatus: {status}\nupdated: 2026-11-15\n---\n# {title}\n\n{body}\n"


def write(root, rel, **kw):
    path = Path(root) / rel
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(NOTE.format(**kw), encoding="utf-8")
    return path


class VaultIndexTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.vault = Path(self.tmp.name) / "vault"
        write(self.vault, "projects/demo/adrs/0001-cache.md", type="adr", status="active",
              title="Use Redis cache", body="We cache prices.")
        write(self.vault, "projects/demo/adrs/0000-old.md", type="adr", status="superseded",
              title="Use Redis cache v0", body="Old decision about cache.")
        write(self.vault, "projects/demo/graph/noise.md", type="note", status="active",
              title="Redis cache export", body="generated")
        self.idx = hive.Index(self.vault, Path(self.tmp.name) / "hive.sqlite")

    def tearDown(self):
        self.idx.db.close()
        self.tmp.cleanup()

    def test_parse_note_reads_flat_frontmatter_and_title(self):
        meta, title, body = hive.parse_note(NOTE.format(type="adr", status="active", title="T", body="b"))
        self.assertEqual((meta["type"], meta["project"], title), ("adr", "demo", "T"))
        self.assertNotIn("---", body)

    def test_search_skips_export_dirs_and_demotes_superseded(self):
        hits = self.idx.search("redis cache", "demo")
        paths = [h["path"] for h in hits]
        self.assertEqual(paths[0], "projects/demo/adrs/0001-cache.md")
        self.assertIn("projects/demo/adrs/0000-old.md", paths)
        self.assertNotIn("projects/demo/graph/noise.md", paths)

    def test_meta_field_matches_project_and_autocut_drops_weak_tail(self):
        other = self.vault / "projects/other/notes.md"
        other.parent.mkdir(parents=True, exist_ok=True)
        other.write_text("# Unrelated\n\nMentions cache once.\n", encoding="utf-8")
        self.idx.refresh(force=True)
        hits = self.idx.search("demo adr", "")  # words only in project/type/path metadata
        self.assertTrue(hits and all(h["path"].startswith("projects/demo/") for h in hits))
        top = hits[0]["score"]
        self.assertTrue(all(h["score"] >= hive.AUTOCUT_RATIO * top for h in self.idx.search("cache")))

    def test_refresh_picks_up_edits_and_deletes(self):
        self.idx.search("redis")
        path = write(self.vault, "projects/demo/adrs/0001-cache.md", type="adr", status="active",
                     title="Use Valkey store", body="Switched.")
        os.utime(path, (time.time() + 5, time.time() + 5))
        (self.vault / "projects/demo/adrs/0000-old.md").unlink()
        self.assertEqual(self.idx.refresh(force=True), 2)
        self.assertEqual(self.idx.search("valkey")[0]["path"], "projects/demo/adrs/0001-cache.md")
        self.assertEqual(self.idx.search("redis"), [])


class GraphTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        nodes = [{"id": i, "label": label, "source_file": f"src/{i}.py", "source_location": "L1",
                  "community_name": "c", "file_type": "code"}
                 for i, label in [("a", "PriceFetcher"), ("b", "parse_rows"), ("c", "store_rows"),
                                  ("far", "UnrelatedThing"), ("hub", "utils")]]
        nodes += [{"id": f"x{k}", "label": f"leaf{k}", "source_file": "src/x.py"} for k in range(30)]
        links = [{"source": "a", "target": "b", "relation": "calls", "confidence_score": 1.0},
                 {"source": "b", "target": "c", "relation": "calls", "confidence_score": 1.0},
                 {"source": "c", "target": "far", "relation": "calls", "confidence_score": 1.0},
                 {"source": "a", "target": "hub", "relation": "imports", "confidence_score": 1.0}]
        links += [{"source": "hub", "target": f"x{k}", "relation": "contains", "confidence_score": 1.0} for k in range(30)]
        self.path = Path(self.tmp.name) / "graph.json"
        self.path.write_text(json.dumps({"built_at_commit": "abc", "nodes": nodes, "links": links}))
        self.g = hive.Graph(self.path)

    def tearDown(self):
        self.tmp.cleanup()

    def test_seeds_match_camel_case_tokens(self):
        self.assertEqual(self.g.seeds("price fetcher")[0], "a")

    def test_ppr_ranks_direct_callee_over_hub_and_far_nodes(self):
        ranked, edges = self.g.query("PriceFetcher", limit=5)
        order = [nid for nid, _, _ in ranked]
        self.assertEqual(order[0], "a")
        self.assertEqual(order[1], "b")
        self.assertNotIn("far", order)  # 3 hops away, outside the 2-hop neighbourhood
        self.assertIn(("a", "calls", "b"), edges)

    def test_pack_respects_budget(self):
        out = hive.pack(["x" * 400, "y" * 4000, "z" * 400], budget=300)
        self.assertLessEqual(hive.est_tokens(out), 320)
        self.assertNotIn("z", out)


class McpTest(unittest.TestCase):
    def test_legacy_initialize_echoes_supported_version(self):
        r = hive.handle({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}})
        self.assertEqual(r["result"]["protocolVersion"], "2025-06-18")
        self.assertIn("tools", r["result"]["capabilities"])

    def test_modern_discover_and_version_error(self):
        meta = {"_meta": {hive.META_VERSION: "2026-07-28"}}
        r = hive.handle({"jsonrpc": "2.0", "id": 2, "method": "server/discover", "params": meta})
        self.assertEqual(r["result"]["resultType"], "complete")
        self.assertEqual(r["result"]["supportedVersions"], ["2026-07-28"])
        bad = hive.handle({"jsonrpc": "2.0", "id": 3, "method": "tools/list",
                           "params": {"_meta": {hive.META_VERSION: "1900-01-01"}}})
        self.assertEqual(bad["error"]["code"], -32022)

    def test_tools_list_and_errors_stay_in_band(self):
        tools = hive.handle({"jsonrpc": "2.0", "id": 4, "method": "tools/list"})["result"]["tools"]
        self.assertEqual({t["name"] for t in tools}, set(hive.TOOLS))
        unknown = hive.handle({"jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": {"name": "nope"}})
        self.assertEqual(unknown["error"]["code"], -32602)
        missing = hive.handle({"jsonrpc": "2.0", "id": 6, "method": "tools/call",
                               "params": {"name": "hive_graph", "arguments": {}}})
        self.assertTrue(missing["result"]["isError"])
        self.assertIsNone(hive.handle({"jsonrpc": "2.0", "method": "notifications/initialized"}))


class SpecAndFreshnessTest(unittest.TestCase):
    def test_tools_are_read_only_and_modern_lists_carry_cache_fields(self):
        tools = hive.tool_list()
        self.assertTrue(all(t["annotations"]["readOnlyHint"] for t in tools))
        ctx = next(t for t in tools if t["name"] == "hive_context")
        self.assertTrue(ctx["_meta"]["anthropic/alwaysLoad"])
        modern = hive.handle({"jsonrpc": "2.0", "id": 1, "method": "tools/list",
                              "params": {"_meta": {hive.META_VERSION: "2026-07-28"}}})["result"]
        self.assertEqual((modern["ttlMs"], modern["cacheScope"]), (3600000, "public"))
        legacy = hive.handle({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})["result"]
        self.assertNotIn("ttlMs", legacy)

    def test_manifest_staleness_names_changed_files(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo = Path(tmp)
            (repo / "graphify-out").mkdir()
            (repo / "a.py").write_text("x = 1\n")
            (repo / "b.py").write_text("y = 2\n")
            manifest = {"a.py": {"mtime": (repo / "a.py").stat().st_mtime},
                        "b.py": {"mtime": (repo / "b.py").stat().st_mtime}}
            (repo / "graphify-out" / "manifest.json").write_text(json.dumps(manifest))
            self.assertEqual(hive.changed_since_build(repo), [])
            os.utime(repo / "b.py", (time.time() + 9, time.time() + 9))
            self.assertEqual(hive.changed_since_build(repo), ["b.py"])
            self.assertIsNone(hive.changed_since_build(repo / "nope"))


class StdioTest(unittest.TestCase):
    def test_serve_round_trip_over_stdio(self):
        import subprocess
        import sys
        with tempfile.TemporaryDirectory() as tmp:
            write(Path(tmp) / "v", "projects/demo/_memory.md", type="project-memory", status="active",
                  title="Demo memory", body="Uses MandiPrices.")
            env = {**os.environ, "HIVE_VAULT": str(Path(tmp) / "v"), "HIVE_DB": str(Path(tmp) / "h.sqlite"),
                   "HIVE_PROJECTS": tmp}
            msgs = [{"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-11-25"}},
                    {"jsonrpc": "2.0", "method": "notifications/initialized"},
                    {"jsonrpc": "2.0", "id": 2, "method": "tools/call",
                     "params": {"name": "hive_context", "arguments": {"project": "demo"}}}]
            proc = subprocess.run([sys.executable, "-I", str(Path(hive.__file__)), "serve"], env=env, timeout=30,
                                  input="\n".join(json.dumps(m) for m in msgs) + "\n", capture_output=True, text=True,
                                  encoding="utf-8")
            replies = [json.loads(line) for line in proc.stdout.splitlines()]
            self.assertEqual([r["id"] for r in replies], [1, 2])
            self.assertIn("Uses MandiPrices", replies[1]["result"]["content"][0]["text"])


if __name__ == "__main__":
    unittest.main()
