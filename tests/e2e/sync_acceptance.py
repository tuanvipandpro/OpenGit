"""Real native Stage/Commit -> authenticated push -> fast-forward pull."""
import json
import os
from pathlib import Path
import sys

from clone_acceptance import (assert_no_credentials, assert_profile_has_no_credentials,
                              value, wait_operation)
from daily_transport_acceptance import advance
from desktop_smoke import DesktopSession
from stage_commit_acceptance import commit_form, execute_form, file_action
from workspace_acceptance import body_contains, click_text, wait_for

ROOT = Path(__file__).resolve().parents[2]
sys.path[:0] = [str(ROOT / '.tools/fixture-python'), str(ROOT / 'tests/e2e/support')]
from live_remotes import LiveRemotes


def current_operations(desktop):
    return value(desktop.invoke('get_operations', {}))['operations']


def new_operation(desktop, before):
    return next((op for op in current_operations(desktop) if op['id'] not in before), None)


def select_value(desktop, label, value):
    desktop.script('''
        const e=document.querySelector('select[aria-label='+JSON.stringify(arguments[0])+']');
        if(!e)throw new Error('Remote control missing');e.value=arguments[1];
        e.dispatchEvent(new Event('change',{bubbles:true}));
    ''', label, value)


def sync_action(desktop, fixture, kind, tag_name=None):
    prompts = set()
    click_text(desktop, 'Pull (FF only)…' if kind == 'pull' else 'Push…')
    wait_for(lambda: desktop.script('return !!document.querySelector(".remote-action-dialog")'),
             'current remote synchronization form', timeout=90)
    if kind in ('remoteTagPush', 'remoteTagDelete'):
        select_value(desktop, 'Remote action', kind)
        if kind == 'remoteTagPush':
            select_value(desktop, 'Source tag', 'refs/tags/' + tag_name)
    if kind != 'pull':
        before = {op['id'] for op in current_operations(desktop)}
        click_text(desktop, 'Read remote references')
        wait_for(lambda: new_operation(desktop, before), 'authenticated reference read queued')
        read, observed = wait_operation(desktop, new_operation(desktop, before)['id'], fixture)
        prompts.update(observed)
        assert read['actionKind'] == 'readRemoteRefs'
        wait_for(lambda: desktop.script('''
            return document.querySelector('.remote-action-dialog')?.textContent.includes('Remote observation:');
        '''), 'actual remote destination snapshot', timeout=90)
        if kind == 'remoteTagDelete':
            select_value(desktop, 'Destination', 'ref:refs/tags/' + tag_name)
    before = {op['id'] for op in current_operations(desktop)}
    click_text(desktop, 'Prepare')
    wait_for(lambda: desktop.script('''return !!document.querySelector('[aria-label="Remote action preview"]')'''),
             'prepared remote mutation preview', timeout=90)
    if kind == 'remoteTagDelete':
        raw = fixture._git('-C', str(fixture.repository), 'rev-parse', 'refs/tags/' + tag_name).stdout.decode().strip()
        assert desktop.script('''
            const l=[...document.querySelectorAll('.remote-action-dialog label')].find(e=>e.textContent.includes('Confirm remote tag deletion'));
            if(!l||!l.textContent.includes(arguments[0])||!l.textContent.includes(arguments[1]))return false;
            l.querySelector('input[type=checkbox]').click();return true;
        ''', 'refs/tags/' + tag_name, raw), 'Exact observed remote tag deletion confirmation missing'
    click_text(desktop, 'Execute')
    wait_for(lambda: new_operation(desktop, before), 'remote mutation queued')
    operation, observed = wait_operation(desktop, new_operation(desktop, before)['id'], fixture)
    prompts.update(observed)
    assert operation['actionKind'] == kind and operation['outcome']['effects'] == 'applied'
    wait_for(lambda: desktop.script('''
        const t=document.querySelector('article[data-operation-id='+JSON.stringify(arguments[0])+']');
        return t?.querySelector('.task-state')?.textContent==='Succeeded';
    ''', operation['id']), 'matching successful remote task')
    print('PASS: native authenticated ' + kind, flush=True)
    return prompts


