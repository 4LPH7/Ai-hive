"""MCP-01/02 (Phase 9 gate): at most 6 registered tools, and the whole tool schema stays under a fixed token
budget so tool descriptions cannot creep into context confusion. Mirror of mcp.rs test_schema_budget.

Measured 1,037 est tokens at v2.7.2; the cap is that +10%. Raising it is a deliberate, reviewed change."""
import json
import unittest

import hive

MAX_TOOLS = 6
SCHEMA_TOKEN_CAP = 1140


class SchemaBudgetTest(unittest.TestCase):
    def test_tool_count_and_schema_size_stay_under_budget(self):
        tools = hive.tool_list()
        self.assertLessEqual(len(tools), MAX_TOOLS, [t["name"] for t in tools])
        size = hive.est_tokens(json.dumps(tools, separators=(",", ":"), sort_keys=True))
        self.assertLessEqual(size, SCHEMA_TOKEN_CAP,
                             f"MCP tool schema is ~{size} est tokens (cap {SCHEMA_TOKEN_CAP}); trim descriptions "
                             "or move options onto existing tools before raising the cap")


if __name__ == "__main__":
    unittest.main()
