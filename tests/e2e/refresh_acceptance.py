"""Native refresh UI/visibility convergence, without modifying the IPC bridge.

Observed timestamps come from actual Git inspection responses rendered by the
app. Unit/coordinator tests separately verify the exact request-count policy.
"""
import json
import os
from pathlib import Path
import sys
import time

from desktop_smoke import DesktopSession
from workspace_acceptance import wait_for, workspace, body_contains, click_text

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'tests/e2e/support'))
from history_fixtures import HistoryFixture


def observation(desktop):
    return desktop.script('return document.querySelector(".observation")?.textContent || null')


if __name__ == '__main__':
    executable = Path(os.environ['OPENGIT_E2E_EXECUTABLE'])
    with HistoryFixture(ROOT, 40) as fixture:
        env = {'OPENGIT_DATA_DIR': str(fixture.root / 'refresh-data'),
               'GIT_CONFIG_GLOBAL': os.devnull, 'GIT_CONFIG_SYSTEM': os.devnull,
               'OPENGIT_E2E_DIALOGS_JSON': json.dumps([str(fixture.source)])}
        with DesktopSession(ROOT, executable, env) as desktop:
            wait_for(lambda: body_contains(desktop, 'Your repositories'), 'native workspace')
            click_text(desktop, 'Open repository')
            wait_for(lambda: len(workspace(desktop)['repositories']) == 1, 'repository open')
            click_text(desktop, 'Overview')
            wait_for(lambda: body_contains(desktop, 'Working directory is clean'), 'initial inspection')
            time.sleep(1)
            before = observation(desktop)
            assert before, 'Actual inspection timestamp was not rendered'
            time.sleep(3)
            assert observation(desktop) == before, 'Old 2-second UI refreshing is still visible'
            assert desktop.script('return document.querySelector(".workspace-content").getAttribute("aria-busy")') != 'true'
            assert not body_contains(desktop, 'Refreshing…')
            print('PASS: real desktop stays quiet and retains content between background checks', flush=True)
            started = time.monotonic()
            wait_for(lambda: observation(desktop) != before, '30-second fallback response rendered', 33)
            assert 22 <= time.monotonic() - started <= 34, 'Refresh response timing differs from fallback policy'
            print('PASS: observed actual background inspection timestamp update near 30-second fallback', flush=True)
            desktop.command('POST', '/window/minimize', {})
            wait_for(lambda: desktop.script('return document.visibilityState') == 'hidden', 'actual hidden WebView')
            time.sleep(1)
            hidden = observation(desktop)
            (fixture.source / 'README.txt').write_text('External edit while app is hidden\n')
            time.sleep(31)
            assert observation(desktop) == hidden, 'Inspection UI updated while hidden'
            desktop.command('POST', '/window/rect', {'width': 1280, 'height': 820})
            wait_for(lambda: desktop.script('return document.visibilityState') == 'visible', 'visible WebView')
            wait_for(lambda: body_contains(desktop, 'README.txt'), 'external edit after resume', 15)
            assert observation(desktop) != hidden
            print('PASS: actual hidden-window UI pause and resume converges to external file change', flush=True)
            before_manual = observation(desktop)
            time.sleep(1)
            wait_for(lambda: desktop.script("""
const button=document.querySelector('button[aria-label="Refresh repository"]');
if(!button||button.disabled)return false;button.click();return true;"""), 'manual refresh button')
            wait_for(lambda: observation(desktop) != before_manual, 'manual inspection response rendered')
            print('PASS: manual Refresh updates actual inspection timestamp', flush=True)
