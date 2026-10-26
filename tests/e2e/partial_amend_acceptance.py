"""Native selected lines/hunks -> exact index -> commit -> message-only amend.

Git commands only prepare or inspect the disposable fixture. Every action being
accepted runs through real renderer controls, prepare/execute and Tasks.
"""
import json
import os
from pathlib import Path
import sys
import time

from daily_context_acceptance import ContextSession
from stage_commit_acceptance import commit_form, execute_form
from workspace_acceptance import body_contains, click_text, wait_for

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'tests/e2e/support'))
from daily_fixtures import DailyFixture


def inspect_file(desktop, name, side, expected_text):
    started = time.monotonic()
    section = 'Staged changes' if side == 'staged' else 'Unstaged changes'
    wait_for(lambda: desktop.script('''
        const s=document.querySelector('[aria-label='+JSON.stringify(arguments[0])+']');
        const b=[...s?.querySelectorAll('.change-path')??[]].find(b=>b.textContent===arguments[1]);
        if(!b||b.disabled)return false;b.click();return true;
    ''', section, name), 'inspect current ' + side + ' file', timeout=90)
    print('Inspecting ' + side + ' diff', flush=True)
    last = None
    def loaded():
        nonlocal last
        state = desktop.script('''
        const d=document.querySelector('[aria-label="Change diff"]');
        return {loaded:!!d&&d.textContent.includes(arguments[0])&&!!d.querySelector('input[type=checkbox]'),
            loading:d?.textContent.includes('Loading diff'),empty:d?.textContent.includes('Select a staged'),
            changesLoading:!!document.querySelector('.changes-workspace')?.textContent.includes('Loading changes'),
            alerts:[...document.querySelectorAll('.changes-workspace [role=alert]')].map(e=>e.textContent)};
    ''', expected_text)
        if state != last:
            print('Diff state %.1fs: %s' % (time.monotonic() - started, json.dumps(state)), flush=True)
            last = state
        return state['loaded']
    wait_for(loaded, 'eligible structured diff', timeout=90)


def check_control(desktop, label):
    wait_for(lambda: desktop.script('''
        const d=document.querySelector('[aria-label="Change diff"]');
        const c=[...d?.querySelectorAll('input[type=checkbox]')??[]]
            .find(c=>c.getAttribute('aria-label')===arguments[0]);
        if(!c||c.disabled)return false;if(!c.checked)c.click();return c.checked;
    ''', label), 'select visible edit ' + label)


def partial_action(desktop, side):
    click_text(desktop, 'Unstage selected' if side == 'staged' else 'Stage selected')
    return execute_form(desktop)


