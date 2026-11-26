"""DETAIL-01/02 (v2.7.1): concise MCP responses by default, detailed bytes unchanged, large output offloaded.

Run `python -m tests.python.test_detail --snapshot` only to re-record tests/fixtures/detail_snapshots.json (detailed bytes).
"""
import json
import os
import sys
import tempfile
import time
import unittest
from pathlib import Path
from unittest import mock

import hive

SNAPSHOTS = Path(__file__).parents[1] / "fixtures" / "detail_snapshots.json"
NOTE = ("---\ntype: {type}\nproject: demo\nagent: claude\nstatus: active\nupdated: 2026-11-2{d}\n"
        "source: test fixture\n---\n# {title}\n\n{body}\n")


def build(root):
    """Hermetic vault + projects root: 12 notes, one repo graph, a few lint findings."""
    root = Path(root)
    vault, projects = root / "vault", root / "projects"
    # BM25 scores must be distinct: query terms sit in under half the notes (IDF > 0) and "rows" repeats i+1
    # times, so no two hits tie. Tied hits come back in SQLite row order, which follows the OS directory walk.
    for i in range(12):
        kind = "adr" if i % 3 == 0 else "note"
        path = vault / "projects" / "demo" / ("adrs" if kind == "adr" else "notes") / f"price-fetch-{i:02d}.md"
        path.parent.mkdir(parents=True, exist_ok=True)
        body = (f"The price fetcher step {i} parses the upstream feed and stores it in the cache. "
                "Retries use exponential backoff; the parser rejects malformed input. " + "rows " * (i + 1))
        path.write_text(NOTE.format(type=kind, d=i % 9, title=f"Price fetcher decision {i}", body=body),
                        encoding="utf-8")
    for i in range(16):
        path = vault / "projects" / "demo" / "notes" / f"deploy-{i:02d}.md"
        path.write_text(NOTE.format(type="note", d=i % 9, title=f"Deploy pipeline step {i}",
                                    body=f"Canary rollout {i} waits for health checks before promotion."),
                        encoding="utf-8")
    (vault / "notes").mkdir(parents=True, exist_ok=True)
    (vault / "notes" / "Bad Name.md").write_text("no frontmatter [[missing-link]]\n", encoding="utf-8")
    nodes = [{"id": i, "label": label, "source_file": f"src/{i}.py", "source_location": "L1",
              "community_name": "pricing", "file_type": "code"}
             for i, label in [("a", "PriceFetcher"), ("b", "parse_rows"), ("c", "store_rows"),
                              ("d", "price_cache"), ("hub", "utils")]]
    # distinct labels and edge weights: graph ranking breaks score ties by set order, so a fixture must have none
    nodes += [{"id": f"x{k}", "label": f"leaf_{chr(97 + k)}", "source_file": f"src/x{k}.py", "source_location": "L9",
               "community_name": "leaves"} for k in range(14)]
    links = [{"source": "a", "target": "b", "relation": "calls", "confidence_score": 1.0},
             {"source": "b", "target": "c", "relation": "calls", "confidence_score": 1.0},
             {"source": "a", "target": "d", "relation": "uses", "confidence_score": 1.0},
             {"source": "a", "target": "hub", "relation": "imports", "confidence_score": 1.0}]
    links += [{"source": "hub", "target": f"x{k}", "relation": "contains", "confidence_score": 1.0 - k * 0.05}
              for k in range(14)]
    graph = projects / "demo" / "graphify-out" / "graph.json"
    graph.parent.mkdir(parents=True)
    graph.write_text(json.dumps({"nodes": nodes, "links": links}), encoding="utf-8")
    return vault, projects


