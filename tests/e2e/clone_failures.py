"""Native invalid input, cancellation and crash/restart clone acceptance."""
from pathlib import Path
import json
import os
import sys
import time

from desktop_smoke import DesktopSession
from workspace_acceptance import wait_for, workspace
from clone_acceptance import value, assert_no_credentials

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / '.tools/fixture-python'))
sys.path.insert(0, str(ROOT / 'tests/e2e/support'))
from live_remotes import LiveRemotes
from rejecting_proxy import RejectingProxy


def operation(desktop, operation_id):
    return next(o for o in value(desktop.invoke('get_operations', {}))['operations'] if o['id'] == operation_id)


def reject_clone(desktop, request, codes):
    result = desktop.invoke('start_clone', request)
    if result.get('ok'):
        operation_id = result['value']['id']
        wait_for(lambda: operation(desktop, operation_id)['state'] == 'Failed', 'safe clone rejection')
        failure = operation(desktop, operation_id)['error']
    else:
        failure = result.get('error', {})
    assert failure.get('code') in codes


def start_waiting(desktop, fixture, name):
    destination = value(desktop.invoke('choose_clone_destination', {}))
    result = value(desktop.invoke('start_clone', {'source': {'kind': 'remote', 'url': fixture.https_url},
        'destinationHandle': destination['destinationHandle'], 'directoryName': name}))
    wait_for(lambda: operation(desktop, result['id'])['state'] == 'WaitingForUser', 'auth wait', 40)
    return result['id'], operation(desktop, result['id'])['authPrompt']['promptHandle']


def trust_failure(desktop, source, name, code):
    destination = value(desktop.invoke('choose_clone_destination', {}))
    started = value(desktop.invoke('start_clone', {'source': {'kind': 'remote', 'url': source},
        'destinationHandle': destination['destinationHandle'], 'directoryName': name}))
    wait_for(lambda: operation(desktop, started['id'])['state'] == 'Failed', 'trust failure', 45)
    failed = operation(desktop, started['id'])
    assert failed['error']['code'] == code and failed['authPrompt'] is None


def staging_inventory(parent):
    result = {}
    for entry in parent.iterdir():
        if entry.name == 'existing':
            continue
        assert entry.name.startswith('.opengit-stage-'), 'Unexpected destination-parent entry'
        assert entry.is_dir() and not entry.is_symlink()
        assert (entry / '.opengit-stage-owner').is_file(), 'Retained normal fixture lacks ownership evidence'
        result[entry.name] = entry.stat().st_ino
    return result


def assert_retention_notice(snapshot):
    message = (snapshot.get('error') or {}).get('message', '').lower()
    assert any(word in message for word in ('retained', 'kept', 'temporary clone files')), \
        'Retained staging requires a visible durable warning'


def assert_no_restart_requests(fixture, baseline):
    assert fixture.https_requests == baseline, 'Restart unexpectedly retried network requests'


