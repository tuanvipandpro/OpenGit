"""Native S3 context/fetch acceptance using real UI, IPC, Git and SQLite.

Only the folder chooser is injected. Requires a frozen e2e-enabled executable;
no command result or membership is supplied to the application by this test.
"""
import json
import os
from pathlib import Path
import sys
import time

from desktop_smoke import DesktopSession
from history_inspector_acceptance import select_result
from history_ui_acceptance import action, idle, search
from workspace_acceptance import body_contains, click_text, wait_for

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'tests/e2e/support'))
from daily_fixtures import DailyFixture


class ContextSession(DesktopSession):
    def __exit__(self, kind, error, traceback):
        if kind is not None:
            try:
                print('Failure screenshot:', self.screenshot('daily-context-failure.png'), flush=True)
                (self.directory / 'daily-context-failure.json').write_text(json.dumps({
                    'body': self.script('return document.body.innerText'),
                }), encoding='utf-8')
            except Exception:
                pass
        return super().__exit__(kind, error, traceback)


def membership_oracle(fixture, oid):
    def names(*options):
        raw = fixture.run('for-each-ref', *options, '--format=%(refname)%00%(symref)',
                          'refs/heads/', 'refs/remotes/').decode().splitlines()
        return {name for line in raw for name, alias in [line.split('\0')] if not alias}
    positive = names('--contains=' + oid)
    return {name: 'contains' if name in positive else 'doesNotContain' for name in names()}


def membership(desktop):
    return desktop.script('''
        const p=document.querySelector('[aria-label="Commit branch containment"]');
        if(!p||p.querySelector('[role=alert]')||p.textContent.includes('Checking all')
            ||p.textContent.includes('Previously loaded'))return null;
        const rows=[...p.querySelectorAll('[data-ref-name][data-membership]')];
        return rows.length?Object.fromEntries(rows.map(e=>[e.dataset.refName,e.dataset.membership])):null;
    ''')


def select_and_check(desktop, fixture, subject, oid):
    search(desktop, subject)
    select_result(desktop, subject)
    expected = membership_oracle(fixture, oid)
    wait_for(lambda: membership(desktop) == expected, 'full branch membership oracle: ' + subject,
             timeout=90)
    assert desktop.script('return document.querySelector(".commit-inspector-heading code")?.textContent') == oid
    assert not body_contains(desktop, 'never pushed')
    return expected


def fetch(desktop, prune=False, all_remotes=False, on_started=None,
          hold_seconds=0, refresh_context=False):
    count = desktop.script('''return document.querySelectorAll('article[aria-label="Fetch remotes task"]').length''')
    click_text(desktop, 'Fetch…')
    wait_for(lambda: desktop.script('return !!document.querySelector(".fetch-dialog")'), 'fetch dialog')
    assert desktop.script('return document.querySelector(".fetch-prune input")?.checked') is False
    if all_remotes:
        click_text(desktop, 'Select all remotes')
    else:
        desktop.click('input[aria-label="Fetch origin"]')
    if prune:
        desktop.click('.fetch-prune input')
    if hold_seconds:
        # Real elapsed time: the application's normal fallback timer keeps running.
        time.sleep(hold_seconds)
        assert desktop.script('return !!document.querySelector(".fetch-dialog")'), 'Passive refresh dismissed the form'
        assert desktop.script('return document.querySelector(".fetch-prune input")?.checked') is prune
        assert desktop.script('''return document.querySelector('input[aria-label="Fetch origin"]')?.checked''') is True
    if refresh_context:
        click_text(desktop, 'Refresh context')
        wait_for(lambda: desktop.script('''
            const d=document.querySelector('.fetch-dialog');
            return !!d&&d.textContent.includes('Repository refreshed. Prepare again.');
        '''), 'manual context change preserves form with explicit notice', timeout=90)
        assert desktop.script('return document.querySelector(".fetch-prune input")?.checked') is prune
        assert desktop.script('''return document.querySelector('input[aria-label="Fetch origin"]')?.checked''') is True
    click_text(desktop, 'Prepare fetch')
    wait_for(lambda: desktop.script('''return !!document.querySelector('[aria-label="Fetch preview"]')'''),
             'actual prepared fetch preview', timeout=60)
    assert desktop.script('''return document.querySelector('[aria-label="Fetch preview"]').textContent.includes(arguments[0])''',
                          'Prune enabled' if prune else 'Prune off')
    before_snapshot = desktop.script('''
        const p=document.querySelector('[aria-label="Repository sync"]');
        return p?.querySelector(':scope > small')?.textContent??null;
    ''')
    assert before_snapshot, 'Fetch requires a visible observed repository snapshot'
    click_text(desktop, 'Start fetch')
    if on_started:
        on_started()
    wait_for(lambda: desktop.script('''
        const tasks=[...document.querySelectorAll('article[aria-label="Fetch remotes task"]')];
        return tasks.length===arguments[0]+1&&tasks.every(t=>t.querySelector('.task-state')?.textContent==='Succeeded'
            &&!!t.querySelector('[aria-label="Fetch remote results"]')&&t.textContent.includes('Changes applied'));
    ''', count), 'verified native fetch task and per-remote effects', timeout=150)
    wait_for(lambda: desktop.script('''
        const p=document.querySelector('[aria-label="Repository sync"]');
        return !!p&&!p.querySelector('[role=alert]')&&!p.textContent.includes('Refreshing local')
            &&!p.textContent.includes('Previously loaded')
            &&!!p.querySelector(':scope > small')
            &&p.querySelector(':scope > small').textContent!==arguments[0];
    ''', before_snapshot), 'new observed snapshot from coordinated refresh', timeout=90)


