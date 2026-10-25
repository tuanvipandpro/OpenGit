"""Actual inspector/Monaco worker and durable UI view acceptance.

Only folder choice is injected. The Worker probe forwards to the real browser
worker. SQLite is observed read-only so UI IPC generations are undisturbed.
"""
import json
import os
from pathlib import Path
import sys

from desktop_smoke import DesktopSession
from history_ui_acceptance import action, idle, search
from workspace_acceptance import body_contains, click_text, wait_for, workspace

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'tests/e2e/support'))
from history_fixtures import HistoryFixture
from history_ui_probe import observe_workers, stored_view, worker_observation


class InspectorSession(DesktopSession):
    def __exit__(self, kind, error, traceback):
        if kind is not None:
            try:
                print('Inspector failure screenshot:', self.screenshot('inspector-failure.png'), flush=True)
                evidence = {'workers': worker_observation(self),
                            'body': self.script('return document.body.innerText')}
                (self.directory / 'inspector-failure.json').write_text(json.dumps(evidence), encoding='utf-8')
                print('Inspector failure evidence:', self.directory / 'inspector-failure.json', flush=True)
            except Exception:
                pass
        return super().__exit__(kind, error, traceback)


def control(desktop, label, value):
    desktop.script('''
        const e=document.querySelector('[aria-label="'+arguments[0]+'"]');
        if(!e)throw new Error('Control unavailable');
        if(e.tagName==='INPUT')Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(e,arguments[1]);
        else e.value=arguments[1];
        e.dispatchEvent(new Event('input',{bubbles:true}));
        e.dispatchEvent(new Event('change',{bubbles:true}));
    ''', label, str(value))


def select_result(desktop, subject):
    wait_for(lambda: desktop.script('''
        const b=[...document.querySelectorAll('.history-search li button')].find(b=>b.textContent.includes(arguments[0]));
        if(!b)return false;b.click();return true;
    ''', subject), 'search result selected: ' + subject)
    wait_for(lambda: desktop.script('''
        const e=document.querySelector('.commit-inspector-heading');
        const message=document.querySelector('.commit-message pre');
        return !!e&&e.textContent.includes(arguments[0])&&!!message&&message.textContent.includes(arguments[0])
            &&!!document.querySelector('.commit-files button');
    ''', subject), 'selected commit details: ' + subject, timeout=45)


def file(desktop, path):
    wait_for(lambda: desktop.script('''
        const b=[...document.querySelectorAll('.commit-files button')].find(b=>b.title===arguments[0]&&!b.disabled);
        if(!b)return false;b.click();return true;
    ''', path), 'changed file: ' + path)
    wait_for(lambda: desktop.script('return document.querySelector(".commit-files button[aria-current=true]")?.title===arguments[0]', path),
             'file selection acknowledged: ' + path)


def text_ready(desktop, text):
    return desktop.script('''
        const e=document.querySelector('.monaco-diff');
        return !!e&&e.getAttribute('aria-busy')==='false'&&!!e.querySelector('.monaco-editor')
            &&e.textContent.replace(/\u00a0/g,' ').includes(arguments[0])&&!e.querySelector('[role=alert]');
    ''', text)


def graph_anchor(desktop):
    return desktop.script('''
        const s=document.querySelector('.history-scroll'),r=s?.querySelector('.history-commit-row');
        if(!r)return null;const h=r.getBoundingClientRect().height,n=Math.floor(s.scrollTop/h);
        const a=[...s.querySelectorAll('.history-commit-row')].find(e=>Number(e.getAttribute('aria-rowindex'))===n+1);
        return a?{oid:a.dataset.oid,offset:s.scrollTop-n*h,top:s.scrollTop}:null;
    ''')


def refresh_comparison(desktop, path, base, text, choose_during_refresh=None):
    wait_for(lambda: desktop.script('return !document.querySelector("button[aria-label=\\"Refresh repository\\"]").disabled'),
             'manual refresh enabled')
    desktop.click('button[aria-label="Refresh repository"]')
    wait_for(lambda: desktop.script('return document.querySelector(".commit-displayed-comparison")?.textContent.includes("previously loaded comparison")'),
             'comparison retained during refresh', timeout=30)
    if choose_during_refresh:
        file(desktop, choose_during_refresh)
    wait_for(lambda: desktop.script('''
        const c=document.querySelector('.commit-displayed-comparison');
        return !!c&&c.textContent.includes(arguments[0])&&!c.textContent.includes('previously loaded comparison')
            &&document.querySelector('.commit-files button[aria-current=true]')?.title===arguments[0]
            &&document.querySelector('select[aria-label="Diff base"]').value===arguments[1];
    ''', path, base) and text_ready(desktop, text), 'current comparison rebound after refresh', timeout=60)