class Fixture:
    def __enter__(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.vault, self.projects = build(self.tmp.name)
        self.idx = hive.Index(self.vault, Path(self.tmp.name) / "hive.sqlite")
        self.patches = [mock.patch.object(hive, "VAULT", self.vault), mock.patch.object(hive, "PROJECTS", self.projects),
                        mock.patch.object(hive, "_INDEX", self.idx),
                        mock.patch.object(hive, "graph_freshness", lambda d, g: "freshness fixed"),
                        mock.patch.object(hive, "LEDGER_PATH", Path(self.tmp.name) / "ledger.jsonl"),
                        mock.patch.dict(os.environ, {"HIVE_ARTIFACTS": str(Path(self.tmp.name) / "artifacts")})]
        for p in self.patches:
            p.start()
        return self

    def __exit__(self, *exc):
        for p in self.patches:
            p.stop()
        self.idx.db.close()
        self.tmp.cleanup()


CALLS = (("hive_search", {"query": "price fetcher rows", "project": "demo"}),
         ("hive_graph", {"repo": "demo", "query": "PriceFetcher price"}),
         ("hive_lint", {}))


def outputs(detail):
    out = {}
    for name, args in CALLS:
        fn = hive.TOOLS[name][0]
        out[name] = fn(**args, **({"detail": detail} if detail else {}))
    return out


class DetailModeTest(unittest.TestCase):
    def test_detailed_bytes_match_snapshot(self):
        with Fixture():
            want = json.loads(SNAPSHOTS.read_text(encoding="utf-8"))
            self.assertEqual(outputs(None), want)
            self.assertEqual(outputs("detailed"), want)

    def test_concise_shapes(self):
        with Fixture():
            out = outputs("concise")
        for line in out["hive_search"].splitlines():
            self.assertRegex(line, r"^- projects/demo/\S+\.md \((adr|note)\) Price fetcher decision \d+$")
        self.assertNotIn("edges among these", out["hive_graph"])
        self.assertNotIn("vault notes about", out["hive_graph"])
        self.assertNotIn("[pricing]", out["hive_graph"])
        self.assertIn("* PriceFetcher  src/a.py:1", out["hive_graph"])
        self.assertTrue(out["hive_lint"].startswith("lint: "))
        self.assertIn('detail="detailed" lists every finding', out["hive_lint"])

    def test_gate_concise_halves_mean_tokens(self):
        """Phase 7 addendum gate: concise cuts mean MCP response tokens by >= 50%."""
        with Fixture():
            detailed, concise = outputs("detailed"), outputs("concise")
        d = sum(hive.est_tokens(t) for t in detailed.values())
        c = sum(hive.est_tokens(t) for t in concise.values())
        self.assertLessEqual(c / d, 0.50, f"concise {c} / detailed {d}")

    def test_mcp_defaults_to_concise_and_validates_enum(self):
        with Fixture():
            def call(name, args):
                return hive.handle({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                                    "params": {"name": name, "arguments": args}})["result"]
            res = call("hive_search", {"query": "price fetcher rows", "project": "demo"})
            self.assertEqual(res["content"][0]["text"], outputs("concise")["hive_search"])
            self.assertIn("\n  ", call("hive_search", {"query": "price fetcher rows", "project": "demo",
                                                         "detail": "detailed"})["content"][0]["text"])
            bad = call("hive_lint", {"detail": "full"})
            self.assertTrue(bad["isError"])
            self.assertTrue(call("hive_status", {"detail": "concise"})["isError"])
        tools = {t["name"]: t for t in hive.tool_list()}
        self.assertEqual(len(tools), 6)
        for name in ("hive_search", "hive_graph", "hive_lint"):
            self.assertEqual(tools[name]["inputSchema"]["properties"]["detail"]["enum"], ["concise", "detailed"])


class OffloadTest(unittest.TestCase):
    def test_excerpt_keeps_buried_signal_lines(self):
        """Signal lines survive offload wherever they sit; bytes pinned = src/artifact.rs."""
        import hashlib
        import artifacts
        lines = [f"2026-11-26T10:{i // 60:02d}:{i % 60:02d} INFO worker {i % 7} processed batch {i} ok"
                 for i in range(1000)]
        lines[66] = "2026-11-26T10:01:06 ERROR worker 3 retry budget exhausted for batch 66"
        lines[899] = "2026-11-26T10:14:59 FATAL worker 1 panicked: index out of range in shard 899"
        out = artifacts.excerpt("\n".join(lines), 400)
        self.assertLessEqual(hive.est_tokens(out), 400)
        self.assertTrue(out.startswith(lines[0]) and out.endswith(lines[-1]))
        self.assertIn(f"L67: {lines[66]}", out)
        self.assertIn(f"L900: {lines[899]}", out)
        self.assertIn("lines omitted; 2 of 2 signal lines kept", out)
        self.assertEqual(hashlib.sha256(out.encode()).hexdigest(),
                         "882c9d9fec3044f42fe857c63f3361dda3ea11664cd3df8aa073a5edc3ed6397")
        self.assertEqual(artifacts.excerpt("short\ntext", 400), "short\ntext")

    def test_small_text_unchanged_large_text_offloaded(self):
        import artifacts
        with Fixture() as fx:
            self.assertEqual(artifacts.offload("short"), "short")
            big = ("row parse store cache retry backoff " * 40 + "\n") * 40 + "token: abcdefghijklmnopqrstu\n"
            out = artifacts.offload(big)
            self.assertLess(hive.est_tokens(out), 520)
            self.assertIn("[full output offloaded: ", out)
            stored = next(Path(os.environ["HIVE_ARTIFACTS"]).glob("*.md"))
            text = stored.read_text(encoding="utf-8")
            self.assertIn("[REDACTED_SECRET]", text)
            self.assertNotIn("abcdefghijklmnopqrstu", text)
            self.assertIn(str(stored.resolve()), out)
            old = Path(os.environ["HIVE_ARTIFACTS"]) / ("b" * 64 + ".md")
            old.write_text("old", encoding="utf-8")
            os.utime(old, (time.time() - 8 * 86400,) * 2)
            keep = Path(os.environ["HIVE_ARTIFACTS"]) / "notes.md"
            keep.write_text("not ours", encoding="utf-8")
            os.utime(keep, (time.time() - 8 * 86400,) * 2)
            artifacts.offload(big + "x")  # a new write purges
            self.assertFalse(old.exists())
            self.assertTrue(keep.exists())
            with mock.patch.dict(os.environ, {"HIVE_ARTIFACTS": str(fx.vault / "cache")}):
                self.assertEqual(artifacts.offload(big), big, "a cache inside the vault is refused")
            self.assertFalse((fx.vault / "cache").exists(), "the refused cache dir must not be created")

    def test_mcp_offloads_large_results_but_not_context_or_errors(self):
        with Fixture():
            big = "x " * 9000
            with mock.patch.dict(hive.TOOLS, {"hive_status": (lambda: big, *hive.TOOLS["hive_status"][1:])}):
                res = hive.handle({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                                   "params": {"name": "hive_status", "arguments": {}}})["result"]
            self.assertIn("[full output offloaded: ", res["content"][0]["text"])
            ctx = hive.TOOLS["hive_context"]
            with mock.patch.dict(hive.TOOLS, {"hive_context": (lambda **kw: big, *ctx[1:])}):
                res = hive.handle({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                                   "params": {"name": "hive_context", "arguments": {"project": "demo"}}})["result"]
            self.assertEqual(res["content"][0]["text"], big)

            def boom():
                raise RuntimeError(big)
            with mock.patch.dict(hive.TOOLS, {"hive_status": (boom, *hive.TOOLS["hive_status"][1:])}):
                res = hive.handle({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                                   "params": {"name": "hive_status", "arguments": {}}})["result"]
            self.assertTrue(res["isError"])
            self.assertNotIn("[full output offloaded", res["content"][0]["text"])


if __name__ == "__main__":
    if sys.argv[1:] == ["--snapshot"]:
        with Fixture():
            SNAPSHOTS.parent.mkdir(exist_ok=True)
            SNAPSHOTS.write_text(json.dumps(outputs(None), indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
        print(f"wrote {SNAPSHOTS}")
    else:
        unittest.main()
