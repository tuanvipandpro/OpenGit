"""Native authenticated remote-ref read before its S7 management UI is added.

The fresh renderer remains on Welcome: the test owns the only history/action
scope using selection returned by the real folder chooser. It does not inject
results or compete with a renderer history generation. Auth and Tasks use real UI.
"""
import json
import os
from pathlib import Path
import subprocess
import sys

from clone_acceptance import (assert_no_credentials, assert_profile_has_no_credentials,
                              value, wait_operation)
from daily_transport_acceptance import advance
from desktop_smoke import DesktopSession
from workspace_acceptance import body_contains, wait_for

ROOT = Path(__file__).resolve().parents[2]
sys.path[:0] = [str(ROOT / '.tools/fixture-python'), str(ROOT / 'tests/e2e/support')]
from live_remotes import LiveRemotes


def tree(path):
    return {str(file.relative_to(path)): (file.read_bytes(), file.stat().st_mtime_ns)
            for file in path.rglob('*') if file.is_file()}


def no_persisted_capabilities(data, tokens):
    for file in data.rglob('*'):
        if file.is_file():
            contents = file.read_bytes()
            for token in tokens:
                assert token.encode() not in contents, 'Runtime authority persisted in app data'
                assert token.encode('utf-16-le') not in contents, 'Runtime authority persisted as UTF-16'


def read_refs(desktop, fixture, role, expected_refs):
    registered = value(desktop.invoke('choose_repository', {}))
    selection = registered['activeSelection']
    assert selection
    assert not desktop.script('return !!document.querySelector(".history-screen")'), \
        'Scope-owning native test must not compete with renderer history'
    scope = {**selection, 'generation': '1'}
    catalog = value(desktop.invoke('get_remote_actions', {'scope': scope}))
    assert catalog['scope'] == scope
    remote = next(item for item in catalog['remotes'] if item['name'] == 'origin')
    endpoint = next(item for item in remote['endpoints'] if item['role'] == role)
    assert endpoint['capability'] == 'eligible' and endpoint['handle']
    prepared = value(desktop.invoke('prepare_repository_action', {
        'scope': scope, 'intent': {'kind': 'readRemoteRefs', 'remoteHandle': endpoint['handle']}}))
    operation = value(desktop.invoke('execute_repository_action', {'planId': prepared['planId']}))
    result, prompts = wait_operation(desktop, operation['id'], fixture)
    assert result['actionKind'] == 'readRemoteRefs'
    assert result['outcome']['effects'] == 'notApplied' and not result['outcome']['repositoryChanged']
    wait_for(lambda: desktop.script('''
        const p=document.querySelector('article[aria-label="Read remote references task"]');
        return p?.querySelector('.task-state')?.textContent==='Succeeded';
    '''), 'correct remote-read task title and outcome')
    assert not body_contains(desktop, 'Per-remote fetch evidence is unavailable')
    outcome = value(desktop.invoke('get_remote_action_outcome', {'operationId': operation['id']}))
    assert outcome['availability'] == 'available' and outcome['evidence']['noGitWrites']
    snapshot = outcome['snapshot']
    assert snapshot['capturedScope'] == scope and snapshot['role'] == role
    assert {ref['name']: ref['oid'] for ref in snapshot['refs']} == expected_refs
    assert int(outcome['evidence']['refCount']) == len(expected_refs)
    main = next(ref for ref in snapshot['refs'] if ref['name'] == 'refs/heads/main')
    current = {**selection, 'generation': '2'}
    existing = value(desktop.invoke('resolve_remote_target', {
        'scope': current, 'snapshotId': snapshot['snapshotId'],
        'source': {'kind': 'existing', 'entryId': main['entryId']}}))
    assert existing['scope'] == current and existing['expectedOid'] == main['oid']
    absent = value(desktop.invoke('resolve_remote_target', {
        'scope': current, 'snapshotId': snapshot['snapshotId'],
        'source': {'kind': 'new', 'namespace': 'tag', 'name': 'native-read-only-proof'}}))
    assert absent['expectedOid'] is None
    assert fixture._git('-C', str(fixture.repository), 'for-each-ref', '--format=%(refname)',
                        'refs/tags/native-read-only-proof').stdout == b''
    tokens = [endpoint['handle'], prepared['planId'], snapshot['snapshotId'],
              existing['handle'], absent['handle'], *[ref['entryId'] for ref in snapshot['refs']]]
    return prompts, tokens


