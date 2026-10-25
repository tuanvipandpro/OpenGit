"""Actual history-screen acceptance; run only after App integration is built.

Folder selection is the only injected response. Canvas pixels, DOM rows,
filters and search use the real renderer, IPC and Git history services.
"""
from pathlib import Path
import json
import math
import os
import sys
import time

from desktop_smoke import DesktopSession
from workspace_acceptance import body_contains, click_text, wait_for

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'tests/e2e/support'))
from history_fixtures import HistoryFixture


def idle(desktop):
    return desktop.script('return !!document.querySelector(".history-commit-row") && '
        '![...document.querySelectorAll(".history-rows,.history-refs,.history-search")].some('
        'e=>e.getAttribute("aria-busy")==="true"||e.querySelector("[aria-busy=true]"))')


def action(desktop, label):
    click_text(desktop, 'References')
    wait_for(lambda: desktop.script('return !!document.querySelector("dialog.history-references-dialog[open]")'), 'References drawer')
    wait_for(lambda: desktop.script('const b=document.querySelector("button[aria-label="+JSON.stringify(arguments[0])+"]");return !!b&&!b.disabled', label),
             'reference action available: ' + label)
    desktop.click('button[aria-label=' + json.dumps(label) + ']')
    desktop.click('button[aria-label="Close References"]')
    wait_for(lambda: desktop.script('return !document.querySelector("dialog[open]")'), 'References drawer closed')


def graph_menu_action(desktop, ref_name, item):
    trigger = 'button[aria-label=' + json.dumps('Actions for ' + ref_name) + ']'
    selected = desktop.script('return document.querySelector(".history-commit-row[aria-selected=true]")?.dataset.oid??null')
    desktop.click(trigger)
    wait_for(lambda: desktop.script('return !!document.querySelector("[role=menu]")'), 'graph reference menu')
    assert desktop.script('return document.querySelector(".history-commit-row[aria-selected=true]")?.dataset.oid??null') == selected
    assert desktop.script('''
        const m=document.querySelector('[role=menu]'),r=m.getBoundingClientRect();
        return r.left>=7&&r.top>=7&&r.right<=innerWidth-7&&r.bottom<=innerHeight-7
          &&m.contains(document.activeElement);
    '''), 'Graph menu must fit the viewport and own keyboard focus'
    assert desktop.script('''
        const b=[...document.querySelectorAll('[role=menuitem]')].find(e=>e.textContent.trim()===arguments[0]);
        if(!b)return false;b.click();return true;
    ''', item), 'Graph action unavailable: ' + item
    wait_for(lambda: desktop.script('return !document.querySelector("[role=menu]")'), 'graph menu closed after action')


def graph_keyboard_probe(desktop, ref_name):
    desktop.script('''
        const b=document.querySelector('button[aria-label='+JSON.stringify(arguments[0])+']');
        b.focus();b.dispatchEvent(new KeyboardEvent('keydown',{key:'F10',shiftKey:true,bubbles:true}));
    ''', 'Actions for ' + ref_name)
    wait_for(lambda: desktop.script('return !!document.querySelector("[role=menu]")'), 'Shift+F10 graph menu')
    assert desktop.script('''
        const menu=document.querySelector('[role=menu]'),items=[...menu.querySelectorAll('[role=menuitem]')];
        document.activeElement.dispatchEvent(new KeyboardEvent('keydown',{key:'End',bubbles:true}));
        const end=document.activeElement===items.at(-1);
        document.activeElement.dispatchEvent(new KeyboardEvent('keydown',{key:'Home',bubbles:true}));
        return end&&document.activeElement===items[0];
    '''), 'Menu keyboard navigation must move actual WebView focus'
    desktop.script('document.activeElement.dispatchEvent(new KeyboardEvent("keydown",{key:"Escape",bubbles:true}));')
    wait_for(lambda: desktop.script('return !document.querySelector("[role=menu]")&&document.activeElement?.getAttribute("aria-label")===arguments[0]',
                                  'Actions for ' + ref_name), 'Escape restores graph badge focus')


def refresh_alignment(desktop):
    wait_for(lambda: desktop.script('return !document.querySelector("button[aria-label=\\"Refresh repository\\"]").disabled'), 'refresh available')
    desktop.click('button[aria-label="Refresh repository"]')
    wait_for(lambda: desktop.script('return !!document.querySelector(".repository-tabs [role=status]")'), 'manual refresh status')
    assert desktop.script('''
        const n=document.querySelector('.repository-tabs'),s=n.querySelector('[role=status]'),t=n.querySelector('[aria-current=page]');
        const a=s.getBoundingClientRect(),b=t.getBoundingClientRect();
        return Math.abs((a.top+a.bottom-b.top-b.bottom)/2)<=2&&a.right<=innerWidth
            &&document.documentElement.scrollWidth<=innerWidth;
    '''), 'Refreshing must be vertically aligned with active tab and fit the viewport'
    wait_for(lambda: desktop.script('return !document.querySelector("button[aria-label=\\"Refresh repository\\"]").disabled'), 'refresh complete', timeout=60)


def canvas_state(desktop):
    return desktop.script('''
        const c=document.querySelector('canvas.history-graph');
        const s=document.querySelector('.history-scroll');
        if(!c||!s||!c.width||!c.height)return null;
        const pixels=c.getContext('2d').getImageData(0,0,c.width,c.height).data;
        let painted=0;for(let i=3;i<pixels.length;i+=4)if(pixels[i])painted++;
        return {width:c.width,height:c.height,viewWidth:s.clientWidth,
            viewHeight:s.clientHeight,dpr:devicePixelRatio,painted,
            rows:document.querySelectorAll('.history-commit-row').length,
            scrollTop:s.scrollTop,scrollHeight:s.scrollHeight};
    ''')


