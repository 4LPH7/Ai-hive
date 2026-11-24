import unittest
import hive

class ProtocolVersionsTest(unittest.TestCase):
    def test_all_supported_versions_handshake_and_tool_call(self):
        for ver in hive.ALL_SUPPORTED_VERSIONS:
            # 1. initialize / handshake
            init_req = {
                "jsonrpc": "2.0",
                "id": f"init-{ver}",
                "method": "initialize",
                "params": {"protocolVersion": ver}
            }
            init_resp = hive.handle(init_req)
            self.assertIsNotNone(init_resp, f"handle returned None for version {ver}")
            self.assertNotIn("error", init_resp, f"initialize failed for version {ver}: {init_resp.get('error')}")
            res = init_resp["result"]
            self.assertEqual(res["protocolVersion"], ver)

            # 2. tools/list under negotiated version
            list_req = {
                "jsonrpc": "2.0",
                "id": f"list-{ver}",
                "method": "tools/list",
                "params": {"_meta": {hive.META_VERSION: ver}} if ver in hive.MODERN_VERSIONS else {}
            }
            list_resp = hive.handle(list_req)
            self.assertIsNotNone(list_resp)
            self.assertNotIn("error", list_resp)
            tools = list_resp["result"]["tools"]
            tool_names = {t["name"] for t in tools}
            self.assertIn("hive_context", tool_names)
            self.assertIn("hive_search", tool_names)

            # 3. tools/call under negotiated version
            call_req = {
                "jsonrpc": "2.0",
                "id": f"call-{ver}",
                "method": "tools/call",
                "params": {
                    "name": "hive_search",
                    "arguments": {"query": "test", "scope": "all"},
                    **({"_meta": {hive.META_VERSION: ver}} if ver in hive.MODERN_VERSIONS else {})
                }
            }
            call_resp = hive.handle(call_req)
            self.assertIsNotNone(call_resp)
            self.assertNotIn("error", call_resp)
            content = call_resp["result"]["content"][0]["text"]
            self.assertIsInstance(content, str)

    def test_unsupported_version_returns_structured_error(self):
        req = {
            "jsonrpc": "2.0",
            "id": 99,
            "method": "tools/list",
            "params": {"_meta": {hive.META_VERSION: "1999-01-01"}}
        }
        resp = hive.handle(req)
        self.assertIsNotNone(resp)
        self.assertIn("error", resp)
        self.assertEqual(resp["error"]["code"], -32022)
        self.assertIn("supported", resp["error"]["data"])

if __name__ == "__main__":
    unittest.main()
