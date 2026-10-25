"""Native workspace acceptance. Build with the opt-in `e2e` chooser seam.

Only folder selection is injected. Rendering, commands, SQLite, Git, events,
refresh, restart and error handling execute in the real Windows application.
The production build does not contain the chooser queue.
"""
from pathlib import Path
import json
import os
import sys
import time

from desktop_smoke import DesktopSession

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / '.tools/fixture-python'))
sys.path.insert(0, str(ROOT / 'tests/e2e/support'))
from live_remotes import LiveRemotes


def wait_for(predicate, label, timeout=25):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(.2)
    raise AssertionError('Timed out: ' + label)


def click_text(desktop, text):
    wait_for(lambda: desktop.script('const b=[...document.querySelectorAll("button")].find('
        'b=>b.textContent.trim()===arguments[0]&&!b.disabled);if(b){b.click();return true}return false;', text),
        'button available: ' + text)


def body_contains(desktop, text):
    return desktop.script('return document.body.innerText.includes(arguments[0])', text)


def workspace(desktop):
    result = desktop.invoke('get_workspace', {})
    assert result.get('ok') is True
    return result['value']


if __name__ == '__main__':
    executable = Path(os.environ.get('OPENGIT_E2E_EXECUTABLE',
                                   str(ROOT / 'src-tauri/target/release/opengit.exe')))
    with LiveRemotes(ROOT) as fixture:
        data = fixture.root / 'app-data'
        env = {'OPENGIT_DATA_DIR': str(data), 'GIT_CONFIG_GLOBAL': os.devnull,
               'GIT_CONFIG_SYSTEM': os.devnull,
               'OPENGIT_E2E_DIALOGS_JSON': json.dumps([str(fixture.source), str(fixture.repository), None])}
        with DesktopSession(ROOT, executable, env) as desktop:
            wait_for(lambda: body_contains(desktop, 'Your repositories'), 'welcome')
            click_text(desktop, 'Open repository')
            wait_for(lambda: len(workspace(desktop)['repositories']) == 1, 'real folder registration')
            first = workspace(desktop)
            selected = first['activeSelection']
            click_text(desktop, 'Overview')
            wait_for(lambda: body_contains(desktop, 'Working directory is clean'), 'clean status')
            assert body_contains(desktop, 'main')
            (fixture.source / 'README.txt').write_text('Modified through the acceptance fixture\n', encoding='utf-8')
            wait_for(lambda: desktop.script("const b=[...document.querySelectorAll('button')].find(b=>b.getAttribute('aria-label')===arguments[0]);return !!b&&!b.disabled;", 'Refresh repository'), 'refresh available')
            desktop.script("[...document.querySelectorAll('button')].find(b=>b.getAttribute('aria-label')===arguments[0]).click()", 'Refresh repository')
            wait_for(lambda: body_contains(desktop, 'README.txt'), 'real Git dirty status')
            snapshot = desktop.invoke('inspect_repository', {**selected, 'generation': '900'})
            assert snapshot['ok'] and snapshot['value']['isDirty'] is True
            assert snapshot['value']['status'][0]['pathDisplay'] == 'README.txt'
            shot = desktop.screenshot('workspace.png')
            print('PASS: native folder selection seam -> registry -> Git -> working-tree UI')
            print('Screenshot:', shot)
            click_text(desktop, 'Open repository')
            wait_for(lambda: len(workspace(desktop)['repositories']) == 2, 'bare repository registration')
            click_text(desktop, 'Overview')
            wait_for(lambda: body_contains(desktop, 'This bare repository has no working directory'), 'bare metadata UI')
            before_cancel = workspace(desktop)
            click_text(desktop, 'Open repository')  # Native picker cancellation seam.
            time.sleep(.5)
            assert workspace(desktop)['activeSelection'] == before_cancel['activeSelection']
            result = desktop.invoke('select_repository', selected)
            assert result['ok']
        with DesktopSession(ROOT, executable, {**env, 'OPENGIT_E2E_DIALOGS_JSON': '[]'}) as reopened:
            wait_for(lambda: reopened.script('return typeof window.__TAURI_INTERNALS__?.invoke === "function"'),
                     'reopened native bridge')
            wait_for(lambda: len(workspace(reopened)['repositories']) == 2, 'SQLite reopen')
            assert workspace(reopened)['activeSelection'] == selected
            click_text(reopened, 'Overview')
            wait_for(lambda: body_contains(reopened, 'README.txt'), 'persistent selection restored')
            current = workspace(reopened)
            repo = next(r for r in current['repositories'] if r['id'] == selected['repoId'])
            label = 'Forget ' + repo['name']
            assert reopened.script('const b=[...document.querySelectorAll("button")].find('
                'b=>b.getAttribute("aria-label")===arguments[0]);if(b){b.click();return true}return false;', label)
            wait_for(lambda: len(workspace(reopened)['repositories']) == 1, 'forget registry entry')
            assert (fixture.source / 'README.txt').is_file()
            assert (fixture.source / '.git').is_dir()
            print('PASS: bare metadata, chooser cancel, durable reopen/selection and forget preserves files')
        broken = fixture.root / 'broken-data'
        broken.mkdir()
        original = b'not a SQLite database: owned synthetic failure fixture'
        (broken / 'workspace.sqlite3').write_bytes(original)
        with DesktopSession(ROOT, executable, {**env, 'OPENGIT_DATA_DIR': str(broken),
                                               'OPENGIT_E2E_DIALOGS_JSON': '[]'}) as failure:
            wait_for(lambda: failure.script('return document.querySelectorAll("[role=alert]").length>0'), 'safe database startup error')
            assert (broken / 'workspace.sqlite3').read_bytes() == original
            assert not body_contains(failure, original.decode())
            print('PASS: native safe startup error preserves corrupt database')