if __name__ == '__main__':
    executable = Path(os.environ.get('OPENGIT_E2E_EXECUTABLE',
                                   str(ROOT / 'src-tauri/target/debug/opengit.exe')))
    with LiveRemotes(ROOT) as fixture:
        peer = fixture.root / 'peer'
        fixture._git('clone', '--', str(fixture.repository), str(peer))
        fixture._git('-C', str(fixture.source), 'remote', 'add', 'origin', fixture.https_url)
        ssh, agent_env = fixture.start_agent()
        from urllib.parse import urlsplit
        port = urlsplit(fixture.ssh_url).port
        public = fixture.server_public_key.export_public_key('openssh').decode().strip()
        fixture.known_hosts.write_text(f'[127.0.0.1]:{port} {public}\n', encoding='ascii')
        ssh_command = (f'"{ssh.as_posix()}" -F /dev/null -o '
                       f'UserKnownHostsFile="{fixture.known_hosts.as_posix()}" '
                       '-o GlobalKnownHostsFile=/dev/null -o StrictHostKeyChecking=yes '
                       '-o IdentityFile=none -o PreferredAuthentications=publickey')
        for transport, role, url in [('https', 'push', fixture.https_url), ('ssh', 'fetch', fixture.ssh_url)]:
            fixture._git('-C', str(fixture.source), 'remote', 'set-url', 'origin', url)
            new_oid = advance(fixture, peer, 'remote-read-' + transport)
            fixture._git('-C', str(fixture.repository), 'update-ref', 'refs/tags/read-' + transport,
                         new_oid.decode())
            expected = dict(line.split() for line in fixture._git('-C', str(fixture.repository),
                            'for-each-ref', '--format=%(refname) %(objectname)',
                            'refs/heads/', 'refs/tags/').stdout.decode().splitlines())
            before = tree(fixture.source)
            data = fixture.root / (transport + '-remote-read-data')
            env = {'OPENGIT_DATA_DIR': str(data), 'GIT_SSL_NO_VERIFY': None,
                   'GIT_CONFIG_GLOBAL': os.devnull, 'GIT_CONFIG_SYSTEM': os.devnull,
                   'GIT_CONFIG_PARAMETERS': None, 'GIT_CONFIG_COUNT': '3',
                   'GIT_CONFIG_KEY_0': 'credential.helper', 'GIT_CONFIG_VALUE_0': '',
                   'GIT_CONFIG_KEY_1': 'http.sslCAInfo', 'GIT_CONFIG_VALUE_1': str(fixture.ca_file),
                   'GIT_CONFIG_KEY_2': 'http.schannelUseSSLCAInfo', 'GIT_CONFIG_VALUE_2': 'true',
                   'GIT_SSH_COMMAND': ssh_command if transport == 'ssh' else None,
                   'GIT_SSH_VARIANT': 'ssh' if transport == 'ssh' else None,
                   'SSH_AUTH_SOCK': agent_env['SSH_AUTH_SOCK'] if transport == 'ssh' else None,
                   'OPENGIT_E2E_DIALOGS_JSON': json.dumps([str(fixture.source)])}
            with DesktopSession(ROOT, executable, env) as desktop:
                wait_for(lambda: body_contains(desktop, 'Your repositories'), 'scope-owning welcome')
                prompts, tokens = read_refs(desktop, fixture, role, expected)
                assert {'username', 'password'}.issubset(prompts) if transport == 'https' else not prompts
                profile = desktop.directory
            assert tree(fixture.source) == before, 'Remote read changed local files or Git metadata'
            missing = subprocess.run([fixture.git, '-C', str(fixture.source), '--no-lazy-fetch',
                                      'cat-file', '-e', new_oid.decode()], env=fixture.env,
                                     capture_output=True, timeout=30,
                                     creationflags=0x08000000 if os.name == 'nt' else 0)
            assert missing.returncode != 0, 'Remote ref read unexpectedly downloaded objects'
            no_persisted_capabilities(data, tokens)
            assert_no_credentials(data, fixture)
            assert_profile_has_no_credentials(profile, fixture)
            assert fixture._agent.poll() is None
            print('PASS: native authenticated ' + transport + ' ' + role +
                  ' endpoint read, current-scope resolver, no Git writes/downloads or persisted authority', flush=True)