if __name__ == '__main__':
    executable = Path(os.environ.get('OPENGIT_E2E_EXECUTABLE',
                                   str(ROOT / 'src-tauri/target/debug/opengit.exe')))
    with LiveRemotes(ROOT, writable=True) as fixture:
        ssh, agent_env = fixture.start_agent()
        from urllib.parse import urlsplit
        port = urlsplit(fixture.ssh_url).port
        public = fixture.server_public_key.export_public_key('openssh').decode().strip()
        fixture.known_hosts.write_text(f'[127.0.0.1]:{port} {public}\n', encoding='ascii')
        ssh_command = (f'"{ssh.as_posix()}" -F /dev/null -o '
                       f'UserKnownHostsFile="{fixture.known_hosts.as_posix()}" '
                       '-o GlobalKnownHostsFile=/dev/null -o StrictHostKeyChecking=yes '
                       '-o IdentityFile=none -o PreferredAuthentications=publickey')
        fixture._git('-C', str(fixture.source), 'config', 'user.name', 'OpenGit Fixture')
        fixture._git('-C', str(fixture.source), 'config', 'user.email', 'fixture@invalid.example')
        fixture._git('-C', str(fixture.source), 'remote', 'add', 'origin', fixture.https_url)
        fixture._git('-C', str(fixture.source), 'config', 'branch.main.remote', 'origin')
        fixture._git('-C', str(fixture.source), 'config', 'branch.main.merge', 'refs/heads/main')
        initial = fixture._git('-C', str(fixture.repository), 'rev-parse', 'main').stdout.strip()
        fixture._git('-C', str(fixture.source), 'update-ref', 'refs/remotes/origin/main', initial.decode())
        peer = fixture.root / 'peer'
        fixture._git('clone', '--', str(fixture.repository), str(peer))
        contents = b'Native staged, committed and pushed object\n'
        (fixture.source / 'sync-new.txt').write_bytes(contents)
        data = fixture.root / 'sync-native-data'
        env = {'OPENGIT_DATA_DIR': str(data), 'GIT_SSL_NO_VERIFY': None,
               'GIT_CONFIG_GLOBAL': os.devnull, 'GIT_CONFIG_SYSTEM': os.devnull,
               'GIT_CONFIG_PARAMETERS': None, 'GIT_CONFIG_COUNT': '3',
               'GIT_CONFIG_KEY_0': 'credential.helper', 'GIT_CONFIG_VALUE_0': '',
               'GIT_CONFIG_KEY_1': 'http.sslCAInfo', 'GIT_CONFIG_VALUE_1': str(fixture.ca_file),
               'GIT_CONFIG_KEY_2': 'http.schannelUseSSLCAInfo', 'GIT_CONFIG_VALUE_2': 'true',
               'GIT_SSH_COMMAND': ssh_command, 'GIT_SSH_VARIANT': 'ssh',
               'SSH_AUTH_SOCK': agent_env['SSH_AUTH_SOCK'],
               'OPENGIT_E2E_DIALOGS_JSON': json.dumps([str(fixture.source)])}
        with DesktopSession(ROOT, executable, env) as desktop:
            wait_for(lambda: body_contains(desktop, 'Your repositories'), 'workspace')
            click_text(desktop, 'Open repository')
            click_text(desktop, 'Changes')
            wait_for(lambda: body_contains(desktop, 'sync-new.txt'), 'actual working file', timeout=90)
            file_action(desktop, 'sync-new.txt', 'unstaged')
            commit_form(desktop, 'Native sync workflow')
            execute_form(desktop)
            local = fixture._git('-C', str(fixture.source), 'rev-parse', 'HEAD').stdout.strip()
            assert local != initial
            assert {'username', 'password'}.issubset(sync_action(desktop, fixture, 'push'))
            assert fixture._git('-C', str(fixture.repository), 'rev-parse', 'main').stdout.strip() == local
            assert fixture._git('-C', str(fixture.repository), 'show', 'main:sync-new.txt').stdout == contents
            fixture._git('-C', str(peer), 'pull', '--ff-only')
            remote = advance(fixture, peer, 'fast-forward-pull')
            sync_action(desktop, fixture, 'pull')
            assert fixture._git('-C', str(fixture.source), 'rev-parse', 'HEAD').stdout.strip() == remote
            assert (fixture.source / 'remote-change.txt').read_bytes()
            assert (fixture.source / 'sync-new.txt').read_bytes() == contents
            assert fixture._git('-C', str(fixture.source), 'status', '--porcelain=v2', '-z').stdout == b''
            fixture._git('-C', str(fixture.source), 'remote', 'set-url', 'origin', fixture.ssh_url)
            (fixture.source / 'ssh-push.txt').write_bytes(b'Owned new SSH pushed object\n')
            fixture._git('-C', str(fixture.source), 'add', '--', 'ssh-push.txt')
            fixture._git('-C', str(fixture.source), 'commit', '-m', 'Owned SSH transport setup')
            ssh_oid = fixture._git('-C', str(fixture.source), 'rev-parse', 'HEAD').stdout.strip()
            click_text(desktop, 'Refresh sync')
            assert not sync_action(desktop, fixture, 'push'), 'Pinned owned SSH agent unexpectedly requested a GUI secret'
            assert fixture._git('-C', str(fixture.repository), 'rev-parse', 'main').stdout.strip() == ssh_oid
            assert fixture._git('-C', str(fixture.repository), 'show', 'main:ssh-push.txt').stdout == b'Owned new SSH pushed object\n'
            assert fixture._agent.poll() is None
            print('PASS: native pinned-host SSH agent push transfers exact new object', flush=True)
            tag_name = 'native-remote-tag'
            fixture._git('-C', str(fixture.source), 'tag', '-a', tag_name, '-m', 'Owned annotated tag')
            tag_oid = fixture._git('-C', str(fixture.source), 'rev-parse', 'refs/tags/' + tag_name).stdout.strip()
            click_text(desktop, 'Refresh sync')
            assert not sync_action(desktop, fixture, 'remoteTagPush', tag_name)
            assert fixture._git('-C', str(fixture.repository), 'rev-parse', 'refs/tags/' + tag_name).stdout.strip() == tag_oid
            fixture._git('-C', str(fixture.source), 'tag', '-d', tag_name)
            click_text(desktop, 'Refresh sync')
            assert not sync_action(desktop, fixture, 'remoteTagDelete', tag_name)
            assert fixture._git('-C', str(fixture.repository), 'for-each-ref', '--format=%(refname)', 'refs/tags/' + tag_name).stdout == b''
            assert fixture._git('-C', str(fixture.repository), 'rev-parse', 'main').stdout.strip() == ssh_oid
            print('PASS: native raw annotated tag push and remote-only conditional deletion preserve branch', flush=True)
            profile = desktop.directory
            print('Screenshot:', desktop.screenshot('sync-workflow.png'), flush=True)
        assert_no_credentials(data, fixture)
        assert_profile_has_no_credentials(profile, fixture)
        print('PASS: exact server objects/FF HEAD/working bytes and private credential scans', flush=True)
