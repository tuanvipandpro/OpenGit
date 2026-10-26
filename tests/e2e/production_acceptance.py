"""Production binary: no chooser seam, workspace, graph/context and daily UI."""
import json
import os
from pathlib import Path
import sys

from desktop_smoke import DesktopSession
from workspace_acceptance import wait_for, workspace, body_contains, click_text

ROOT = Path(__file__).resolve().parents[2]
sys.path[:0] = [str(ROOT / '.tools/fixture-python'), str(ROOT / 'tests/e2e/support')]
from live_remotes import LiveRemotes


if __name__ == '__main__':
    production = ROOT / 'src-tauri/target/release/opengit.exe'
    seed = Path(os.environ['OPENGIT_E2E_EXECUTABLE'])
    assert seed.resolve() != production.resolve()
    assert b'OPENGIT_E2E_DIALOGS_JSON' not in production.read_bytes(), 'Production contains test chooser switch'
    with LiveRemotes(ROOT) as fixture:
        for index in range(35):
            (fixture.source / f'change-{index:02}.txt').write_text('Owned layout fixture\n', encoding='utf-8', newline='\n')
        for index in range(12):
            fixture._git('-C', str(fixture.source), 'remote', 'add', f'fixture-{index:02}',
                         f'https://example.invalid/owned-{index:02}.git')
        env = {'OPENGIT_DATA_DIR': str(fixture.root / 'production-data'),
               'GIT_CONFIG_GLOBAL': os.devnull, 'GIT_CONFIG_SYSTEM': os.devnull,
               'OPENGIT_E2E_DIALOGS_JSON': json.dumps([str(fixture.source)])}
        with DesktopSession(ROOT, seed, env) as desktop:
            wait_for(lambda: body_contains(desktop, 'Your repositories'), 'seed workspace')
            click_text(desktop, 'Open repository')
            wait_for(lambda: len(workspace(desktop)['repositories']) == 1, 'seed repository')
            selection = workspace(desktop)['activeSelection']
        # The production process cannot consume the test chooser queue. It
        # recovers its repository from the persisted, authoritative registry.
        with DesktopSession(ROOT, production, {**env, 'OPENGIT_E2E_DIALOGS_JSON': '[null]'}) as desktop:
            wait_for(lambda: desktop.script('return typeof window.__TAURI_INTERNALS__?.invoke === "function"'),
                     'production native bridge')
            wait_for(lambda: workspace(desktop)['activeSelection'] == selection, 'production selection restored')
            click_text(desktop, 'Overview')
            wait_for(lambda: body_contains(desktop, 'change-00.txt'), 'production status rendered')
            assert workspace(desktop)['activeSelection'] == selection
            environment = desktop.invoke('get_environment', {})
            assert environment['ok'] and environment['value']['appVersion'] == (ROOT / 'VERSION').read_text().strip()
            invalid = desktop.invoke('get_environment', {'unexpected': 'synthetic'})
            assert invalid.get('transportRejected') or invalid.get('ok') is False
            for width, height in ((1280, 820), (960, 640)):
                desktop.command('POST', '/window/rect', {'width': width, 'height': height})
                wait_for(lambda: desktop.script('return [innerWidth,innerHeight]') == [width, height],
                         'actual WebView viewport dimensions')
                def layout_ready():
                    return desktop.script('''
const content=document.querySelector('.workspace-content');
const main=document.querySelector('.repository-main');
const info=document.querySelector('.repository-info');
return !!info && getComputedStyle(content).overflowY==='hidden'
 && getComputedStyle(main).overflowY==='auto'
 && getComputedStyle(info).overflowY==='auto';''')
                wait_for(layout_ready, 'independent scroll pane CSS')
                assert desktop.script('''
const main=document.querySelector('.repository-main'), info=document.querySelector('.repository-info');
const initial=main.scrollTop;
info.scrollTop=100;
const infoOnly=info.scrollTop>0 && main.scrollTop===initial;
const infoTop=info.scrollTop;
main.scrollTop=100;
const mainOnly=main.scrollTop>0 && info.scrollTop===infoTop;
return infoOnly && mainOnly && document.documentElement.scrollWidth<=innerWidth
 && document.documentElement.scrollHeight<=innerHeight;'''), 'Panes cannot scroll independently or viewport overflows'
                desktop.script('document.querySelectorAll(".repository-main,.repository-info").forEach(e=>e.scrollTop=0)')
                print('Actual WebView viewport:', desktop.script('return [innerWidth,innerHeight]'), flush=True)
                print('Screenshot:', desktop.screenshot(f'production-{width}x{height}.png'), flush=True)
            click_text(desktop, 'History')
            wait_for(lambda: desktop.script('return !!document.querySelector(".history-commit-row")'),
                     'production history rows', timeout=90)
            assert desktop.script('''
                const c=document.querySelector('canvas.history-graph');
                if(!c||!c.width||!c.height)return false;
                const p=c.getContext('2d').getImageData(0,0,c.width,c.height).data;
                for(let i=3;i<p.length;i+=4)if(p[i])return true;
                return false;
            '''), 'Production canvas did not render the actual graph'
            desktop.click('.history-commit-row')
            wait_for(lambda: desktop.script('''
                const p=document.querySelector('[aria-label="Commit branch containment"]');
                const b=p?.querySelector('[data-ref-name="refs/heads/main"]');
                return b?.getAttribute('data-membership')==='contains'&&!p.querySelector('[role=alert]');
            '''), 'production selected-commit branch context', timeout=90)
            assert desktop.script('''return ['Local actions','Stash'].every(name=>
                [...document.querySelectorAll('button')].some(b=>b.textContent===name&&!b.disabled))''')
            click_text(desktop, 'Changes')
            wait_for(lambda: desktop.script('''
                const b=[...document.querySelectorAll('.change-path')].find(b=>b.textContent==='change-00.txt'&&!b.disabled);
                if(!b)return false;b.click();return true;
            '''), 'production Changes selection', timeout=90)
            wait_for(lambda: desktop.script('''
                const d=document.querySelector('[aria-label="Change diff"]');
                return d?.textContent.includes('Owned layout fixture')&&!!d.querySelector('input[type=checkbox]');
            '''), 'production eligible structured diff', timeout=90)
            click_text(desktop, 'Close diff')
            click_text(desktop, 'Stash')
            wait_for(lambda: desktop.script('''return !!document.querySelector('dialog[open], [role=dialog]')&&
                [...document.querySelectorAll('button')].some(b=>b.textContent==='Save stash')'''),
                     'production Stash manager', timeout=90)
            click_text(desktop, 'Close')
            assert workspace(desktop)['activeSelection'] == selection
            print('Screenshot:', desktop.screenshot('production-daily.png'), flush=True)
            print('PASS: no-e2e production binary, persisted workspace, typed IPC, two layouts, graph/containment, structured Changes and Stash controls')
