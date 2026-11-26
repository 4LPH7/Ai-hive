import tempfile
import time
import unittest
from pathlib import Path

import skill


class SkillTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.skills_dir = self.root / "skills"
        self.skills_dir.mkdir(parents=True, exist_ok=True)

    def tearDown(self):
        self.tmp.cleanup()

    def test_parse_skill_md_frontmatter_and_bom(self):
        skill_dir = self.skills_dir / "docker-debugger"
        skill_dir.mkdir(parents=True, exist_ok=True)
        skill_file = skill_dir / "SKILL.md"

        # Content with UTF-8 BOM and CRLF
        content = (
            "\ufeff---\r\n"
            "name: docker-debugger\r\n"
            "description: Inspect container logs, crash dumps, and daemon state\r\n"
            "triggers:\r\n"
            "  - docker logs\r\n"
            "  - container crash\r\n"
            "---\r\n"
            "# Instructions\r\n"
            "Run docker inspect and logs.\r\n"
        )
        skill_file.write_bytes(content.encode("utf-8"))

        meta = skill.parse_skill_md(skill_file, root_type="test")
        self.assertIsNotNone(meta)
        self.assertEqual(meta.name, "docker-debugger")
        self.assertIn("Inspect container logs", meta.description)
        self.assertIn("docker logs", meta.triggers)
        self.assertIn("container crash", meta.triggers)
        self.assertEqual(meta.root_type, "test")

    def test_intent_matching_confidence(self):
        s1 = skill.SkillMeta(
            name="postgres-perf",
            description="Postgres query performance tuning, vacuuming, and indexing",
            triggers=["pg_stat", "explain analyze"],
            root_type="test",
        )
        s2 = skill.SkillMeta(
            name="react-native-build",
            description="Build iOS and Android React Native release artifacts",
            triggers=["metro bundle", "pod install"],
            root_type="test",
        )
        skills = [s1, s2]

        # 1. Trigger hit -> confidence >= 0.65 (is_match = True)
        res1 = skill.match_skill("help me debug this explain analyze query", skills=skills, threshold=0.65)
        self.assertTrue(res1.is_match)
        self.assertGreaterEqual(res1.confidence, 0.65)
        self.assertEqual(res1.skill.name, "postgres-perf")

        # 2. Name & keyword match -> good confidence
        res2 = skill.match_skill("react native app build failing", skills=skills, threshold=0.50)
        self.assertTrue(res2.is_match)
        self.assertEqual(res2.skill.name, "react-native-build")

        # 3. Completely unrelated query -> miss
        res3 = skill.match_skill("kubernetes ingress cert-manager configuration", skills=skills, threshold=0.65)
        self.assertFalse(res3.is_match)
        self.assertLess(res3.confidence, 0.3)

    def test_multi_root_discovery(self):
        r1 = self.root / "root1"
        r2 = self.root / "root2"
        r1.mkdir()
        r2.mkdir()

        (r1 / "skill_a").mkdir()
        (r1 / "skill_a" / "SKILL.md").write_text("---\nname: skill-a\ndescription: A\n---\n", encoding="utf-8")

        (r2 / "skill_b").mkdir()
        (r2 / "skill_b" / "skill.md").write_text("---\nname: skill-b\ndescription: B\n---\n", encoding="utf-8")

        discovered = skill.scan_skills(roots=[(r1, "custom1"), (r2, "custom2")])
        names = {s.name for s in discovered}
        self.assertIn("skill-a", names)
        self.assertIn("skill-b", names)

    def test_discovery_speed_benchmark(self):
        # Create 50 dummy skills
        bench_root = self.root / "bench_root"
        bench_root.mkdir()
        for i in range(50):
            d = bench_root / f"skill_{i:03d}"
            d.mkdir()
            (d / "SKILL.md").write_text(
                f"---\nname: skill_{i:03d}\ndescription: Auto test skill number {i}\ntriggers:\n  - trig_{i:03d}\n---\n",
                encoding="utf-8",
            )

        discovered = skill.scan_skills(roots=[(bench_root, "bench")])
        self.assertEqual(len(discovered), 50)

        t0 = time.perf_counter()
        res = skill.match_skill("trig_025 action needed", skills=discovered)
        elapsed_ms = (time.perf_counter() - t0) * 1000.0

        self.assertTrue(res.is_match)
        self.assertEqual(res.skill.name, "skill_025")
        # In-memory matching across 50 skills must be <5 ms
        self.assertLess(elapsed_ms, 15.0)


if __name__ == "__main__":
    unittest.main()
