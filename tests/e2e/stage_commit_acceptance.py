"""Native whole-file Stage/Unstage -> rejected hook -> successful real Commit."""
import json
import os
from pathlib import Path
import sys

from clone_acceptance import value
from daily_context_acceptance import ContextSession
from workspace_acceptance import body_contains, click_text, wait_for

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'tests/e2e/support'))
from daily_fixtures import DailyFixture


def operations(desktop):
    return value(desktop.invoke('get_operations', {}))['operations']


def execute_form(desktop, *, success=True):
    before = {op['id'] for op in operations(desktop)}
    click_text(desktop, 'Prepare')
    wait_for(lambda: desktop.script('''
        const d=document.querySelector('[role=dialog]');
        return !!d&&d.textContent.includes('Prepared ')
            &&[...d.querySelectorAll('button')].some(b=>b.textContent==='Execute'&&!b.disabled);
    '''), 'actual prepared changes action', timeout=90)
    click_text(desktop, 'Execute')
    result = None
    def ended():
        nonlocal result
        result = next((op for op in operations(desktop) if op['id'] not in before), None)
        return result and result['state'] in ('Succeeded', 'Failed', 'Interrupted', 'Cancelled')
    wait_for(ended, 'real changes task terminal result', timeout=150)
    assert (result['state'] == 'Succeeded') is success
    if success:
        assert result['outcome']['effects'] == 'applied'
    wait_for(lambda: desktop.script('''
        const t=document.querySelector('article[data-operation-id='+JSON.stringify(arguments[0])+']');
        return t?.querySelector('.task-state')?.textContent===arguments[1];
    ''', result['id'], result['state']), 'matching visible task result')
    wait_for(lambda: desktop.script('return !document.querySelector("[role=dialog]")'), 'changes form closed')
    print('PASS: native ' + result['actionKind'] + ' ' + result['state'], flush=True)
    return result


def file_action(desktop, name, side):
    label = 'Staged changes' if side == 'staged' else 'Unstaged changes'
    button = 'Unstage file' if side == 'staged' else 'Stage file'
    wait_for(lambda: desktop.script('''
        const s=document.querySelector('[aria-label='+JSON.stringify(arguments[0])+']');
        const row=[...s?.querySelectorAll('li')??[]].find(li=>li.querySelector('.change-path')?.textContent.split(' → ').at(-1)===arguments[1]);
        const b=[...row?.querySelectorAll('button')??[]].find(e=>e.textContent===arguments[2]&&!e.disabled);
        if(!b)return false;b.click();return true;
    ''', label, name, button), 'current file action ' + name, timeout=90)
    return execute_form(desktop)


def commit_form(desktop, message=None):
    click_text(desktop, 'Commit staged changes')
    wait_for(lambda: desktop.script('''return !!document.querySelector('textarea[aria-label="Commit message"]')'''),
             'current commit form', timeout=90)
    if message is not None:
        desktop.fill('textarea[aria-label="Commit message"]', message)


def rename_only(executable):
    with DailyFixture(ROOT) as fixture:
        base = b''.join(('base line ' + str(i) + '\n').encode() for i in range(10))
        edited, recreated = base + b'new edit\n', b'unrelated recreated old file\n'
        (fixture.source / 'rename-old.txt').write_bytes(base)
        fixture.run('add', '--', 'rename-old.txt')
        fixture.run('commit', '-m', 'Owned rename base')
        fixture.run('mv', '--', 'rename-old.txt', 'rename-new.txt')
        (fixture.source / 'rename-new.txt').write_bytes(edited)
        (fixture.source / 'rename-old.txt').write_bytes(recreated)
        (fixture.source / 'keep-staged.txt').write_bytes(b'keep staged\n')
        fixture.run('add', '--', 'keep-staged.txt')
        keep = fixture.run('rev-parse', ':keep-staged.txt')
        env = {'OPENGIT_DATA_DIR': str(fixture.root / 'rename-data'),
               'GIT_CONFIG_GLOBAL': os.devnull, 'GIT_CONFIG_SYSTEM': os.devnull,
               'OPENGIT_E2E_DIALOGS_JSON': json.dumps([str(fixture.source)])}
        with ContextSession(ROOT, executable, env) as desktop:
            wait_for(lambda: body_contains(desktop, 'Your repositories'), 'workspace')
            click_text(desktop, 'Open repository')
            click_text(desktop, 'Changes')
            wait_for(lambda: body_contains(desktop, 'rename-new.txt'), 'staged rename with unstaged edit', timeout=90)
            file_action(desktop, 'rename-new.txt', 'unstaged')
            assert fixture.run('ls-files', '--', 'rename-old.txt') == b''
            assert fixture.run('show', ':rename-new.txt') == edited
            assert fixture.run('rev-parse', ':keep-staged.txt') == keep
            assert (fixture.source / 'rename-old.txt').read_bytes() == recreated
            status = fixture.run('status', '--porcelain=v2', '-z', '--untracked-files=all')
            assert any(row.startswith(b'2 R') and b'rename-new.txt' in row for row in status.split(b'\0')), \
                'Unstage-pair case requires a Git-classified staged rename'
            file_action(desktop, 'rename-new.txt', 'staged')
            assert fixture.run('show', ':rename-old.txt') == base
            assert fixture.run('ls-files', '--', 'rename-new.txt') == b''
            assert fixture.run('rev-parse', ':keep-staged.txt') == keep
            assert (fixture.source / 'rename-old.txt').read_bytes() == recreated
            assert (fixture.source / 'rename-new.txt').read_bytes() == edited
            print('PASS: native rename-side selection preserves recreated old file and unrelated index/working bytes', flush=True)
            print('Screenshot:', desktop.screenshot('rename-side.png'), flush=True)


