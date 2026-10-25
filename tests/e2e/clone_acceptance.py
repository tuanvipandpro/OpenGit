"""Real native clone/auth acceptance against owned loopback Git transports.

Requires an e2e-feature desktop build. Only native folder selection is injected;
Git, auth broker, SQLite and filesystem publication execute in the application.
Never print credentials or serialize request/response payloads into artifacts.
"""
from pathlib import Path
import json
import os
import sqlite3
import sys
import time

from desktop_smoke import DesktopSession
from workspace_acceptance import wait_for, body_contains, workspace, click_text

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / '.tools/fixture-python'))
sys.path.insert(0, str(ROOT / 'tests/e2e/support'))
from live_remotes import LiveRemotes


def value(result):
    if not result.get('ok'):
        # Only bounded safe error codes are eligible for test diagnostics.
        code = result.get('error', {}).get('code', 'transport')
        raise AssertionError('Application request failed: ' + str(code)[:40])
    return result['value']


def answer_auth_form(desktop, prompt, response, source_display):
    modal = 'dialog.auth-modal[open]'
    wait_for(lambda: desktop.script('return !!document.querySelector(arguments[0])', modal), 'visible auth dialog')
    assert desktop.script('return document.querySelector(arguments[0]).innerText.includes(arguments[1])',
                          modal, source_display), 'Authentication dialog shows the wrong repository context'
    if prompt['kind'] == 'confirmation':
        assert body_contains(desktop, prompt['message'])
        desktop.click(modal + ' button.primary')
    else:
        selector = modal + ' input[name="auth-response"]'
        wait_for(lambda: desktop.script('const e=document.querySelector(arguments[0]);return !!e&&!e.disabled', selector),
                 'enabled auth field')
        expected_type = 'text' if prompt['kind'] == 'username' else 'password'
        assert desktop.script('return document.querySelector(arguments[0]).type', selector) == expected_type
        desktop.fill(selector, response)
        if prompt['kind'] != 'username':
            assert not body_contains(desktop, response), 'Secret rendered as visible text'
        desktop.click(modal + ' button[type="submit"]')


def wait_operation(desktop, operation_id, fixture, timeout=150):
    deadline = time.monotonic() + timeout
    answered = set()
    seen = set()
    while time.monotonic() < deadline:
        snapshots = value(desktop.invoke('get_operations', {}))
        operation = next(o for o in snapshots['operations'] if o['id'] == operation_id)
        prompt = operation['authPrompt']
        if prompt and prompt['promptHandle'] not in answered:
            kind = prompt['kind']
            print('Observed auth prompt kind:', kind, flush=True)
            assert kind in ('username', 'password', 'passphrase', 'confirmation')
            response = {'username': fixture.username, 'password': fixture.password,
                        'passphrase': fixture.key_passphrase, 'confirmation': 'yes'}[kind]
            if kind == 'confirmation':
                assert fixture.host_fingerprint in prompt['message'], 'Confirmation lacks actual host fingerprint'
            answer_auth_form(desktop, prompt, response, operation['sourceDisplay'])
            wait_for(lambda: not any(o['id'] == operation_id and o['authPrompt'] and
                     o['authPrompt']['promptHandle'] == prompt['promptHandle']
                     for o in value(desktop.invoke('get_operations', {}))['operations']), 'one-time GUI response')
            replay = desktop.invoke('respond_auth_prompt', {
                'operationId': operation_id, 'promptHandle': prompt['promptHandle'], 'value': response})
            assert replay.get('ok') is False and replay['error']['code'] == 'InvalidHandle'
            answered.add(prompt['promptHandle'])
            seen.add(kind)
        if operation['state'] in ('Succeeded', 'Failed', 'Cancelled', 'Interrupted'):
            if operation['state'] != 'Succeeded':
                code = (operation['error'] or {}).get('code', 'none')
                raise AssertionError('Clone terminal state: ' + operation['state'] + '/' + code +
                                     '; prompt kinds: ' + ','.join(sorted(seen)))
            return operation, seen
        time.sleep(.1)
    raise AssertionError('Clone did not reach terminal state')


def clone(desktop, source, name, fixture):
    destination = value(desktop.invoke('choose_clone_destination', {}))
    assert destination
    operation = value(desktop.invoke('start_clone', {
        'source': source, 'destinationHandle': destination['destinationHandle'], 'directoryName': name}))
    result, prompts = wait_operation(desktop, operation['id'], fixture)
    target = fixture.root / 'clones' / name
    assert (target / '.git').is_dir() and (target / 'README.txt').is_file()
    fixture._git('-C', str(target), 'fsck', '--no-reflogs')
    assert result['repoId']
    assert any(r['id'] == result['repoId'] for r in workspace(desktop)['repositories'])
    return prompts


