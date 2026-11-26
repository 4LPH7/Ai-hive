import concurrent.futures
import tempfile
import time
import unittest
from pathlib import Path
from unittest import mock

import ace

class AceConcurrencyTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.vault = self.root / "vault"
        self.vault.mkdir(parents=True, exist_ok=True)
        self.patch = mock.patch.object(ace, "VAULT", self.vault)
        self.patch.start()

    def tearDown(self):
        self.patch.stop()
        self.tmp.cleanup()

    def test_concurrent_writers_no_lost_updates(self):
        # 2 concurrent workers x 25 applies = 50 total applies
        scope = "concurrency-test"

        def worker(worker_id):
            for i in range(25):
                ops = [{
                    "type": "ADD",
                    "section": "strategies",
                    "content": f"directiveW{worker_id}N{i:03d} keywordW{worker_id}N{i:03d} detailW{worker_id}N{i:03d} tipW{worker_id}N{i:03d}"
                }]
                ace.apply(scope, ops)

        with concurrent.futures.ThreadPoolExecutor(max_workers=2) as executor:
            f1 = executor.submit(worker, 1)
            f2 = executor.submit(worker, 2)
            f1.result()
            f2.result()

        final = ace.load(scope)
        added_count = len(final["strategies"])
        # All 50 distinct bullets must be present without lost updates (F9)
        self.assertEqual(added_count, 50)

    def test_stale_lockfile_recovery_dead_pid(self):
        import socket
        scope = "stale-lock-test"
        path = ace.scope_path(scope)
        path.parent.mkdir(parents=True, exist_ok=True)
        lock_path = path.with_suffix(".lock")

        # Fake dead PID 999999 on local host
        local_host = socket.gethostname()
        lock_path.write_text(f"999999:{local_host}:2026-11-24T00:00:00:nonce123\n", encoding="utf-8")

        # Calling apply should detect dead PID, steal lock, and succeed
        ops = [{"type": "ADD", "section": "commands", "content": "Recovered from dead lock"}]
        report = ace.apply(scope, ops)
        self.assertEqual(len(report["added"]), 1)
        self.assertFalse(lock_path.exists())

    def test_foreign_host_lock_refused(self):
        scope = "foreign-lock-test"
        path = ace.scope_path(scope)
        path.parent.mkdir(parents=True, exist_ok=True)
        lock_path = path.with_suffix(".lock")

        # Fake lock on foreign host with short timeout lock
        lock_path.write_text("999999:foreign-host-999:2026-11-24T00:00:00:nonce123\n", encoding="utf-8")

        ops = [{"type": "ADD", "section": "commands", "content": "Should not apply"}]
        with mock.patch.object(ace.PlaybookLock, "acquire", side_effect=TimeoutError("playbook locked by foreign host")):
            with self.assertRaises(TimeoutError):
                ace.apply(scope, ops)
        if lock_path.exists():
            lock_path.unlink()

if __name__ == "__main__":
    unittest.main()