def opening_and_cancel(desktop):
    wait_for(lambda: desktop.script('''
        const p=document.querySelector('[aria-label="Repository sync"]');
        const b=[...p.querySelectorAll('button')].find(e=>e.textContent==='Fetch…');
        return !!b&&!b.disabled&&!p.textContent.includes('Refreshing local');
    '''), 'initial current catalog for quiet-refresh opening', timeout=90)
    result = desktop.command('POST', '/execute/async', {'script': '''
        const done=arguments[arguments.length-1];
        window.dispatchEvent(new Event('focus'));
        // The real coordinator debounces focus for150ms; inspect runs via real IPC/Git.
        setTimeout(()=>{
            const b=[...document.querySelectorAll('[aria-label="Repository sync"] button')].find(e=>e.textContent==='Fetch…');
            const quiet=!document.querySelector('.repository-tabs [role=status]');
            if(!b||b.disabled){done({clicked:false,quiet});return;}
            b.click();setTimeout(()=>done({clicked:true,quiet,
                waiting:document.body.textContent.includes('Waiting for repository refresh before opening Fetch…'),
                form:!!document.querySelector('.fetch-dialog')}),0);
        },160);
    ''', 'args': []})
    assert result == {'clicked': True, 'quiet': True, 'waiting': True, 'form': False}, result
    wait_for(lambda: desktop.script('return !!document.querySelector(".fetch-dialog")'),
             'form opens after real quiet refresh/catalog settle', timeout=90)
    before = desktop.script('''return document.querySelector('[aria-label="Repository sync"]')?.querySelector(':scope > small')?.textContent''')
    assert before
    desktop.script('window.dispatchEvent(new Event("focus"));')
    time.sleep(.25)
    assert desktop.script('return !!document.querySelector(".fetch-dialog")')
    click_text(desktop, 'Cancel')
    wait_for(lambda: desktop.script('''
        const p=document.querySelector('[aria-label="Repository sync"]');
        return !document.querySelector('.fetch-dialog')&&!!p
            &&!p.textContent.includes('Refreshing local')&&!p.textContent.includes('Previously loaded')
            &&p.querySelector('small')?.textContent!==arguments[0];
    ''', before), 'cancel releases pending passive refresh', timeout=90)
    assert desktop.script('''return document.querySelectorAll('article[aria-label="Fetch remotes task"]').length''') == 0
    print('PASS: native opening waits for quiet refresh; cancel releases pending refresh without a Git task', flush=True)


