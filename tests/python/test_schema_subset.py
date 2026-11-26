import unittest
import hive

class SchemaSubsetTest(unittest.TestCase):
    def test_missing_required_field(self):
        errs = hive.validate_tool_args("hive_context", {})
        self.assertTrue(any(e["field"] == "project" and e["expected"] == "provided" for e in errs))

    def test_unexpected_keys_rejected(self):
        errs = hive.validate_tool_args("hive_context", {"project": "alpha", "unrecognized_flag": True})
        self.assertTrue(any(e["field"] == "unrecognized_flag" and e["expected"] == "none" for e in errs))

    def test_type_mismatches(self):
        # Integer expected, got string
        errs = hive.validate_tool_args("hive_context", {"project": "alpha", "budget": "not-an-int"})
        self.assertTrue(any(e["field"] == "budget" and e["expected"] == "integer" for e in errs))

        # String expected, got int
        errs2 = hive.validate_tool_args("hive_context", {"project": 12345})
        self.assertTrue(any(e["field"] == "project" and e["expected"] == "string" for e in errs2))

    def test_structured_error_in_handle(self):
        res = hive.handle({
            "jsonrpc": "2.0",
            "id": 101,
            "method": "tools/call",
            "params": {
                "name": "hive_context",
                "arguments": {"project": 999, "unknown_field": "bad"}
            }
        })
        self.assertIsNotNone(res)
        result = res["result"]
        self.assertTrue(result["isError"])
        self.assertIn("errorDetails", result)
        details = result["errorDetails"]
        self.assertTrue(any(d["field"] == "unknown_field" for d in details))
        self.assertTrue(any(d["field"] == "project" and d["got"] == "int" for d in details))

if __name__ == "__main__":
    unittest.main()
