"""Native local branch/tag/upstream workflow; only folder choice is injected."""
import json
import os
from pathlib import Path
import sys

from daily_context_acceptance import ContextSession
from history_ui_acceptance import graph_menu_action, idle
from workspace_acceptance import body_contains, click_text, wait_for

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'tests/e2e/support'))
from daily_fixtures import DailyFixture


def control(desktop, label, value):
    desktop.script('''
        const d=document.querySelector('.repository-action-dialog');
        const l=[...d.querySelectorAll('label')].find(e=>e.firstChild?.nodeType===3&&e.firstChild.textContent.trim()===arguments[0]);
        const e=l?.querySelector('input,select,textarea');if(!e)throw new Error('Local action control missing');
        Object.getOwnPropertyDescriptor(Object.getPrototypeOf(e),'value').set.call(e,arguments[1]);
        e.dispatchEvent(new Event('input',{bubbles:true}));e.dispatchEvent(new Event('change',{bubbles:true}));
    ''', label, value)


def checkbox(desktop, label):
    assert desktop.script('''
        const d=document.querySelector('.repository-action-dialog');
        const l=[...d.querySelectorAll('label')].find(e=>e.textContent.includes(arguments[0]));
        const e=l?.querySelector('input[type=checkbox]');if(!e)return false;e.click();return true;
    ''', label)


def action(desktop, kind, *, reference=None, name=None, checkout=False,
           upstream=None, annotated=False, graph=None):
    before = desktop.script('return document.querySelectorAll("article.task").length')
    if graph:
        graph_menu_action(desktop, graph[0], graph[1])
    else:
        click_text(desktop, 'Local actions')
    wait_for(lambda: desktop.script('return !!document.querySelector(".repository-action-dialog")'),
             'current local action dialog', timeout=90)
    if not graph:
        control(desktop, 'Action', kind)
    if reference:
        control(desktop, 'Reference', 'ref:' + reference)
    if name:
        control(desktop, 'Tag name' if kind == 'tagCreate' else 'Branch name', name)
    if checkout:
        checkbox(desktop, 'Check out the new branch')
    if kind == 'branchTracking':
        control(desktop, 'Upstream', upstream or '')
    if annotated:
        checkbox(desktop, 'Annotated tag')
        control(desktop, 'Tag message', 'Owned native UI annotation')
    click_text(desktop, 'Prepare action')
    wait_for(lambda: desktop.script('''return !!document.querySelector('[aria-label="Local action preview"]')'''),
             'real prepared local action', timeout=90)
    if kind in ('branchDelete', 'tagDelete'):
        checkbox(desktop, 'Confirm local deletion')
    click_text(desktop, 'Apply action')
    wait_for(lambda: desktop.script('''
        const tasks=[...document.querySelectorAll('article.task')];
        return tasks.length===arguments[0]+1&&!document.querySelector('.repository-action-dialog')
            &&tasks.every(t=>t.querySelector('.task-state')?.textContent==='Succeeded'
                &&t.textContent.includes('Changes applied')&&!!t.querySelector('[aria-label="Local action steps"]'));
    ''', before), 'actual local action and verified per-step effects: ' + kind, timeout=150)
    print('PASS: native local action ' + kind, flush=True)


def visible_layout(desktop):
    return desktop.script('''
        const visible=e=>{if(!e)return 0;let r=e.getBoundingClientRect(),top=Math.max(0,r.top),bottom=Math.min(innerHeight,r.bottom);
            for(let p=e.parentElement;p;p=p.parentElement){const s=getComputedStyle(p);if(['hidden','clip','auto','scroll'].includes(s.overflowY)){
                const b=p.getBoundingClientRect();top=Math.max(top,b.top+p.clientTop);bottom=Math.min(bottom,b.top+p.clientTop+p.clientHeight);}}
            return Math.max(0,bottom-top);};
        const p=document.querySelector('[aria-label="Repository sync"]'),h=document.querySelector('.history-screen'),b=document.querySelector('.task-body');
        return {graph:visible(document.querySelector('.history-scroll')),inspector:visible(document.querySelector('.history-inspector-pane')),
            stable:!!p&&!p.querySelector('[role=alert]')&&!h?.querySelector('[role=alert]')
                &&!p.textContent.includes('Refreshing local')&&!p.textContent.includes('Previously loaded')
                &&!document.querySelector('.repository-refresh-status'),
            scrolling:!!b&&b.clientHeight>20&&b.scrollHeight>b.clientHeight,
            fits:document.documentElement.scrollWidth<=innerWidth&&document.documentElement.scrollHeight<=innerHeight};
    ''')


def check_layout(desktop):
    measurements = []
    for width, height in ((1280, 820), (960, 640), (1280, 820)):
        desktop.command('POST', '/window/rect', {'width': width, 'height': height})
        wait_for(lambda: (lambda m: m['stable'] and m['graph'] > 60 and m['inspector'] > 60
                         and m['scrolling'] and m['fits'])(visible_layout(desktop)),
                 'stable visible graph/inspector through clipping ancestors', timeout=90)
        measurements.append({'viewport': [width, height], **visible_layout(desktop)})
    print('PASS: native visible layout:', json.dumps(measurements), flush=True)


