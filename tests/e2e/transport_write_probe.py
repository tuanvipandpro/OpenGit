"""Owned HTTPS/SSH receive-pack fixture proof, not application push acceptance.

HTTP credentials exist only in fixture memory/child environment. The existing
SSH helper owns an encrypted-key file and a transient agent-loading key, removed
by its guarded cleanup. Never print Git output or credentials. No app command/
response or global trust is replaced.
Small known-length packs only; large/chunked HTTP support is not claimed.
"""
import base64
import os
from pathlib import Path
import subprocess
import sys
from urllib.parse import urlsplit

from desktop_smoke import process_environment

ROOT = Path(__file__).resolve().parents[2]
sys.path[:0] = [str(ROOT / '.tools/fixture-python'), str(ROOT / 'tests/e2e/support')]
from live_remotes import LiveRemotes


def client(fixture, env, *args):
    assert fixture.source.resolve().is_relative_to(fixture.root)
    return subprocess.run([fixture.git, '-C', str(fixture.source), *args],
                          env=process_environment(env), capture_output=True,
                          timeout=30, creationflags=0x08000000 if os.name == 'nt' else 0)


def base_environment(fixture):
    env = {key: value for key, value in fixture.env.items()
           if not key.upper().startswith('GIT_TRACE') and key.upper() != 'GIT_CURL_VERBOSE'}
    return {**env, 'GIT_SSL_NO_VERIFY': None, 'GIT_CONFIG_PARAMETERS': None,
            'GIT_ASKPASS': None, 'SSH_ASKPASS': None,
            'GIT_TERMINAL_PROMPT': '0', 'GCM_INTERACTIVE': 'never'}


def ref_value(fixture, name):
    result = fixture._git('-C', str(fixture.repository), 'for-each-ref',
                          '--format=%(objectname)', name)
    return result.stdout.strip()


def new_commit(fixture, transport):
    name = f'push-{transport}.txt'
    (fixture.source / name).write_text(f'Owned new {transport} object transfer\n', encoding='utf-8')
    fixture._git('-C', str(fixture.source), 'add', '--', name)
    fixture._git('-C', str(fixture.source), '-c', 'user.name=OpenGit Fixture',
                 '-c', 'user.email=fixture@invalid.example', 'commit', '-m', 'Owned transport probe')
    return fixture._git('-C', str(fixture.source), 'rev-parse', 'HEAD').stdout.strip()


def prove(writable):
    with LiveRemotes(ROOT, writable=writable) as fixture:
        secret_header = 'Authorization: Basic ' + base64.b64encode(
            (fixture.username + ':' + fixture.password).encode()).decode()
        https = {**base_environment(fixture), 'GIT_SSL_CAINFO': str(fixture.ca_file),
                 'GIT_CONFIG_COUNT': '3', 'GIT_CONFIG_KEY_0': 'credential.helper',
                 'GIT_CONFIG_VALUE_0': '', 'GIT_CONFIG_KEY_1': 'http.extraHeader',
                 'GIT_CONFIG_VALUE_1': secret_header, 'GIT_CONFIG_KEY_2': 'http.sslBackend',
                 'GIT_CONFIG_VALUE_2': 'openssl'}
        expected = new_commit(fixture, 'https')
        assert client(fixture, https, 'ls-remote', '--heads', '--', fixture.https_url).returncode == 0
        target = 'refs/heads/https-write-probe'
        denied_auth = client(fixture, {**https, 'GIT_CONFIG_VALUE_1': ''},
                             'push', '--porcelain', '--', fixture.https_url, 'HEAD:' + target)
        assert denied_auth.returncode != 0 and not ref_value(fixture, target)
        denied_tls = client(fixture, {**https, 'GIT_SSL_CAINFO': None},
                            'push', '--porcelain', '--', fixture.https_url, 'HEAD:' + target)
        assert denied_tls.returncode != 0 and not ref_value(fixture, target)
        pushed = client(fixture, https, 'push', '--porcelain', '--', fixture.https_url, 'HEAD:' + target)
        assert (pushed.returncode == 0) == writable
        assert ref_value(fixture, target) == (expected if writable else b'')

        ssh, agent_env = fixture.start_agent()
        port = urlsplit(fixture.ssh_url).port
        public = fixture.server_public_key.export_public_key('openssh').decode().strip()
        fixture.known_hosts.write_text(f'[127.0.0.1]:{port} {public}\n', encoding='ascii')
        ssh_env = {**base_environment(fixture), 'SSH_AUTH_SOCK': agent_env['SSH_AUTH_SOCK'],
                   'GIT_CONFIG_COUNT': '0', 'GIT_SSH_VARIANT': 'ssh',
                   'GIT_SSH_COMMAND': f'"{ssh.as_posix()}" -F /dev/null -oBatchMode=yes '
                       f'-oStrictHostKeyChecking=yes -oUserKnownHostsFile="{fixture.known_hosts.as_posix()}" '
                       '-oGlobalKnownHostsFile=/dev/null'}
        assert client(fixture, ssh_env, 'ls-remote', '--heads', '--', fixture.ssh_url).returncode == 0
        expected = new_commit(fixture, 'ssh')
        target = 'refs/heads/ssh-write-probe'
        pushed = client(fixture, ssh_env, 'push', '--porcelain', '--', fixture.ssh_url, 'HEAD:' + target)
        assert (pushed.returncode == 0) == writable
        assert ref_value(fixture, target) == (expected if writable else b'')
        assert fixture._agent.poll() is None
        print('PASS: owned authenticated HTTPS/SSH receive-pack ' +
              ('enabled explicitly' if writable else 'denied by default') +
              '; TLS/auth negative controls and exact remote refs verified', flush=True)


if __name__ == '__main__':
    prove(False)
    prove(True)