if __name__ == '__main__':
    executable = Path(os.environ.get('OPENGIT_E2E_EXECUTABLE',
                                   str(ROOT / 'src-tauri/target/release/opengit.exe')))
    with LiveRemotes(ROOT) as fixture:
        parent = fixture.root / 'failure-clones'
        parent.mkdir()
        existing = parent / 'existing'
        existing.mkdir()
        sentinel = existing / 'sentinel.txt'
        sentinel.write_bytes(b'preserve-existing-destination')
        data = fixture.root / 'failure-app-data'
        env = {'OPENGIT_DATA_DIR': str(data), 'GIT_SSL_NO_VERIFY': None, 'GIT_CONFIG_GLOBAL': os.devnull,
            'GIT_CONFIG_SYSTEM': os.devnull, 'GIT_CONFIG_COUNT': '3',
            'GIT_CONFIG_KEY_0': 'credential.helper', 'GIT_CONFIG_VALUE_0': '',
            'GIT_CONFIG_KEY_1': 'http.sslCAInfo', 'GIT_CONFIG_VALUE_1': str(fixture.ca_file),
            'GIT_CONFIG_KEY_2': 'http.schannelUseSSLCAInfo', 'GIT_CONFIG_VALUE_2': 'true',
            'OPENGIT_E2E_DIALOGS_JSON': json.dumps([str(parent)] * 4)}
        with DesktopSession(ROOT, executable, env) as desktop:
            wait_for(lambda: desktop.script('return !!window.__TAURI_INTERNALS__'), 'IPC ready')
            value(desktop.invoke('get_workspace', {}))
            destination = value(desktop.invoke('choose_clone_destination', {}))
            before_invalid = fixture.https_requests
            request = {'source': {'kind': 'remote', 'url': fixture.https_url},
                       'destinationHandle': destination['destinationHandle'], 'directoryName': 'existing'}
            reject_clone(desktop, request, ('DestinationExists',))
            assert sentinel.read_bytes() == b'preserve-existing-destination'
            for name in ('../escape', 'NUL', 'trail.', 'C:drive'):
                reject_clone(desktop, {**request, 'directoryName': name}, ('InvalidRequest', 'UnsafePath'))
            for url in ('ext::synthetic', '--upload-pack=synthetic',
                        fixture.https_url + '?token=SYNTHETIC_REDACTION_CANARY'):
                reject_clone(desktop, {**request, 'directoryName': 'invalid',
                    'source': {'kind': 'remote', 'url': url}}, ('InvalidRequest', 'InvalidSource'))
            assert 'SYNTHETIC_REDACTION_CANARY' not in json.dumps(value(desktop.invoke('get_operations', {})))
            assert fixture.https_requests == before_invalid
            print('PASS: native rejects unsafe source/name and preserves existing destination')
            denied_id, username_prompt = start_waiting(desktop, fixture, 'denied')
            assert operation(desktop, denied_id)['authPrompt']['kind'] == 'username'
            value(desktop.invoke('respond_auth_prompt', {'operationId': denied_id,
                'promptHandle': username_prompt, 'value': fixture.username}))
            wait_for(lambda: (operation(desktop, denied_id)['authPrompt'] or {}).get('kind') == 'password',
                     'password challenge')
            wrong_password = 'deliberately-denied-' + fixture.password
            value(desktop.invoke('respond_auth_prompt', {'operationId': denied_id,
                'promptHandle': operation(desktop, denied_id)['authPrompt']['promptHandle'],
                'value': wrong_password}))
            wait_for(lambda: operation(desktop, denied_id)['state'] == 'Failed', 'denied authentication')
            assert operation(desktop, denied_id)['error']['code'] == 'AuthenticationFailed'
            assert not (parent / 'denied').exists()
            assert_no_credentials(data, fixture, active=True)
            wait_for(lambda: desktop.script('return !document.querySelector("dialog.auth-modal[open]")'),
                     'denied dialog dismissed')
            print('PASS: real denied HTTPS credentials return safe failure without publication or persisted secrets')
            before_cancel = staging_inventory(parent)
            operation_id, prompt = start_waiting(desktop, fixture, 'cancelled')
            # Exercise the real auth modal's cancel path, not just its IPC.
            wait_for(lambda: desktop.script('return !!document.querySelector("dialog.auth-modal[open]")'),
                     'visible cancellation dialog')
            desktop.script('const b=[...document.querySelectorAll("dialog.auth-modal[open] button")].find('
                'b=>b.textContent.trim()==="Cancel authentication");b.click();')
            wait_for(lambda: operation(desktop, operation_id)['state'] == 'Cancelled', 'cancel terminal')
            stale = desktop.invoke('respond_auth_prompt', {'operationId': operation_id,
                'promptHandle': prompt, 'value': fixture.username})
            assert not stale['ok'] and stale['error']['code'] == 'InvalidHandle'
            assert not (parent / 'cancelled').exists()
            after_cancel = staging_inventory(parent)
            if set(after_cancel) - set(before_cancel):
                assert_retention_notice(operation(desktop, operation_id))
            print('PASS: native cancellation invalidates auth prompt and leaves no destination')
            interrupted_id, _ = start_waiting(desktop, fixture, 'interrupted')
            before_crash = staging_inventory(parent)
            desktop.crash_application()
        # Include requests made before the restarted process exposes its IPC.
        requests_before_restart = fixture.https_requests
        with DesktopSession(ROOT, executable, {**env, 'OPENGIT_E2E_DIALOGS_JSON': '[]'}) as reopened:
            wait_for(lambda: reopened.script('return !!window.__TAURI_INTERNALS__'), 'restart IPC')
            wait_for(lambda: operation(reopened, interrupted_id)['state'] == 'Interrupted', 'durable interrupted task')
            assert operation(reopened, interrupted_id)['authPrompt'] is None
            assert_no_restart_requests(fixture, requests_before_restart)
            time.sleep(2)
            assert_no_restart_requests(fixture, requests_before_restart)
            assert operation(reopened, interrupted_id)['state'] == 'Interrupted'
            assert not (parent / 'interrupted').exists()
            after_restart = staging_inventory(parent)
            for name, identity in after_cancel.items():
                assert after_restart.get(name) == identity, 'Restart altered a retained artifact'
            if set(after_restart) - set(after_cancel):
                assert_retention_notice(operation(reopened, interrupted_id))
            for name, identity in after_restart.items():
                assert before_crash.get(name) == identity, 'Restart substituted an owned fixture artifact'
            assert sentinel.read_bytes() == b'preserve-existing-destination'
            assert not workspace(reopened)['repositories']
            assert_no_credentials(data, fixture, active=True)
            print('PASS: native crash/restart preserves destination, invalidates prompts and avoids auto-resume')
        # Remove fixture CA trust for this process only. Verification must fail
        # before authentication rather than being disabled to make clone work.
        untrusted_env = {**env, 'GIT_CONFIG_COUNT': '1', 'GIT_SSL_NO_VERIFY': None,
                         'OPENGIT_E2E_DIALOGS_JSON': json.dumps([str(parent)])}
        with DesktopSession(ROOT, executable, untrusted_env) as desktop:
            wait_for(lambda: desktop.script('return !!window.__TAURI_INTERNALS__'), 'untrusted TLS IPC')
            value(desktop.invoke('get_workspace', {}))
            trust_failure(desktop, fixture.https_url, 'bad-certificate', 'Certificate')
            assert not (parent / 'bad-certificate').exists()
            print('PASS: native untrusted certificate is rejected without weakening TLS')
        port = fixture.ssh_url.split(':')[-1].split('/')[0]
        wrong_host = fixture.root / 'wrong-known-hosts'
        wrong_host.write_bytes(f'[127.0.0.1]:{port} '.encode() + fixture._client_key.export_public_key())
        ssh = Path(os.environ.get('WINDIR', r'C:\Windows')) / 'System32/OpenSSH/ssh.exe'
        ssh_env = {**env, 'GIT_SSH_VARIANT': 'ssh', 'SSH_AUTH_SOCK': '',
            'GIT_SSH_COMMAND': (f'"{ssh.as_posix()}" -F NUL -o '
                f'UserKnownHostsFile="{wrong_host.as_posix()}" -o StrictHostKeyChecking=yes -o IdentityFile=none'),
            'OPENGIT_E2E_DIALOGS_JSON': json.dumps([str(parent)])}
        with DesktopSession(ROOT, executable, ssh_env) as desktop:
            wait_for(lambda: desktop.script('return !!window.__TAURI_INTERNALS__'), 'host mismatch IPC')
            value(desktop.invoke('get_workspace', {}))
            trust_failure(desktop, fixture.ssh_url, 'bad-host-key', 'SshHostKey')
            assert not (parent / 'bad-host-key').exists()
            assert sentinel.read_bytes() == b'preserve-existing-destination'
            assert_no_credentials(data, fixture, active=True)
            print('PASS: native changed SSH host key is rejected without trusting it')
        with RejectingProxy() as proxy:
            proxy_env = {**env, 'HTTPS_PROXY': proxy.url, 'https_proxy': proxy.url,
                'NO_PROXY': '', 'no_proxy': '', 'ALL_PROXY': '', 'all_proxy': '',
                'OPENGIT_E2E_DIALOGS_JSON': json.dumps([str(parent)])}
            with DesktopSession(ROOT, executable, proxy_env) as desktop:
                wait_for(lambda: desktop.script('return !!window.__TAURI_INTERNALS__'), 'proxy failure IPC')
                value(desktop.invoke('get_workspace', {}))
                trust_failure(desktop, fixture.https_url, 'rejected-proxy', 'Network')
                assert proxy.connect_requests > 0, 'Configured proxy was bypassed'
                assert not (parent / 'rejected-proxy').exists()
                assert_no_credentials(data, fixture, active=True)
                print('PASS: native configured proxy is respected; rejected CONNECT has a safe network error')

        assert_no_credentials(data, fixture)
