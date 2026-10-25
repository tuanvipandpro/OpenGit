"""Real native fetch over owned private HTTPS and pinned-agent SSH remotes.

Folder choice is injected; fetch preparation/execution uses the visible UI.
Read-only operation snapshots support the existing actual GUI auth helper.
No credential or IPC command result is mocked or written into artifacts.
"""
import json
import os
from pathlib import Path
import sys

from clone_acceptance import (assert_no_credentials, assert_profile_has_no_credentials,
                              value, wait_operation)
from desktop_smoke import DesktopSession
from history_ui_acceptance import idle
from workspace_acceptance import body_contains, click_text, wait_for

ROOT = Path(__file__).resolve().parents[2]
sys.path[:0] = [str(ROOT / '.tools/fixture-python'), str(ROOT / 'tests/e2e/support')]
from live_remotes import LiveRemotes


def advance(fixture, peer, label):
    assert peer.resolve().is_relative_to(fixture.root)
    (peer / 'remote-change.txt').write_text('Owned remote update: ' + label + '\n', encoding='utf-8')
    fixture._git('-C', str(peer), 'add', '--', 'remote-change.txt')
    fixture._git('-C', str(peer), '-c', 'user.name=OpenGit Fixture',
                 '-c', 'user.email=fixture@invalid.example', 'commit', '-m', 'Owned remote update')
    fixture._git('-C', str(peer), 'push', 'origin', 'HEAD:refs/heads/main')
    return fixture._git('-C', str(peer), 'rev-parse', 'HEAD').stdout.strip()


def native_fetch(desktop, fixture):
    before = {op['id'] for op in value(desktop.invoke('get_operations', {}))['operations']}
    click_text(desktop, 'Fetch…')
    wait_for(lambda: desktop.script('return !!document.querySelector(".fetch-dialog")'), 'fetch dialog')
    desktop.click('input[aria-label="Fetch origin"]')
    assert desktop.script('return document.querySelector(".fetch-prune input")?.checked') is False
    click_text(desktop, 'Prepare fetch')
    wait_for(lambda: desktop.script('''return !!document.querySelector('[aria-label="Fetch preview"]')'''),
             'real prepared private fetch', timeout=60)
    click_text(desktop, 'Start fetch')
    def started():
        return next((op for op in value(desktop.invoke('get_operations', {}))['operations']
                     if op['id'] not in before), None)
    wait_for(started, 'actual fetch queued')
    operation, prompts = wait_operation(desktop, started()['id'], fixture)
    assert operation['kind'] == 'action' and operation['actionKind'] == 'fetch'
    assert operation['outcome']['effects'] == 'applied'
    wait_for(lambda: desktop.script('''
        const task=document.querySelector('article[aria-label="Fetch remotes task"]');
        return !!task&&task.querySelector('.task-state')?.textContent==='Succeeded'
            &&task.textContent.includes('Changes applied')
            &&!!task.querySelector('[aria-label="Fetch remote results"]');
    '''), 'private fetch actual task effects', timeout=60)
    return prompts