def run(executable):
    with DailyFixture(ROOT) as fixture:
        fixture.run('config', 'core.autocrlf', 'false')
        name = 'partial-native.txt'
        middle = b''.join(('context %02d\n' % i).encode() for i in range(1, 16))
        baseline = b'first\nold-one\n' + middle + b'tail-old\nlast\n'
        working = b'first\nnew-one\nnew-two\n' + middle + b'tail-new\nlast\n'
        first_only = b'first\nnew-one\n' + middle + b'tail-old\nlast\n'
        first_hunk = b'first\nnew-one\nnew-two\n' + middle + b'tail-old\nlast\n'
        (fixture.source / name).write_bytes(baseline)
        fixture.run('add', '--', name)
        fixture.run('commit', '-m', 'Owned partial base')
        original_head = fixture.run('rev-parse', 'HEAD').strip()
        (fixture.source / name).write_bytes(working)
        (fixture.source / 'unrelated-staged.txt').write_bytes(b'preserve staged entry\n')
        fixture.run('add', '--', 'unrelated-staged.txt')
        unrelated = fixture.run('rev-parse', ':unrelated-staged.txt')
        data = fixture.root / 'partial-amend-data'
        env = {'OPENGIT_DATA_DIR': str(data), 'GIT_CONFIG_GLOBAL': os.devnull,
               'GIT_CONFIG_SYSTEM': os.devnull,
               'OPENGIT_E2E_DIALOGS_JSON': json.dumps([str(fixture.source)])}
        message = 'Native partial commit'
        amend_message = 'Native message-only amend'
        with ContextSession(ROOT, executable, env) as desktop:
            wait_for(lambda: body_contains(desktop, 'Your repositories'), 'workspace')
            click_text(desktop, 'Open repository')
            click_text(desktop, 'Changes')
            inspect_file(desktop, name, 'unstaged', 'new-one')
            check_control(desktop, 'Select deleted line 2')
            check_control(desktop, 'Select added line 2')
            # Cross a real normal fallback tick while the inline selection is
            # active. This is not a changed timer or a mocked refresh result.
            time.sleep(31)
            assert desktop.script('''
                const d=document.querySelector('[aria-label="Change diff"]');
                return ['Select deleted line 2','Select added line 2'].every(label=>
                    d?.querySelector('input[aria-label='+JSON.stringify(label)+']')?.checked);
            '''), 'A passive refresh discarded the explicit line selection'
            print('PASS: inline line selection survives a real 30-second fallback tick', flush=True)
            partial_action(desktop, 'unstaged')
            assert fixture.run('show', ':' + name) == first_only
            assert fixture.run('rev-parse', ':unrelated-staged.txt') == unrelated
            assert (fixture.source / name).read_bytes() == working

            inspect_file(desktop, name, 'staged', 'new-one')
            check_control(desktop, 'Select deleted line 2')
            check_control(desktop, 'Select added line 2')
            partial_action(desktop, 'staged')
            assert fixture.run('show', ':' + name) == baseline
            assert fixture.run('rev-parse', ':unrelated-staged.txt') == unrelated
            assert (fixture.source / name).read_bytes() == working

            inspect_file(desktop, name, 'unstaged', 'new-two')
            check_control(desktop, 'Select hunk 1')
            partial_action(desktop, 'unstaged')
            assert fixture.run('show', ':' + name) == first_hunk
            assert fixture.run('rev-parse', ':unrelated-staged.txt') == unrelated
            assert (fixture.source / name).read_bytes() == working
            print('PASS: exact selected line/unstage/hunk blobs; unrelated index and working bytes preserved', flush=True)

            commit_form(desktop, message)
            execute_form(desktop)
            committed = fixture.run('rev-parse', 'HEAD').strip()
            assert committed != original_head
            assert fixture.run('rev-parse', 'HEAD^').strip() == original_head
            assert fixture.run('show', 'HEAD:' + name) == first_hunk
            assert fixture.run('show', 'HEAD:unrelated-staged.txt') == b'preserve staged entry\n'
            assert fixture.run('diff', '--cached', '--name-only') == b''
            tree = fixture.run('rev-parse', 'HEAD^{tree}')
            author = fixture.run('show', '-s', '--format=%an%x00%ae%x00%aI', 'HEAD')

            # No staged changes: opening and explicitly choosing amend must work.
            commit_form(desktop, amend_message)
            wait_for(lambda: desktop.script('''
                const d=document.querySelector('[role=dialog]');
                const c=d?.querySelector('input[aria-label="Amend last commit"]');
                if(!c||c.disabled)return false;
                if(!c.checked)c.click();return c.checked;
            '''), 'explicit message-only amend')
            execute_form(desktop)
            amended = fixture.run('rev-parse', 'HEAD').strip()
            assert amended != committed
            assert fixture.run('rev-parse', 'HEAD^').strip() == original_head
            assert fixture.run('rev-parse', 'HEAD^{tree}') == tree
            assert fixture.run('show', '-s', '--format=%an%x00%ae%x00%aI', 'HEAD') == author
            assert fixture.run('show', '-s', '--format=%B', 'HEAD').strip() == amend_message.encode()
            assert fixture.run('diff', '--cached', '--name-only') == b''
            assert (fixture.source / name).read_bytes() == working
            print('PASS: actual commit then explicit message-only amend preserves tree, original parent, author and unstaged bytes', flush=True)
            print('Screenshot:', desktop.screenshot('partial-amend.png'), flush=True)
        for path in data.rglob('*'):
            if path.is_file():
                content = path.read_bytes()
                assert message.encode() not in content and amend_message.encode() not in content, \
                    'Commit message persisted outside Git'


if __name__ == '__main__':
    run(Path(os.environ.get('OPENGIT_E2E_EXECUTABLE',
                            str(ROOT / 'src-tauri/target/debug/opengit.exe'))))
