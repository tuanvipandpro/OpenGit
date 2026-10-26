"""Proposed native Save/Apply -> two conflicts -> per-file Stage -> Reconcile.

Prepared Git fixtures and independent blob/index oracles, no mocked app results.
UI labels follow the S6 packet; acceptance is claimed only after an actual run.
"""
import json
import os
from pathlib import Path
import sys

from daily_context_acceptance import ContextSession
from stage_commit_acceptance import file_action, operations
from workspace_acceptance import body_contains, click_text, wait_for

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'tests/e2e/support'))
from daily_fixtures import DailyFixture


def close_dialogs(desktop):
    for _ in range(2):
        closed = desktop.script('''
            const d=[...document.querySelectorAll('dialog[open], [role=dialog]')].at(-1);
            const b=[...d?.querySelectorAll('button')??[]].find(b=>b.textContent==='Close'&&!b.disabled);
            if(!b)return false;b.click();return true;
        ''')
        if not closed:
            break
    wait_for(lambda: desktop.script('return !document.querySelector("dialog[open], [role=dialog]")'), 'stash dialogs closed')


def execute_stash(desktop, *, success=True):
    before = {op['id'] for op in operations(desktop)}
    print('Preparing native stash action', flush=True)
    click_text(desktop, 'Prepare')
    wait_for(lambda: desktop.script('''
        const d=document.querySelector('dialog[open], [role=dialog]');
        return d?.textContent.includes('Prepared ')&&[...d.querySelectorAll('button')]
            .some(b=>b.textContent==='Execute'&&!b.disabled);
    '''), 'prepared native stash action', timeout=90)
    click_text(desktop, 'Execute')
    print('Waiting for native stash task', flush=True)
    result = None
    def ended():
        nonlocal result
        result = next((op for op in operations(desktop) if op['id'] not in before), None)
        return result and result['state'] in ('Succeeded', 'Failed', 'Interrupted', 'Cancelled')
    wait_for(ended, 'stash task terminal', timeout=180)
    assert (result['state'] == 'Succeeded') is success
    wait_for(lambda: desktop.script('''
        const t=document.querySelector('article[data-operation-id='+JSON.stringify(arguments[0])+']');
        return t?.querySelector('.task-state')?.textContent===arguments[1];
    ''', result['id'], result['state']), 'matching visible stash task')
    wait_for(lambda: desktop.script('''
        const t=document.querySelector('article[data-operation-id='+JSON.stringify(arguments[0])+']');
        const e=t?.querySelector('[aria-label="Stash action evidence"]');
        return e?.textContent.includes('Stash object in stack: present.')&&
            (arguments[1]||e.textContent.includes('Recovery pin retention: present.')&&
                e.textContent.includes('Unmerged paths remain.'));
    ''', result['id'], success), 'visible retained stash and conflict evidence', timeout=90)
    close_dialogs(desktop)
    print('PASS: native ' + result['actionKind'] + ' ' + result['state'], flush=True)
    return result


def open_stash(desktop):
    wait_for(lambda: desktop.script('''
        const b=[...document.querySelectorAll('button')].find(b=>b.textContent==='Stash'&&!b.disabled);
        if(!b)return false;b.click();return true;
    '''), 'current published repository enables Stash', timeout=90)


def save(desktop, message, *, untracked=False):
    open_stash(desktop)
    wait_for(lambda: body_contains(desktop, 'Save stash'), 'stash manager')
    click_text(desktop, 'Save stash')
    wait_for(lambda: desktop.script("""return !!document.querySelector('textarea[aria-label="Stash message"]')"""), 'save form')
    desktop.fill('textarea[aria-label="Stash message"]', message)
    assert desktop.script("""return !document.querySelector('input[aria-label="Include untracked files"]').checked"""), 'Untracked default must be off'
    if untracked:
        desktop.script("""document.querySelector('input[aria-label="Include untracked files"]').click()""")
    return execute_stash(desktop)


def apply_latest(desktop, *, success=True):
    open_stash(desktop)
    wait_for(lambda: desktop.script('''
        const d=document.querySelector('dialog[open], [role=dialog]');
        const b=[...d?.querySelectorAll('button')??[]].find(b=>b.textContent==='Apply stash'&&!b.disabled);
        if(!b)return false;b.click();return true;
    '''), 'apply newest visible stash', timeout=90)
    return execute_stash(desktop, success=success)


def index_stages(fixture, path):
    return fixture.run('ls-files', '--stage', '-z', '--', path)


