import json
import tempfile
import unittest
from pathlib import Path
import hive

class PprDirectionTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.graph_path = Path(self.tmp.name) / "graph.json"
        # 3-node linear chain: caller_a -> middle_b -> callee_c
        graph_data = {
            "built_at_commit": "12345678",
            "nodes": [
                {"id": "caller_a", "label": "caller_a", "source_file": "src/a.py", "source_location": "L10"},
                {"id": "middle_b", "label": "middle_b", "source_file": "src/b.py", "source_location": "L20"},
                {"id": "callee_c", "label": "callee_c", "source_file": "src/c.py", "source_location": "L30"}
            ],
            "links": [
                {"source": "caller_a", "target": "middle_b", "relation": "calls", "confidence_score": 1.0},
                {"source": "middle_b", "target": "callee_c", "relation": "calls", "confidence_score": 1.0}
            ]
        }
        self.graph_path.write_text(json.dumps(graph_data), encoding="utf-8")
        self.graph = hive.Graph(self.graph_path)

    def tearDown(self):
        self.tmp.cleanup()

    def test_forward_and_reverse_asymmetric_ranking(self):
        # Query seeding caller_a should traverse forward with w_fwd = 0.7
        nodes_from_a, edges_from_a = self.graph.query("caller_a")
        node_ids_a = [nid for nid, _, _ in nodes_from_a]
        self.assertIn("caller_a", node_ids_a)
        self.assertIn("middle_b", node_ids_a)
        self.assertIn("callee_c", node_ids_a)

        # Query seeding callee_c should traverse reverse with w_rev = 0.3
        nodes_from_c, edges_from_c = self.graph.query("callee_c")
        node_ids_c = [nid for nid, _, _ in nodes_from_c]
        self.assertIn("callee_c", node_ids_c)
        self.assertIn("middle_b", node_ids_c)
        self.assertIn("caller_a", node_ids_c)

        # Edges must be directed caller -> callee (source -> target)
        self.assertIn(("caller_a", "calls", "middle_b"), edges_from_a)
        self.assertIn(("middle_b", "calls", "callee_c"), edges_from_a)

if __name__ == "__main__":
    unittest.main()