if __name__ == '__main__':
    executable = Path(os.environ.get('OPENGIT_E2E_EXECUTABLE',
                                   str(ROOT / 'src-tauri/target/debug/opengit.exe')))
    with HistoryFixture(ROOT, 650) as fixture:
        shallow = fixture.shallow_clone()
        shallow_before = (fixture.run('for-each-ref', '--format=%(refname) %(objectname)', cwd=shallow),
                          fixture.run('status', '--porcelain=v2', '-z', cwd=shallow),
                          (shallow / '.git/index').read_bytes(), (shallow / '.git/shallow').read_bytes())
        data = fixture.root / 'inspector-data'
        env = {'OPENGIT_DATA_DIR': str(data), 'GIT_CONFIG_GLOBAL': os.devnull,
               'GIT_CONFIG_SYSTEM': os.devnull,
               'OPENGIT_E2E_DIALOGS_JSON': json.dumps([str(fixture.source), str(shallow)])}
        before = fixture.snapshot()
        with InspectorSession(ROOT, executable, env) as desktop:
            wait_for(lambda: body_contains(desktop, 'Your repositories'), 'workspace')
            click_text(desktop, 'Open repository')
            wait_for(lambda: idle(desktop), 'history hydrated', timeout=60)
            repo_id = workspace(desktop)['activeSelection']['repoId']
            observe_workers(desktop)
            desktop.click('.history-commit-row[data-oid="' + fixture.tips['main'] + '"]')
            wait_for(lambda: desktop.script('return document.querySelectorAll(".commit-files button").length>0'),
                     'merge file list')
            file(desktop, 'feature.txt')
            wait_for(lambda: text_ready(desktop, 'Feature change'), 'real Monaco diff', timeout=45)
            # Monaco's pinned diff provider returns a trivial result without a
            # worker when the original model is empty. Exercise two nonempty
            # models before requiring actual worker construction/messages.
            control(desktop, 'Diff layout', 'inline')
            control(desktop, 'Diff base', '1')
            file(desktop, 'README.txt')
            wait_for(lambda: text_ready(desktop, 'History fixture revision 650'), 'explicit merge parent diff')
            assert text_ready(desktop, 'History fixture revision 325')
            wait_for(lambda: worker_observation(desktop)['messages'] > 0, 'actual editor worker response', timeout=30)
            probe = worker_observation(desktop)
            assert probe['urls'] and probe['errors'] == 0, probe
            assert desktop.script('''
                return window.__nativeHistoryWorkers.urls.every(value=>{
                    const u=new URL(value,location.href);
                    return u.origin===location.origin||value.startsWith('blob:'+location.origin+'/');
                });
            '''), 'Monaco worker is not a local application asset'
            assert desktop.script('const t=[...document.querySelectorAll(".monaco-diff textarea")];return t.length>0&&t.every(e=>e.readOnly);'), 'Monaco DOM inputs must be read-only'
            refresh_comparison(desktop, 'other.txt', '1', 'Other change', choose_during_refresh='other.txt')
            print('PASS: native local Monaco worker/messages/CSP/read-only and merge-parent/inline diff', flush=True)

            search(desktop, 'DeepNeedle')
            select_result(desktop, 'DeepNeedle earliest history')
            file(desktop, 'binary.dat')
            wait_for(lambda: body_contains(desktop, 'This file contains binary data.'), 'typed binary feedback')
            assert not desktop.script('return !!document.querySelector(".monaco-diff")')
            search(desktop, 'History fixture commit 000005')
            select_result(desktop, 'History fixture commit 000005')
            file(desktop, 'whitespace.txt')
            wait_for(lambda: text_ready(desktop, 'Same line'), 'whitespace text diff')
            wait_for(lambda: desktop.script('return document.querySelectorAll(".monaco-diff .char-insert,.monaco-diff .char-delete,.monaco-diff .line-insert,.monaco-diff .line-delete").length>0'),
                     'whitespace-only change decoration')
            refresh_comparison(desktop, 'whitespace.txt', '0', 'Same line')
            print('PASS: native binary feedback and whitespace-only diff rendering', flush=True)

            wait_for(lambda: stored_view(data, repo_id) is not None, 'initial acknowledged view')
            legacy_sidebar = {key: stored_view(data, repo_id)['view'][key] for key in ('sidebarWidth', 'sidebarCollapsed')}
            action(desktop, 'Solo refs/heads/main')
            wait_for(lambda: idle(desktop) and body_contains(desktop, '1 solo'), 'Solo')
            action(desktop, 'Solo refs/heads/feature/alpha')
            wait_for(lambda: idle(desktop) and body_contains(desktop, '2 solo'), 'multi Solo')
            action(desktop, 'Hide refs/heads/orphan/root')
            wait_for(lambda: idle(desktop) and body_contains(desktop, '1 hidden'), 'Hide')
            search(desktop, 'UnmergedNeedle')
            select_result(desktop, 'UnmergedNeedle outside visible history')
            assert desktop.script('return !!document.querySelector(".commit-inspector .history-outside")')
            assert not desktop.script('return !!document.querySelector(".history-commit-row[data-oid=\\""+arguments[0]+"\\"]")', fixture.tips['topic/unmerged'])
            assert not desktop.script('return !!document.querySelector("input[aria-label=\\"Sidebar width\\"],.history-sidebar-pane")')
            control(desktop, 'Details height', 400)
            for width, height in ((1440, 900), (960, 640)):
                desktop.command('POST', '/window/rect', {'width': width, 'height': height})
                wait_for(lambda: desktop.script('''
                    const g=document.querySelector('.history-scroll'),i=document.querySelector('.history-inspector-pane');
                    return !!g&&!!i&&g.clientHeight>60&&i.clientHeight>60
                        &&document.documentElement.scrollWidth<=innerWidth&&document.documentElement.scrollHeight<=innerHeight;
                '''), 'usable graph/inspector within viewport')
                print('Screenshot:', desktop.screenshot(f'inspector-{width}.png'), flush=True)
            desktop.script('const s=document.querySelector(".history-scroll");s.scrollTop=1451;')
            wait_for(lambda: (anchor := graph_anchor(desktop)) and anchor['top'] >= 1400, 'saved scroll position')
            anchor = graph_anchor(desktop)
            expected_filter = {'hiddenRefs': ['refs/heads/orphan/root'],
                               'soloRefs': ['refs/heads/feature/alpha', 'refs/heads/main']}

            def saved():
                current = stored_view(data, repo_id)
                if not current:
                    return False
                view = current['view']
                return (view['filter'] == expected_filter and view['selectedOid'] == fixture.tips['topic/unmerged']
                        and view['anchorOid'] == anchor['oid'] and abs(view['anchorOffset'] - anchor['offset']) < 1
                        and view['diffLayout'] == 'inline' and view['detailsHeight'] == 400
                        and all(view[key] == value for key, value in legacy_sidebar.items()))
            wait_for(saved, 'actual durable UI preferences', timeout=30)
            click_text(desktop, 'Open repository')
            wait_for(lambda: workspace(desktop)['activeSelection']['repoId'] != repo_id,
                     'second repository selected')
            second_id = workspace(desktop)['activeSelection']['repoId']
            wait_for(lambda: idle(desktop) and not desktop.script('return !!document.querySelector(".history-active-filters")'),
                     'independent second-repository filters', timeout=60)
            action(desktop, 'Solo refs/heads/main')
            # Deliberately switch immediately, inside the save debounce. This
            # exercises the normal navigation flush, not an artificial IPC call.
            assert desktop.script('''
                const b=[...document.querySelectorAll('.repo-select')].find(b=>b.querySelector('strong')?.textContent==='history');
                if(!b)return false;b.click();return true;
            ''')
            wait_for(lambda: workspace(desktop)['activeSelection']['repoId'] == repo_id and idle(desktop),
                     'return to first repository', timeout=60)
            wait_for(lambda: saved(), 'first-repository view preserved', timeout=30)
            wait_for(lambda: (second := stored_view(data, second_id)) and second['view']['filter'] ==
                     {'hiddenRefs': [], 'soloRefs': ['refs/heads/main']},
                     'second-repository immediate-switch preference saved', timeout=30)
            assert fixture.snapshot() == before
        with InspectorSession(ROOT, executable, {**env, 'OPENGIT_E2E_DIALOGS_JSON': '[]'}) as reopened:
            wait_for(lambda: idle(reopened), 'restored filtered history', timeout=60)
            wait_for(lambda: reopened.script('return !!document.querySelector(".commit-inspector-heading")&&document.querySelector(".commit-inspector-heading").textContent.includes("UnmergedNeedle")'),
                     'restored selected commit', timeout=45)
            wait_for(lambda: (restored := graph_anchor(reopened)) and abs(restored['top'] - anchor['top']) < 2,
                     'restored scroll anchor', timeout=45)
            assert not reopened.script('return !!document.querySelector(".history-sidebar-pane,dialog[open]")')
            assert reopened.script('return document.querySelector("select[aria-label=\\"Diff layout\\"]").value') == 'inline'
            click_text(reopened, 'References')
            assert body_contains(reopened, '2 solo') and body_contains(reopened, '1 hidden')
            reopened.click('button[aria-label="Close References"]')
            assert all(stored_view(data, repo_id)['view'][key] == value for key, value in legacy_sidebar.items())
            assert fixture.snapshot() == before
            assert (fixture.run('for-each-ref', '--format=%(refname) %(objectname)', cwd=shallow),
                    fixture.run('status', '--porcelain=v2', '-z', cwd=shallow),
                    (shallow / '.git/index').read_bytes(), (shallow / '.git/shallow').read_bytes()) == shallow_before
            print('PASS: native outside-filter inspector, independent repo filters/fast switch and layout/selection/anchor after restart', flush=True)
