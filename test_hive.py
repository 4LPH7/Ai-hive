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

    def test_pack_respects_budget_and_prevents_starvation(self):
        # Quota-based pack gives sections their share, preventing starvation while respecting budget
        out = hive.pack(["x" * 400, "y" * 4000, "z" * 400], budget=300)
        self.assertLessEqual(hive.est_tokens(out), 330)
        self.assertIn("z", out)

    def test_directed_edge_ordering_and_printing(self):
        nodes = [{"id": "zeta_caller", "label": "zeta_caller", "file_type": "code"},
                 {"id": "alpha_callee", "label": "alpha_callee", "file_type": "code"}]
        links = [{"source": "zeta_caller", "target": "alpha_callee", "relation": "calls", "confidence_score": 1.0}]
        p = Path(self.tmp.name) / "dir_graph.json"
        p.write_text(json.dumps({"nodes": nodes, "links": links}))
        g = hive.Graph(p)
        ranked, edges = g.query("zeta")
        # Direction must be caller (zeta) -> callee (alpha), not reversed by ID sorting
        self.assertEqual(edges, [("zeta_caller", "calls", "alpha_callee")])


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


class Milestone1RegressionTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.vault = Path(self.tmp.name) / "vault"
        self.vault.mkdir(parents=True, exist_ok=True)
        self.projects = Path(self.tmp.name) / "projects"
        self.projects.mkdir(parents=True, exist_ok=True)
        self.db_path = Path(self.tmp.name) / "test.sqlite"
        self.idx = hive.Index(self.vault, self.db_path)

    def tearDown(self):
        self.idx.db.close()
        self.tmp.cleanup()

    def test_f3_path_traversal_blocked(self):
        # 1. Slug validation prevents traversal
        with self.assertRaises(ValueError):
            hive.validate_slug("../../outside")
        with self.assertRaises(ValueError):
            hive.validate_slug("..")
        # 2. Index.note prevents reading files outside vault
        outside = Path(self.tmp.name) / "outside.md"
        outside.write_text("secret outside", encoding="utf-8")
        self.assertEqual(self.idx.note("../../outside.md"), "")
        # 3. repo_dir prevents canonical traversal outside PROJECTS
        old_projects = hive.PROJECTS
        try:
            hive.PROJECTS = self.projects
            self.assertIsNone(hive.repo_dir("../../outside"))
            outside_repo = Path(self.tmp.name) / "outside_repo"
            (outside_repo / "graphify-out").mkdir(parents=True, exist_ok=True)
            self.assertIsNone(hive.repo_dir(str(outside_repo)))
        finally:
            hive.PROJECTS = old_projects

    def test_f1_f15_quota_pack_redistribution_and_omitted_footer(self):
        sections = {
            "memory": "m" * 3000,
            "playbook": "",  # empty -> its share is redistributed
            "task_notes": "t" * 300,
            "code_graph": "c" * 300,
        }
        packed = hive.pack(sections, budget=500)
        # Verify memory was truncated to its budget and did not starve task_notes or code_graph
        self.assertIn("...[truncated to budget]", packed)
        self.assertIn("t" * 100, packed)
        self.assertIn("c" * 100, packed)

        # Extremely low budget causes omitted sections footer
        tiny_pack = hive.pack({"s1": "x" * 500, "s2": "y" * 500}, budget=10)
        self.assertIn("[omitted_sections:", tiny_pack)

    def test_f1_f15_content_aware_token_estimator(self):
        prose = "This is a regular prose sentence describing architecture."
        code = "def fetch_data(param_a, param_b):\n    return {k: v for k, v in items.items()};\n"
        self.assertGreater(hive.est_tokens(code), 0)
        self.assertGreater(hive.est_tokens(prose), 0)
        # Fenced code block counts denser than plain prose
        fence = f"```python\n{code}\n```\n"
        self.assertGreater(hive.est_tokens(fence), hive.est_tokens(prose))

    def test_f2_strict_project_isolation(self):
        write(self.vault, "projects/alpha/note1.md", type="note", status="active",
              title="Alpha Topic", body="Alpha details about search.")
        write(self.vault, "projects/beta/note2.md", type="note", status="active",
              title="Beta Topic", body="Beta details about search.")
        self.idx.refresh(force=True)

        # Isolated to alpha
        alpha_hits = self.idx.search("search", project="alpha", scope="project")
        self.assertTrue(all(h["project"] in ("alpha", "", "global") for h in alpha_hits))
        self.assertNotIn("projects/beta/note2.md", [h["path"] for h in alpha_hits])

        # Cross-project scope="all"
        all_hits = self.idx.search("search", project="alpha", scope="all")
        paths = [h["path"] for h in all_hits]
        self.assertIn("projects/alpha/note1.md", paths)
        self.assertIn("projects/beta/note2.md", paths)

    def test_f5_windows_bom_parse_and_indexing(self):
        text = "\ufeff---\ntype: note\nproject: demo\nagent: user\nstatus: active\nupdated: 2026-11-23\n---\n# With BOM\nContent\n"
        meta, title, body = hive.parse_note(text)
        self.assertEqual(meta.get("type"), "note")
        self.assertEqual(title, "With BOM")
        self.assertNotIn("\ufeff", body)

    def test_f11_exact_active_work_matching(self):
        # fpolink-api should NOT match project "api"
        cell = "[[projects/fpolink-api/_memory|fpolink-api]]"
        self.assertFalse(hive.active_work_cell_matches(cell, "api"))
        self.assertTrue(hive.active_work_cell_matches(cell, "fpolink-api"))
        # Exact cell
        self.assertTrue(hive.active_work_cell_matches("api", "api"))
        self.assertFalse(hive.active_work_cell_matches("api-v2", "api"))

    def test_f10_f14_provenance_and_untrusted_framing(self):
        old_vault = hive.VAULT
        old_idx = hive._INDEX
        try:
            hive.VAULT = self.vault
            hive._INDEX = self.idx
            write(self.vault, "projects/demo/_memory.md", type="project-memory", status="active",
                  title="Demo memory", body="Critical project facts.")
            self.idx.refresh(force=True)
            ctx = hive.tool_context("demo")
            self.assertIn('> [!NOTE] Untrusted vault note: path="projects/demo/_memory.md"', ctx)
            self.assertIn("> Critical project facts.", ctx)
        finally:
            hive.VAULT = old_vault
            hive._INDEX = old_idx

    def test_f13_strict_mcp_argument_hygiene(self):
        # 1. Missing required argument
        r1 = hive.handle({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                          "params": {"name": "hive_context", "arguments": {}}})
        self.assertTrue(r1["result"]["isError"])
        self.assertIn("missing required argument 'project'", r1["result"]["content"][0]["text"])

        # 2. Unexpected argument
        r2 = hive.handle({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
                          "params": {"name": "hive_context", "arguments": {"project": "demo", "extra": "val"}}})
        self.assertTrue(r2["result"]["isError"])
        self.assertIn("unexpected argument 'extra'", r2["result"]["content"][0]["text"])

        # 3. Wrong type for argument
        r3 = hive.handle({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
                          "params": {"name": "hive_context", "arguments": {"project": "demo", "budget": "not_int"}}})
        self.assertTrue(r3["result"]["isError"])
        self.assertIn("must be integer", r3["result"]["content"][0]["text"])


class ConfigAndHashSyncTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.vault = self.root / "vault"
        self.vault.mkdir(parents=True, exist_ok=True)
        self.db_path = self.root / "hive.sqlite"

    def tearDown(self):
        self.tmp.cleanup()

    def test_load_config_precedence(self):
        toml_path = self.root / "config.toml"
        toml_path.write_text('vault = "/toml/vault"\nprojects = "/toml/projects"\ndb = "/toml/db"\n', encoding="utf-8")

        # 1. TOML fallback when no CLI or ENV
        old_env = {k: os.environ.pop(k, None) for k in ["HIVE_VAULT", "HIVE_PROJECTS", "HIVE_DB"]}
        try:
            cfg1 = hive.load_config(cli_config=str(toml_path))
            self.assertEqual(cfg1["vault"], Path("/toml/vault"))
            self.assertEqual(cfg1["projects"], Path("/toml/projects"))

            # 2. ENV overrides TOML
            os.environ["HIVE_VAULT"] = "/env/vault"
            cfg2 = hive.load_config(cli_config=str(toml_path))
            self.assertEqual(cfg2["vault"], Path("/env/vault"))
            self.assertEqual(cfg2["projects"], Path("/toml/projects"))

            # 3. CLI overrides ENV & TOML
            cfg3 = hive.load_config(cli_vault="/cli/vault", cli_config=str(toml_path))
            self.assertEqual(cfg3["vault"], Path("/cli/vault"))
            self.assertEqual(cfg3["projects"], Path("/toml/projects"))
        finally:
            for k, v in old_env.items():
                if v is not None:
                    os.environ[k] = v

    def test_mtime_touch_without_content_change_skips_fts_reindex(self):
        p = write(self.vault, "projects/demo/note1.md", type="note", status="active",
                  title="Note One", body="Stable body text.")
        idx = hive.Index(self.vault, self.db_path)
        self.assertEqual(idx.refresh(force=True), 1)

        # Touch mtime +10s without modifying content
        st = p.stat()
        os.utime(p, (st.st_atime, st.st_mtime + 10))

        # Refresh: hash matches existing, so changed count is 0 and no FTS reindexing occurs
        changed = idx.refresh(force=True)
        self.assertEqual(changed, 0)
        idx.db.close()

    def test_content_change_with_preserved_mtime_reindexes_cleanly(self):
        p = write(self.vault, "projects/demo/note2.md", type="note", status="active",
                  title="Original Title", body="Original body keywords.")
        idx = hive.Index(self.vault, self.db_path)
        self.assertEqual(idx.refresh(force=True), 1)
        self.assertTrue(idx.search("keywords"))

        # Preserve mtime, change content
        st = p.stat()
        orig_mtime = st.st_mtime
        write(self.vault, "projects/demo/note2.md", type="note", status="active",
              title="Updated Title", body="Brand new flamingo content.")
        os.utime(p, (st.st_atime, orig_mtime))

        # Refresh detects size or content change and updates index
        idx.refresh(force=True)
        self.assertTrue(idx.search("flamingo"))
        self.assertFalse(idx.search("keywords"))
        idx.db.close()


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