def clone_form(desktop, source, name, fixture):
    before = {o['id'] for o in value(desktop.invoke('get_operations', {}))['operations']}
    click_text(desktop, 'Clone repository')
    wait_for(lambda: desktop.script('return !!document.querySelector("dialog[open] input[name=repository-url]")'),
             'clone form')
    if source == 'local':
        desktop.click('dialog[open] input[type="radio"][value="local"]')
        click_text(desktop, 'Choose source folder')
        wait_for(lambda: not body_contains(desktop, 'No source selected'), 'source chooser result')
    else:
        desktop.fill('dialog[open] input[name="repository-url"]', source)
    click_text(desktop, 'Choose destination folder')
    wait_for(lambda: not body_contains(desktop, 'No parent folder selected'), 'destination chooser result')
    desktop.fill('dialog[open] input[name="directory-name"]', name)
    desktop.click('dialog[open] button[type="submit"]')
    def started():
        return next((o for o in value(desktop.invoke('get_operations', {}))['operations'] if o['id'] not in before), None)
    wait_for(started, 'GUI clone queued')
    result, prompts = wait_operation(desktop, started()['id'], fixture)
    target = fixture.root / 'clones' / name
    assert (target / '.git').is_dir() and (target / 'README.txt').is_file()
    fixture._git('-C', str(target), 'fsck', '--no-reflogs')
    wait_for(lambda: any(r['id'] == result['repoId'] for r in workspace(desktop)['repositories']), 'GUI workspace refresh')
    return prompts


def assert_no_credentials(data, fixture, *, active=False):
    for path in data.rglob('*'):
        if path.is_file():
            # The app intentionally holds this empty ownership lock with no sharing.
            # Closed-session scans below still read every file without exceptions.
            if active and path == data / "workspace.owner":
                assert path.stat().st_size == 0
                continue
            contents = path.read_bytes()
            for secret in (fixture.password, fixture.key_passphrase):
                assert secret.encode() not in contents, 'Credential persisted in application data'
                assert secret.encode('utf-16-le') not in contents, 'Credential persisted as UTF-16'


def assert_profile_has_no_credentials(directory, fixture):
    """Inspect only this closed native session's generated browser/log data."""
    assert directory.parent == ROOT / '.tmp' and directory.name.startswith('desktop-')
    assert_no_credentials(directory, fixture)
    for database in directory.rglob('Login Data'):
        with sqlite3.connect(database.as_uri() + '?mode=ro', uri=True) as connection:
            tables = {row[0] for row in connection.execute("SELECT name FROM sqlite_master WHERE type='table'")}
            if 'logins' in tables:
                assert connection.execute('SELECT COUNT(*) FROM logins').fetchone()[0] == 0, \
                    'App credential form was persisted by WebView password storage'