def run(executable):
    with DailyFixture(ROOT) as fixture:
        fixture.run('config', 'core.autocrlf', 'false')
        # Apply must follow the explicit UI promise even if ambient Git would
        # otherwise restore the original staged partition.
        fixture.run('config', 'stash.index', 'true')
        # Retain the fixture's DAG/remotes, but give this daily workflow a small
        # regular-file HEAD. Graph/binary/symlink coverage has separate evidence.
        tracked = [path.decode('utf-8') for path in fixture.run('ls-files', '-z').split(b'\0') if path]
        fixture.run('rm', '--', *tracked)
        for name, data in [('alpha.txt', b'alpha base\n'), ('beta.txt', b'beta base\n'), ('.gitignore', b'ignored-cache.tmp\n')]:
            (fixture.source / name).write_bytes(data)
            fixture.run('add', '--', name)
        fixture.run('commit', '-m', 'Owned stash base')
        original_head = fixture.run('rev-parse', 'HEAD')
        (fixture.source / 'alpha.txt').write_bytes(b'alpha staged\n')
        fixture.run('add', '--', 'alpha.txt')
        (fixture.source / 'alpha.txt').write_bytes(b'alpha stash\n')
        (fixture.source / 'beta.txt').write_bytes(b'beta stash\n')
        (fixture.source / 'loose.txt').write_bytes(b'owned untracked\n')
        (fixture.source / 'ignored-cache.tmp').write_bytes(b'owned ignored\n')
        data = fixture.root / 'stash-data'
        env = {'OPENGIT_DATA_DIR': str(data), 'GIT_CONFIG_GLOBAL': os.devnull,
               'GIT_CONFIG_SYSTEM': os.devnull,
               'OPENGIT_E2E_DIALOGS_JSON': json.dumps([str(fixture.source)])}
        messages = ['Native Save including untracked', 'Native Save tracked only']
        with ContextSession(ROOT, executable, env) as desktop:
            wait_for(lambda: body_contains(desktop, 'Your repositories'), 'workspace')
            click_text(desktop, 'Open repository')
            save(desktop, messages[0], untracked=True)
            first = fixture.run('rev-parse', 'refs/stash').strip()
            assert fixture.run('rev-parse', 'HEAD') == original_head
            assert fixture.run('show', 'refs/stash:alpha.txt') == b'alpha stash\n'
            assert fixture.run('show', 'refs/stash^2:alpha.txt') == b'alpha staged\n'
            assert fixture.run('show', 'refs/stash^3:loose.txt') == b'owned untracked\n'
            assert b'ignored-cache.tmp' not in fixture.run('ls-tree', '-r', '--name-only', 'refs/stash^3')
            assert (fixture.source / 'alpha.txt').read_bytes() == b'alpha base\n'
            assert not (fixture.source / 'loose.txt').exists()
            assert (fixture.source / 'ignored-cache.tmp').read_bytes() == b'owned ignored\n'

            apply_latest(desktop)
            assert fixture.run('rev-parse', 'refs/stash').strip() == first
            assert (fixture.source / 'alpha.txt').read_bytes() == b'alpha stash\n'
            assert (fixture.source / 'beta.txt').read_bytes() == b'beta stash\n'
            assert (fixture.source / 'loose.txt').read_bytes() == b'owned untracked\n'
            assert fixture.run('diff', '--cached', '--name-only') == b''

            save(desktop, messages[1])
            selected = fixture.run('rev-parse', 'refs/stash').strip()
            assert selected != first
            assert (fixture.source / 'loose.txt').read_bytes() == b'owned untracked\n'
            assert len(fixture.run('rev-list', '--parents', '-n1', 'refs/stash').split()) == 3
            for name in ('alpha', 'beta'):
                (fixture.source / (name + '.txt')).write_bytes((name + ' other\n').encode())
                fixture.run('add', '--', name + '.txt')
            fixture.run('commit', '-m', 'Owned external conflict destination')
            conflict_head = fixture.run('rev-parse', 'HEAD')
            conflict = apply_latest(desktop, success=False)
            assert not (fixture.source / '.git/MERGE_HEAD').exists()
            for name in ('alpha.txt', 'beta.txt'):
                assert len(index_stages(fixture, name).split(b'\0')) == 4
            assert fixture.run('rev-parse', 'refs/stash').strip() == selected
            assert selected in fixture.run('for-each-ref', '--format=%(objectname)', 'refs/opengit/recovery/').splitlines()
            beta_stages = index_stages(fixture, 'beta.txt')
            click_text(desktop, 'Changes')
            (fixture.source / 'alpha.txt').write_bytes(b'alpha resolved\n')
            file_action(desktop, 'alpha.txt', 'unstaged')
            assert fixture.run('show', ':alpha.txt') == b'alpha resolved\n'
            assert index_stages(fixture, 'beta.txt') == beta_stages
            (fixture.source / 'beta.txt').write_bytes(b'beta resolved\n')
            file_action(desktop, 'beta.txt', 'unstaged')
            assert fixture.run('ls-files', '--unmerged', '-z') == b''
            before_index = (fixture.source / '.git/index').read_bytes()
            before_ops = {op['id'] for op in operations(desktop)}
            wait_for(lambda: desktop.script('''
                const t=document.querySelector('article[data-operation-id='+JSON.stringify(arguments[0])+']');
                const b=[...t?.querySelectorAll('button')??[]].find(b=>b.textContent==='Reconcile stash'&&!b.disabled);
                if(!b)return false;b.click();return true;
            ''', conflict['id']), 'reconcile conflicted stash task')
            wait_for(lambda: desktop.script('''
                const t=document.querySelector('article[data-operation-id='+JSON.stringify(arguments[0])+']');
                return t?.textContent.toLowerCase().includes('resolved externally');
            ''', conflict['id']), 'resolved externally evidence', timeout=90)
            assert fixture.run('rev-parse', 'HEAD') == conflict_head
            assert (fixture.source / '.git/index').read_bytes() == before_index
            assert {op['id'] for op in operations(desktop)} == before_ops
            assert fixture.run('rev-parse', 'refs/stash').strip() == selected
            assert (fixture.source / 'loose.txt').read_bytes() == b'owned untracked\n'
            assert (fixture.source / 'ignored-cache.tmp').read_bytes() == b'owned ignored\n'
            print('PASS: Save/Apply retains stash, preserves ignored/untracked data; two-file conflict staged independently and read-only reconciled', flush=True)
            print('Screenshot:', desktop.screenshot('stash-conflict.png'), flush=True)
        for path in data.rglob('*'):
            if path.is_file():
                content = path.read_bytes()
                assert all(message.encode() not in content for message in messages), 'Stash message persisted outside Git'


if __name__ == '__main__':
    run(Path(os.environ.get('OPENGIT_E2E_EXECUTABLE', str(ROOT / 'src-tauri/target/debug/opengit.exe'))))