if __name__ == '__main__':
    executable = Path(os.environ.get('OPENGIT_E2E_EXECUTABLE',
                                   str(ROOT / 'src-tauri/target/debug/opengit.exe')))
    with LiveRemotes(ROOT) as fixture:
        peer = fixture.root / 'peer'
        fixture._git('clone', '--', str(fixture.repository), str(peer))
        fixture._git('-C', str(fixture.source), 'remote', 'add', 'origin', fixture.https_url)
        fixture._git('-C', str(fixture.source), 'config', 'branch.main.remote', 'origin')
        fixture._git('-C', str(fixture.source), 'config', 'branch.main.merge', 'refs/heads/main')
        head = fixture._git('-C', str(fixture.source), 'rev-parse', 'HEAD').stdout
        index = (fixture.source / '.git/index').read_bytes()
        source_file = (fixture.source / 'README.txt').read_bytes()
        expected = advance(fixture, peer, 'https')
        data = fixture.root / 'https-fetch-data'
        env = {'OPENGIT_DATA_DIR': str(data), 'GIT_SSL_NO_VERIFY': None,
               'GIT_CONFIG_GLOBAL': os.devnull, 'GIT_CONFIG_SYSTEM': os.devnull,
               'GIT_CONFIG_PARAMETERS': None, 'GIT_CONFIG_COUNT': '3',
               'GIT_CONFIG_KEY_0': 'credential.helper', 'GIT_CONFIG_VALUE_0': '',
               'GIT_CONFIG_KEY_1': 'http.sslCAInfo', 'GIT_CONFIG_VALUE_1': str(fixture.ca_file),
               'GIT_CONFIG_KEY_2': 'http.schannelUseSSLCAInfo', 'GIT_CONFIG_VALUE_2': 'true',
               'OPENGIT_E2E_DIALOGS_JSON': json.dumps([str(fixture.source)])}
        with DesktopSession(ROOT, executable, env) as desktop:
            wait_for(lambda: body_contains(desktop, 'Your repositories'), 'private HTTPS workspace')
            click_text(desktop, 'Open repository')
            wait_for(lambda: idle(desktop), 'private HTTPS history', timeout=90)
            prompts = native_fetch(desktop, fixture)
            assert {'username', 'password'}.issubset(prompts)
            assert fixture._git('-C', str(fixture.source), 'rev-parse', 'origin/main').stdout.strip() == expected
            profile = desktop.directory
        assert_no_credentials(data, fixture)
        assert_profile_has_no_credentials(profile, fixture)
        print('PASS: native private HTTPS fetch with verified trust, GUI auth and real new objects', flush=True)

        ssh, agent_env = fixture.start_agent()
        from urllib.parse import urlsplit
        port = urlsplit(fixture.ssh_url).port
        public = fixture.server_public_key.export_public_key('openssh').decode().strip()
        fixture.known_hosts.write_text(f'[127.0.0.1]:{port} {public}\n', encoding='ascii')
        command = (f'"{ssh.as_posix()}" -F /dev/null -o '
                   f'UserKnownHostsFile="{fixture.known_hosts.as_posix()}" '
                   '-o GlobalKnownHostsFile=/dev/null -o StrictHostKeyChecking=yes '
                   '-o IdentityFile=none -o PreferredAuthentications=publickey')
        fixture._git('-C', str(fixture.source), 'remote', 'set-url', 'origin', fixture.ssh_url)
        expected = advance(fixture, peer, 'ssh')
        data = fixture.root / 'ssh-fetch-data'
        ssh_env = {**env, 'OPENGIT_DATA_DIR': str(data), 'GIT_CONFIG_COUNT': '1',
                   'GIT_SSH_COMMAND': command, 'GIT_SSH_VARIANT': 'ssh',
                   'SSH_AUTH_SOCK': agent_env['SSH_AUTH_SOCK']}
        with DesktopSession(ROOT, executable, ssh_env) as desktop:
            wait_for(lambda: body_contains(desktop, 'Your repositories'), 'private SSH workspace')
            click_text(desktop, 'Open repository')
            wait_for(lambda: idle(desktop), 'private SSH history', timeout=90)
            prompts = native_fetch(desktop, fixture)
            assert not prompts, 'Pinned owned agent must not need a GUI secret'
            assert fixture._agent.poll() is None
            assert fixture._git('-C', str(fixture.source), 'rev-parse', 'origin/main').stdout.strip() == expected
            profile = desktop.directory
        assert_no_credentials(data, fixture)
        assert_profile_has_no_credentials(profile, fixture)
        assert fixture._git('-C', str(fixture.source), 'rev-parse', 'HEAD').stdout == head
        assert (fixture.source / '.git/index').read_bytes() == index
        assert (fixture.source / 'README.txt').read_bytes() == source_file
        print('PASS: native pinned-host SSH agent fetch; worktree, index and local HEAD preserved', flush=True)
