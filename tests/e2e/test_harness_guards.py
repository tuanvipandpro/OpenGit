"""Negative controls for acceptance assertions; no native app required."""
import unittest
from types import SimpleNamespace

from clone_failures import assert_no_restart_requests


class RestartRequestGuardTests(unittest.TestCase):
    def test_request_before_first_reopened_assertion_is_rejected(self):
        fixture = SimpleNamespace(https_requests=7)
        baseline = fixture.https_requests
        fixture.https_requests += 1  # Simulate one startup request before IPC.
        with self.assertRaisesRegex(AssertionError, 'Restart unexpectedly'):
            assert_no_restart_requests(fixture, baseline)

    def test_unchanged_request_count_is_accepted(self):
        assert_no_restart_requests(SimpleNamespace(https_requests=7), 7)


if __name__ == '__main__':
    unittest.main()