def layout_only(executable):
    with DailyFixture(ROOT) as fixture:
        env = {'OPENGIT_DATA_DIR': str(fixture.root / 'layout-data'),
               'GIT_CONFIG_GLOBAL': os.devnull, 'GIT_CONFIG_SYSTEM': os.devnull,
               'OPENGIT_E2E_DIALOGS_JSON': json.dumps([str(fixture.source)])}
        with ContextSession(ROOT, executable, env) as desktop:
            wait_for(lambda: body_contains(desktop, 'Your repositories'), 'workspace')
            click_text(desktop, 'Open repository')
            wait_for(lambda: idle(desktop), 'history', timeout=90)
            for index in range(3):
                action(desktop, 'tagCreate', name='native-layout-' + str(index))
            desktop.script('''const r=document.querySelector('.history-commit-row[data-oid="'+arguments[0]+'"]');if(!r)throw new Error('Missing selected row');r.click();''', fixture.local_only)
            check_layout(desktop)
            print('Screenshot:', desktop.screenshot('local-layout.png'), flush=True)


if __name__ == '__main__':
    executable = Path(os.environ.get('OPENGIT_E2E_EXECUTABLE',
                                   str(ROOT / 'src-tauri/target/debug/opengit.exe')))
    if '--layout-only' in sys.argv:
        layout_only(executable)
        raise SystemExit(0)
    with DailyFixture(ROOT) as fixture:
        original_head = fixture.local_only
        original_tag = fixture.run('rev-parse', 'refs/tags/v-fixture')
        working = {str(p.relative_to(fixture.source)): p.read_bytes()
                   for p in fixture.source.rglob('*') if p.is_file() and '.git' not in p.relative_to(fixture.source).parts}
        env = {'OPENGIT_DATA_DIR': str(fixture.root / 'local-actions-data'),
               'GIT_CONFIG_GLOBAL': os.devnull, 'GIT_CONFIG_SYSTEM': os.devnull,
               'OPENGIT_E2E_DIALOGS_JSON': json.dumps([str(fixture.source)])}
        with ContextSession(ROOT, executable, env) as desktop:
            wait_for(lambda: body_contains(desktop, 'Your repositories'), 'workspace')
            click_text(desktop, 'Open repository')
            wait_for(lambda: idle(desktop), 'history', timeout=90)
            action(desktop, 'branchCreate', name='native/ui-branch', checkout=True,
                   graph=('refs/heads/main', 'Create branch here'))
            assert fixture.run('symbolic-ref', 'HEAD').strip() == b'refs/heads/native/ui-branch'
            assert fixture.run('rev-parse', 'HEAD').decode().strip() == original_head
            action(desktop, 'branchTracking', reference='refs/heads/native/ui-branch', upstream='refs/remotes/origin/main')
            assert fixture.run('config', 'branch.native/ui-branch.remote').strip() == b'origin'
            action(desktop, 'branchTracking', reference='refs/heads/native/ui-branch')
            assert fixture.run('for-each-ref', '--format=%(upstream)', 'refs/heads/native/ui-branch').strip() == b''
            action(desktop, 'branchRename', name='native/ui-renamed',
                   graph=('refs/heads/native/ui-branch', 'Rename branch'))
            assert fixture.run('symbolic-ref', 'HEAD').strip() == b'refs/heads/native/ui-renamed'
            action(desktop, 'tagCreate', name='native-v0.3.0', annotated=True,
                   graph=('refs/heads/main', 'Create tag here'))
            assert fixture.run('cat-file', '-t', 'refs/tags/native-v0.3.0').strip() == b'tag'
            assert fixture.run('rev-parse', 'refs/tags/native-v0.3.0^{commit}').decode().strip() == original_head
            action(desktop, 'branchSwitch', reference='refs/heads/main')
            assert fixture.run('symbolic-ref', 'HEAD').strip() == b'refs/heads/main'
            action(desktop, 'branchDelete', reference='refs/heads/native/ui-renamed')
            assert fixture.run('for-each-ref', '--format=%(refname)', 'refs/heads/native/ui-renamed') == b''
            action(desktop, 'tagDelete', reference='refs/tags/native-v0.3.0')
            assert fixture.run('for-each-ref', '--format=%(refname)', 'refs/tags/native-v0.3.0') == b''
            action(desktop, 'tagCreate', name='native-light')
            assert fixture.run('cat-file', '-t', 'refs/tags/native-light').strip() == b'commit'
            action(desktop, 'tagDelete', reference='refs/tags/native-light')
            assert fixture.run('rev-parse', 'refs/tags/v-fixture') == original_tag
            assert {str(p.relative_to(fixture.source)): p.read_bytes() for p in fixture.source.rglob('*')
                    if p.is_file() and '.git' not in p.relative_to(fixture.source).parts} == working
            assert fixture.run('status', '--porcelain=v2', '-z') == b''
            desktop.click('.history-commit-row[data-oid="' + original_head + '"]')
            wait_for(lambda: desktop.script('return !!document.querySelector(".commit-inspector-heading code")'), 'selected commit')
            check_layout(desktop)
            print('PASS: native graph/toolbar branch create/checkout/rename/switch/delete, upstream set/unset, annotated/light tags/delete', flush=True)
            print('PASS: local working bytes/unrelated tag preserved; post-task layout1280x820/960x640', flush=True)
            print('Screenshot:', desktop.screenshot('local-actions.png'), flush=True)