if __name__ == '__main__':
    executable = Path(os.environ.get('OPENGIT_E2E_EXECUTABLE',
                                   str(ROOT / 'src-tauri/target/debug/opengit.exe')))
    with DailyFixture(ROOT) as fixture:
        fixture.run('remote', 'add', 'backup', str(fixture.remote))
        protected = fixture.protected_source()
        env = {'OPENGIT_DATA_DIR': str(fixture.root / 'app-data'),
               'GIT_CONFIG_GLOBAL': os.devnull, 'GIT_CONFIG_SYSTEM': os.devnull,
               'OPENGIT_E2E_DIALOGS_JSON': json.dumps([str(fixture.source)])}
        with ContextSession(ROOT, executable, env) as desktop:
            wait_for(lambda: body_contains(desktop, 'Your repositories'), 'workspace')
            click_text(desktop, 'Open repository')
            wait_for(lambda: idle(desktop), 'real history loaded', timeout=90)
            opening_and_cancel(desktop)
            expected = select_and_check(desktop, fixture, 'Local-only publication context', fixture.local_only)
            assert expected['refs/heads/main'] == expected['refs/heads/local/same-tip'] == 'contains'
            assert expected['refs/remotes/origin/main'] == 'doesNotContain'
            assert body_contains(desktop, 'No matching successful app fetch evidence')
            for width, height in ((1440, 900), (960, 640), (1280, 820)):
                desktop.command('POST', '/window/rect', {'width': width, 'height': height})
                wait_for(lambda: desktop.script('''
                    const g=document.querySelector('.history-scroll'),i=document.querySelector('.history-inspector-pane');
                    return !!g&&!!i&&g.clientHeight>60&&i.clientHeight>60
                        &&document.documentElement.scrollWidth<=innerWidth&&document.documentElement.scrollHeight<=innerHeight;
                '''), 'graph/context remain usable within native viewport')
            select_and_check(desktop, fixture, 'Octopus merge fixture', fixture.ancestor)
            action(desktop, 'Solo refs/heads/feature/alpha')
            wait_for(lambda: idle(desktop), 'Solo applied', timeout=90)
            select_and_check(desktop, fixture, 'Local-only publication context', fixture.local_only)
            assert desktop.script('return !!document.querySelector(".commit-inspector .history-outside")')
            print('PASS: native all-branch containment, qualified same-OID names and Solo-independent selection', flush=True)

            new_tip = fixture.advance_remote()
            fetch(desktop)
            assert fixture.run('rev-parse', 'origin/main').decode().strip() == new_tip
            assert fixture.run('rev-parse', 'origin/new-on-remote').decode().strip() == new_tip
            assert fixture.run('rev-parse', 'origin/stale').decode().strip() == fixture.ancestor
            assert fixture.protected_source() == protected
            select_and_check(desktop, fixture, 'Octopus merge fixture', fixture.ancestor)
            wait_for(lambda: desktop.script('''
                const p=document.querySelector('[aria-label="Branch membership"]');
                if(!p)return false;
                const r=[...p.querySelectorAll('[data-ref-name]')];
                return r.find(e=>e.dataset.refName==='refs/remotes/origin/main')?.textContent.includes('Last successful app fetch:')
                    &&r.find(e=>e.dataset.refName==='refs/remotes/origin/stale')?.textContent.includes('No matching successful app fetch evidence');
            '''), 'per-ref successful freshness excludes stale deleted ref', timeout=90)
            print('PASS: native prepared fetch, exact remote refs and per-ref freshness; local state preserved', flush=True)

            fetch(desktop, prune=True, hold_seconds=31, refresh_context=True)
            assert fixture.run('for-each-ref', '--format=%(refname)', 'refs/remotes/origin/stale') == b''
            assert fixture.protected_source() == protected
            select_and_check(desktop, fixture, 'Octopus merge fixture', fixture.ancestor)
            assert 'refs/remotes/origin/stale' not in membership(desktop)
            print('PASS: native opt-in prune removes only stale remote-tracking evidence', flush=True)

            def refresh_after_first_remote_effect():
                wait_for(lambda: fixture.run('for-each-ref', '--format=%(objectname)',
                                             'refs/remotes/backup/main').decode().strip() == new_tip,
                         'first captured remote has real effects', timeout=90)
                click_text(desktop, 'Refresh sync')
                wait_for(lambda: desktop.script('''
                    return document.querySelector('[aria-label="Repository sync"]')?.textContent.includes('Refreshing local');
                '''), 'normal same-repository refresh during accepted fetch', timeout=30)
                wait_for(lambda: desktop.script('''
                    const n=document.querySelector('.repository-tabs'),s=n?.querySelector('[role=status]'),t=n?.querySelector('[aria-current=page]');
                    if(!s||!t)return false;const a=s.getBoundingClientRect(),b=t.getBoundingClientRect();
                    return Math.abs((a.top+a.bottom-b.top-b.bottom)/2)<=2&&a.right<=innerWidth;
                '''), 'Refreshing remains aligned during accepted multi-remote fetch', timeout=30)

            fetch(desktop, all_remotes=True, on_started=refresh_after_first_remote_effect)
            assert fixture.run('rev-parse', 'backup/main').decode().strip() == new_tip
            assert fixture.run('rev-parse', 'origin/main').decode().strip() == new_tip
            assert fixture.protected_source() == protected
            assert desktop.script('''
                const task=[...document.querySelectorAll('article[aria-label="Fetch remotes task"]')].at(-1);
                return [...task.querySelectorAll('.fetch-results>li>strong')].map(e=>e.textContent);
            ''') == ['backup', 'origin']
            select_and_check(desktop, fixture, 'Octopus merge fixture', fixture.ancestor)
            print('PASS: native two-remote fetch survives normal same-repository refresh after first effects', flush=True)
            for width, height in ((1280, 820), (960, 640), (1280, 820)):
                desktop.command('POST', '/window/rect', {'width': width, 'height': height})
                wait_for(lambda: desktop.script('''
                    const g=document.querySelector('.history-scroll'),i=document.querySelector('.history-inspector-pane');
                    const panel=document.querySelector('.task-panel'),body=panel?.querySelector('.task-body');
                    return !!g&&!!i&&!!body&&g.clientHeight>60&&i.clientHeight>60
                        &&body.scrollHeight>body.clientHeight&&body.clientHeight>20
                        &&document.documentElement.scrollWidth<=innerWidth&&document.documentElement.scrollHeight<=innerHeight;
                '''), 'post-three-task graph/inspector and independently scrollable task pane')
            dimensions = desktop.script('''
                return {graph:document.querySelector('.history-scroll').clientHeight,
                    inspector:document.querySelector('.history-inspector-pane').clientHeight,
                    tasks:document.querySelector('.task-panel').clientHeight};
            ''')
            print('PASS: native post-three-task layout at1280x820/960x640; final dimensions:', dimensions, flush=True)
            print('Screenshot:', desktop.screenshot('daily-context.png'), flush=True)