if __name__ == '__main__':
    executable = Path(os.environ.get('OPENGIT_E2E_EXECUTABLE',
                                   str(ROOT / 'src-tauri/target/release/opengit.exe')))
    with LiveRemotes(ROOT) as fixture:
        clones = fixture.root / 'clones'
        clones.mkdir()
        data = fixture.root / 'clone-app-data'
        env = {'OPENGIT_DATA_DIR': str(data), 'GIT_SSL_NO_VERIFY': None, 'GIT_CONFIG_GLOBAL': os.devnull,
               'GIT_CONFIG_SYSTEM': os.devnull, 'GIT_CONFIG_COUNT': '3',
               'GIT_CONFIG_KEY_0': 'credential.helper', 'GIT_CONFIG_VALUE_0': '',
               'GIT_CONFIG_KEY_1': 'http.sslCAInfo', 'GIT_CONFIG_VALUE_1': str(fixture.ca_file),
               'GIT_CONFIG_KEY_2': 'http.schannelUseSSLCAInfo', 'GIT_CONFIG_VALUE_2': 'true',
               'OPENGIT_E2E_DIALOGS_JSON': json.dumps([str(fixture.source), str(clones), str(clones)])}
        with DesktopSession(ROOT, executable, env) as desktop:
            wait_for(lambda: body_contains(desktop, 'Your repositories'), 'native workspace ready')
            clone_form(desktop, 'local', 'local', fixture)
            print('PASS: native local clone form, publication, Git verification and registry')
            prompts = clone_form(desktop, fixture.https_url, 'private-https', fixture)
            assert 'username' in prompts and 'password' in prompts
            assert_no_credentials(data, fixture, active=True)
            click_text(desktop, 'Overview')
            wait_for(lambda: body_contains(desktop, 'Working directory is clean'),
                     'cloned repository inspection rendered')
            desktop.screenshot('clone.png')
            print('PASS: native private HTTPS clone with certificate verification and GUI auth broker')
        assert_profile_has_no_credentials(desktop.directory, fixture)
        assert_no_credentials(data, fixture)
        # A new GUI process uses actual OpenSSH, isolated config/known_hosts and
        # no inherited agent or personal identities. Unknown host requires GUI
        # confirmation of this fixture's generated fingerprint.
        ssh = Path(os.environ.get('WINDIR', r'C:\Windows')) / 'System32/OpenSSH/ssh.exe'
        assert ssh.is_file()
        ssh_command = (f'"{ssh.as_posix()}" -F NUL -o '
                       f'UserKnownHostsFile="{fixture.known_hosts.as_posix()}" '
                       '-o GlobalKnownHostsFile=NUL -o StrictHostKeyChecking=ask -o PubkeyAuthentication=no -o IdentityFile=none '
                       '-o PreferredAuthentications=password -o NumberOfPasswordPrompts=1')
        ssh_env = {**env, 'GIT_SSH_COMMAND': ssh_command, 'GIT_SSH_VARIANT': 'ssh',
                   'SSH_AUTH_SOCK': '', 'OPENGIT_E2E_DIALOGS_JSON': json.dumps([str(clones)])}
        with DesktopSession(ROOT, executable, ssh_env) as desktop:
            wait_for(lambda: body_contains(desktop, 'OpenGit'), 'SSH native process ready')
            prompts = clone(desktop, {'kind': 'remote', 'url': fixture.ssh_url}, 'private-ssh', fixture)
            assert {'confirmation', 'password'}.issubset(prompts)
            assert_no_credentials(data, fixture, active=True)
            print('PASS: native private SSH password clone and explicit verified host fingerprint')
        assert_profile_has_no_credentials(desktop.directory, fixture)
        # A separately owned agent proves reuse without putting a private key
        # into the app. The host was explicitly accepted in the previous case.
        agent_ssh, agent_env = fixture.start_agent()
        command = (f'"{agent_ssh.as_posix()}" -F /dev/null -o '
                   f'UserKnownHostsFile="{fixture.known_hosts.as_posix()}" '
                   '-o GlobalKnownHostsFile=/dev/null -o StrictHostKeyChecking=yes -o IdentityFile=none -o PreferredAuthentications=publickey')
        with DesktopSession(ROOT, executable, {**env, 'SSH_AUTH_SOCK': agent_env['SSH_AUTH_SOCK'],
                'GIT_SSH_COMMAND': command, 'GIT_SSH_VARIANT': 'ssh',
                'OPENGIT_E2E_DIALOGS_JSON': json.dumps([str(clones)])}) as desktop:
            wait_for(lambda: body_contains(desktop, 'OpenGit'), 'agent native process ready')
            prompts = clone(desktop, {'kind': 'remote', 'url': fixture.ssh_url}, 'ssh-agent', fixture)
            assert not prompts
            assert fixture._agent.poll() is None
            print('PASS: native SSH agent clone; pre-existing owned fixture agent remains alive')
        key_command = (f'"{agent_ssh.as_posix()}" -F /dev/null -o '
                       f'UserKnownHostsFile="{fixture.known_hosts.as_posix()}" '
                       f'-i "{fixture.client_key_file.as_posix()}" '
                       '-o GlobalKnownHostsFile=/dev/null -o StrictHostKeyChecking=yes -o IdentitiesOnly=yes -o IdentityAgent=none '
                       '-o PreferredAuthentications=publickey')
        with DesktopSession(ROOT, executable, {**env, 'SSH_AUTH_SOCK': '',
                'GIT_SSH_COMMAND': key_command, 'GIT_SSH_VARIANT': 'ssh',
                'OPENGIT_E2E_DIALOGS_JSON': json.dumps([str(clones)])}) as desktop:
            wait_for(lambda: body_contains(desktop, 'OpenGit'), 'encrypted-key native process ready')
            prompts = clone(desktop, {'kind': 'remote', 'url': fixture.ssh_url}, 'ssh-passphrase', fixture)
            assert prompts == {'passphrase'}
            print('PASS: native encrypted SSH key clone with transient passphrase prompt')
        assert_profile_has_no_credentials(desktop.directory, fixture)
        fixture.allow_anonymous = True
        with DesktopSession(ROOT, executable, {**env, 'GIT_SSH_COMMAND': command,
                'GIT_SSH_VARIANT': 'ssh', 'SSH_AUTH_SOCK': '',
                'OPENGIT_E2E_DIALOGS_JSON': json.dumps([str(clones), str(clones)])}) as desktop:
            wait_for(lambda: body_contains(desktop, 'OpenGit'), 'public transport process ready')
            assert not clone(desktop, {'kind': 'remote', 'url': fixture.https_url}, 'public-https', fixture)
            assert not clone(desktop, {'kind': 'remote', 'url': fixture.ssh_url}, 'public-ssh', fixture)
            print('PASS: native public HTTPS and SSH clones without credential prompts')
        assert_no_credentials(data, fixture)
