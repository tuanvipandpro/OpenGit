"""Negative controls for acceptance assertions; no native app required."""
import unittest
import os
from unittest.mock import patch
from types import SimpleNamespace

from clone_failures import assert_no_restart_requests
from desktop_smoke import process_environment


class RestartRequestGuardTests(unittest.TestCase):
    def test_request_before_first_reopened_assertion_is_rejected(self):
        fixture = SimpleNamespace(https_requests=7)
        baseline = fixture.https_requests
        fixture.https_requests += 1  # Simulate one startup request before IPC.
        with self.assertRaisesRegex(AssertionError, 'Restart unexpectedly'):
            assert_no_restart_requests(fixture, baseline)

    def test_unchanged_request_count_is_accepted(self):
        assert_no_restart_requests(SimpleNamespace(https_requests=7), 7)


@unittest.skipUnless(os.name == 'nt', 'Windows environment-name semantics')
class EnvironmentGuardTests(unittest.TestCase):
    def test_case_insensitive_override_has_one_value(self):
        with patch.dict(os.environ, {'PROGRAMFILES': 'original'}, clear=True):
            env = process_environment({'ProgramFiles': 'owned fixture'})
        self.assertEqual(env, {'PROGRAMFILES': 'owned fixture'})

    def test_case_insensitive_none_really_removes_inherited_switch(self):
        with patch.dict(os.environ, {'GIT_SSL_NO_VERIFY': 'false'}, clear=True):
            env = process_environment({'git_ssl_no_verify': None})
        self.assertNotIn('GIT_SSL_NO_VERIFY', env)


if __name__ == '__main__':
    unittest.main()