def search(desktop, text):
    desktop.fill('.history-search input[type=search]', text)
    desktop.click('.history-search button[type=submit]')
    wait_for(lambda: desktop.script('const r=document.querySelector("[aria-label=\\"Search results\\"]");'
        'return !!r&&r.getAttribute("aria-busy")!=="true"&&r.textContent.includes(arguments[0]);', text),
        'real history search: ' + text, timeout=45)


if __name__ == '__main__':
    executable = Path(os.environ.get('OPENGIT_E2E_EXECUTABLE',
                                   str(ROOT / 'src-tauri/target/debug/opengit.exe')))
    with HistoryFixture(ROOT, 10_000) as fixture:
        for label in ('older', 'current'):
            (fixture.source / 'README.txt').write_text('Native ' + label + ' stash\n', encoding='utf-8')
            fixture.run('stash', 'push', '-m', 'native ' + label + ' stash')
        before = fixture.snapshot()
        env = {'OPENGIT_DATA_DIR': str(fixture.root / 'ui-data'),
               'GIT_CONFIG_GLOBAL': os.devnull, 'GIT_CONFIG_SYSTEM': os.devnull,
               'OPENGIT_E2E_DIALOGS_JSON': json.dumps([str(fixture.source)])}
        with DesktopSession(ROOT, executable, env) as desktop:
            wait_for(lambda: body_contains(desktop, 'Your repositories'), 'workspace')
            started = time.perf_counter()
            click_text(desktop, 'Open repository')
            wait_for(lambda: idle(desktop), 'first native history page', timeout=60)
            wait_for(lambda: (state := canvas_state(desktop)) and state['painted'] > 0,
                     'first native graph paint')
            first_page_seconds = time.perf_counter() - started
            click_text(desktop, 'References')
            wait_for(lambda: desktop.script('return !!document.querySelector("dialog.history-references-dialog[open]")'), 'References stash discovery')
            assert body_contains(desktop, 'stash@{0}') and body_contains(desktop, 'stash@{1}')
            desktop.click('button[aria-label="Close References"]')
            assert not desktop.script('return !!document.querySelector(".history-sidebar-pane,input[aria-label=\\"Sidebar width\\"]")')
            for width, height in ((1440, 900), (960, 640)):
                desktop.command('POST', '/window/rect', {'width': width, 'height': height})
                wait_for(lambda: (state := canvas_state(desktop)) and state['painted'] > 0
                         and state['width'] == math.ceil(state['viewWidth'] * state['dpr'])
                         and state['height'] == math.ceil(state['viewHeight'] * state['dpr']),
                         'painted native Canvas')
                state = canvas_state(desktop)
                assert 0 < state['rows'] < 100, state
                assert state['width'] == math.ceil(state['viewWidth'] * state['dpr']), state
                assert state['height'] == math.ceil(state['viewHeight'] * state['dpr']), state
                assert desktop.script('''
                    const a=document.querySelector('.history-graph-container').getBoundingClientRect();
                    const b=document.querySelector('.history-main').getBoundingClientRect();
                    return a.width>=b.width-28;
                '''), 'Graph must use the available history width'
                refresh_alignment(desktop)
                print('Canvas viewport:', json.dumps(state))
                print('Screenshot:', desktop.screenshot(f'history-{width}.png'))
            desktop.script('const s=document.querySelector(".history-scroll");s.scrollTop=2400;')
            wait_for(lambda: canvas_state(desktop)['scrollTop'] >= 2000, 'native graph scrolling')
            assert 0 < canvas_state(desktop)['rows'] < 100
            desktop.script('document.querySelector(".history-scroll").scrollTop=0;')
            wait_for(lambda: desktop.script('return !!document.querySelector("button[aria-label=\\"Actions for refs/heads/main\\"]")'), 'main badge visible')
            graph_keyboard_probe(desktop, 'refs/heads/main')
            graph_menu_action(desktop, 'refs/heads/main', 'Add to Solo')
            wait_for(lambda: idle(desktop) and body_contains(desktop, '1 solo'), 'Solo main')
            assert desktop.script('return document.activeElement!==document.body&&document.activeElement!==document.documentElement'), 'Graph action lost keyboard focus after session replacement'
            graph_menu_action(desktop, 'refs/heads/feature/alpha', 'Add to Solo')
            wait_for(lambda: idle(desktop) and body_contains(desktop, '2 solo'), 'multi Solo')
            action(desktop, 'Hide refs/heads/orphan/root')
            wait_for(lambda: idle(desktop) and body_contains(desktop, '1 hidden'), 'Hide with Solo')
            action(desktop, 'Show refs/heads/orphan/root')
            wait_for(lambda: idle(desktop) and not body_contains(desktop, '1 hidden'), 'restore hidden ref from References')
            action(desktop, 'Hide refs/heads/orphan/root')
            wait_for(lambda: idle(desktop) and body_contains(desktop, '1 hidden'), 'hide restored ref again')
            started = time.perf_counter()
            search(desktop, 'DeepNeedle')
            assert body_contains(desktop, 'DeepNeedle earliest history')
            search_seconds = time.perf_counter() - started
            search(desktop, 'UnmergedNeedle')
            assert body_contains(desktop, 'Outside current filters')
            assert body_contains(desktop, '2 solo') and body_contains(desktop, '1 hidden')
            assert fixture.snapshot() == before, 'History UI mutated Git refs/index/status'
            print('PASS: native Canvas/two viewports/virtual rows/scroll/multi Hide-Solo/full search')
            print('Warm native timings (not a release threshold):', json.dumps({
                'fixtureCommits': 10005, 'openThroughFirstPaintSeconds': first_page_seconds,
                'deepSearchThroughRenderedResultSeconds': search_seconds}))