if __name__ == '__main__':
    executable = Path(os.environ.get('OPENGIT_E2E_EXECUTABLE',
                                   str(ROOT / 'src-tauri/target/debug/opengit.exe')))
    if '--rename-only' in sys.argv:
        rename_only(executable)
        raise SystemExit(0)
    with DailyFixture(ROOT) as fixture:
        head_before = fixture.run('rev-parse', 'HEAD').strip()
        readme_before = fixture.run('show', 'HEAD:README.txt')
        (fixture.source / 'README.txt').write_bytes(b'Unstaged working edit stays in place\n')
        (fixture.source / 'unrelated-staged.txt').write_bytes(b'Previously staged entry\n')
        fixture.run('add', '--', 'unrelated-staged.txt')
        unrelated_oid = fixture.run('rev-parse', ':unrelated-staged.txt')
        first = b'Native staged snapshot\n'
        later = b'Later unstaged working content\n'
        (fixture.source / 'native-new.txt').write_bytes(first)
        hook = fixture.source / '.git/hooks/pre-commit'
        hook.write_text('#!/bin/sh\nprintf ran > .git/native-hook-ran\nif test -f .git/native-reject; then exit 1; fi\n', encoding='utf-8', newline='\n')
        hook.chmod(0o700)
        marker = fixture.source / '.git/native-reject'
        marker.write_bytes(b'owned fixture\n')
        data = fixture.root / 'stage-commit-data'
        env = {'OPENGIT_DATA_DIR': str(data), 'GIT_CONFIG_GLOBAL': os.devnull,
               'GIT_CONFIG_SYSTEM': os.devnull,
               'OPENGIT_E2E_DIALOGS_JSON': json.dumps([str(fixture.source)])}
        message = 'Native staged commit'
        with ContextSession(ROOT, executable, env) as desktop:
            wait_for(lambda: body_contains(desktop, 'Your repositories'), 'workspace')
            click_text(desktop, 'Open repository')
            click_text(desktop, 'Changes')
            wait_for(lambda: body_contains(desktop, 'native-new.txt'), 'actual staged/unstaged catalog', timeout=90)
            file_action(desktop, 'native-new.txt', 'unstaged')
            assert fixture.run('show', ':native-new.txt') == first
            assert fixture.run('rev-parse', ':unrelated-staged.txt') == unrelated_oid
            file_action(desktop, 'native-new.txt', 'staged')
            assert fixture.run('ls-files', '--', 'native-new.txt') == b''
            assert (fixture.source / 'native-new.txt').read_bytes() == first
            assert fixture.run('rev-parse', ':unrelated-staged.txt') == unrelated_oid
            file_action(desktop, 'native-new.txt', 'unstaged')
            (fixture.source / 'native-new.txt').write_bytes(later)
            commit_form(desktop, message)
            execute_form(desktop, success=False)
            assert (fixture.source / '.git/native-hook-ran').is_file(), 'Configured pre-commit hook did not run'
            assert fixture.run('rev-parse', 'HEAD').strip() == head_before
            assert fixture.run('show', ':native-new.txt') == first
            marker.unlink()
            commit_form(desktop)
            assert desktop.script('''return document.querySelector('textarea[aria-label="Commit message"]').value''') == message
            execute_form(desktop)
            head_after = fixture.run('rev-parse', 'HEAD').strip()
            assert head_after != head_before and fixture.run('rev-parse', 'HEAD^').strip() == head_before
            assert fixture.run('show', 'HEAD:native-new.txt') == first
            assert fixture.run('show', 'HEAD:unrelated-staged.txt') == b'Previously staged entry\n'
            assert fixture.run('show', 'HEAD:README.txt') == readme_before
            assert (fixture.source / 'native-new.txt').read_bytes() == later
            assert (fixture.source / 'README.txt').read_bytes() == b'Unstaged working edit stays in place\n'
            assert fixture.run('diff', '--cached', '--name-only') == b''
            wait_for(lambda: desktop.script('''
                const s=document.querySelector('[aria-label="Staged changes"]');
                return !!s&&!s.querySelector('li')&&!document.querySelector('.changes-workspace [role=alert]');
            '''), 'fresh unstaged/staged UI after successful commit', timeout=90)
            print('PASS: exact staged tree/parent, working edits and failed-hook draft preserved', flush=True)
            print('Screenshot:', desktop.screenshot('stage-commit.png'), flush=True)
        for path in data.rglob('*'):
            if path.is_file():
                assert message.encode() not in path.read_bytes(), 'Commit message persisted outside Git'
