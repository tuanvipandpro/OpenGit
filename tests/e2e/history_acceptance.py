"""Native history-service acceptance; graph/diff UI checks are added separately.

Only folder selection uses the opt-in seam. History, cursors, filters, search
and invalidation execute in the actual Windows application with real Git.
"""
from pathlib import Path
import json
import os
import sys
import time

from desktop_smoke import DesktopSession
from workspace_acceptance import wait_for, workspace, body_contains, click_text

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'tests/e2e/support'))
from history_fixtures import HistoryFixture


def value(result):
    assert result.get('ok'), 'Native history request failed: ' + str(result.get('error', {}).get('code', 'transport'))
    return result['value']


def open_history(desktop, selection, generation, hidden=(), solo=()):
    scope = {**selection, 'generation': str(generation)}
    return value(desktop.invoke('open_history', {'scope': scope,
        'filter': {'hiddenRefs': list(hidden), 'soloRefs': list(solo)}, 'pageSize': 200}))


def collect_history(desktop, session):
    page = session['page']
    commits = list(page['commits'])
    seen_cursors = set()
    while page['nextCursor'] is not None:
        cursor = page['nextCursor']
        assert cursor not in seen_cursors, 'Page cursor failed to advance'
        seen_cursors.add(cursor)
        page = value(desktop.invoke('load_history_page', {'context': session['context'],
            'sessionId': session['sessionId'], 'cursor': cursor, 'pageSize': 200}))
        assert page['context'] == session['context']
        commits.extend(page['commits'])
    assert len({commit['oid'] for commit in commits}) == len(commits), 'Duplicate paginated commit'
    return commits


def assert_parent_truth(commits, oracle):
    expected = {row.split()[0]: row.split()[1:] for row in oracle}
    actual = {commit['oid']: [parent['oid'] for parent in commit['parents']] for commit in commits}
    assert actual == expected, 'Native history differs from independent Git parent oracle'
    positions = {commit['oid']: index for index, commit in enumerate(commits)}
    for child, parents in actual.items():
        for parent in parents:
            assert positions[parent] > positions[child], 'History is not topological'


if __name__ == '__main__':
    executable = Path(os.environ.get('OPENGIT_E2E_EXECUTABLE',
                                   str(ROOT / 'src-tauri/target/release/opengit.exe')))
    with HistoryFixture(ROOT, 650) as fixture:
        linked = fixture.linked_worktree()
        shallow = fixture.shallow_clone()
        env = {'OPENGIT_DATA_DIR': str(fixture.root / 'app-data'),
               'GIT_CONFIG_GLOBAL': os.devnull, 'GIT_CONFIG_SYSTEM': os.devnull,
               'OPENGIT_E2E_DIALOGS_JSON': json.dumps([str(fixture.source), str(linked), str(shallow)])}
        with DesktopSession(ROOT, executable, env) as desktop:
            wait_for(lambda: body_contains(desktop, 'Your repositories'), 'native workspace')
            click_text(desktop, 'Open repository')
            wait_for(lambda: len(workspace(desktop)['repositories']) == 1, 'history repo registered')
            selection = workspace(desktop)['activeSelection']
            before = fixture.snapshot()
            started = time.perf_counter()
            session = open_history(desktop, selection, 100)
            commits = collect_history(desktop, session)
            assert session['context']['scope'] == {**selection, 'generation': '100'}
            assert_parent_truth(commits, fixture.oracle)
            elapsed = time.perf_counter() - started
            print('PASS: native paginated DAG655 commits/raw parents/octopus/orphan; seconds:', round(elapsed, 3))
            deep = value(desktop.invoke('search_history', {'context': session['context'],
                'sessionId': session['sessionId'], 'query': 'DeepNeedle', 'cursor': None, 'pageSize': 100}))
            assert len(deep['commits']) == 1
            assert deep['commits'][0]['commit']['oid'] not in {c['oid'] for c in session['page']['commits']}

            solo = open_history(desktop, selection, 101, solo=('refs/heads/main',))
            visible = collect_history(desktop, solo)
            solo_oracle = fixture.run('rev-list', '--topo-order', '--parents', 'main').decode().splitlines()
            assert_parent_truth(visible, solo_oracle)
            assert fixture.tips['topic/unmerged'] not in {commit['oid'] for commit in visible}
            found = value(desktop.invoke('search_history', {'context': solo['context'],
                'sessionId': solo['sessionId'], 'query': 'UnmergedNeedle', 'cursor': None, 'pageSize': 100}))
            assert any(hit['commit']['oid'] == fixture.tips['topic/unmerged'] and hit['outsideView']
                       for hit in found['commits']), 'Search did not reveal history outside Solo'
            print('PASS: native positive-root Solo and full-history search outside filters/loaded pages')

            invalid = desktop.invoke('load_history_page', {'context': solo['context'],
                'sessionId': solo['sessionId'], 'cursor': session['page']['nextCursor'], 'pageSize': 200})
            assert invalid.get('ok') is False and invalid['error']['code'] == 'InvalidHandle'
            assert fixture.snapshot() == before, 'History read changed source refs/index/status'

            stale = open_history(desktop, selection, 102)
            fixture.run('update-ref', 'refs/heads/external/change', fixture.tips['main'])
            result = desktop.invoke('load_history_page', {'context': stale['context'],
                'sessionId': stale['sessionId'], 'cursor': stale['page']['nextCursor'], 'pageSize': 200})
            assert result.get('ok') is False and result['error']['code'] == 'ExternalChange'
            print('PASS: native cross-snapshot cursor rejection, source preservation and external-ref invalidation')

            click_text(desktop, 'Open repository')
            wait_for(lambda: workspace(desktop)['activeSelection'] != selection, 'linked worktree selected')
            linked_selection = workspace(desktop)['activeSelection']
            assert linked_selection['repoId'] == selection['repoId']
            assert linked_selection['worktreeId'] != selection['worktreeId']
            linked_session = open_history(desktop, linked_selection, 104)
            assert linked_session['head']['kind'] == 'detached'
            assert_parent_truth(collect_history(desktop, linked_session), fixture.oracle)
            print('PASS: native linked-worktree identity and detached HEAD history')

            click_text(desktop, 'Open repository')
            wait_for(lambda: workspace(desktop)['activeSelection']['repoId'] != selection['repoId'],
                     'shallow repository selected')
            shallow_selection = workspace(desktop)['activeSelection']
            shallow_session = open_history(desktop, shallow_selection, 105)
            shallow_commits = collect_history(desktop, shallow_session)
            boundaries = set((shallow / '.git/shallow').read_text().splitlines())
            marked = False
            for commit in shallow_commits:
                header = fixture.run('cat-file', 'commit', commit['oid'], cwd=shallow).split(b'\n\n', 1)[0]
                parents = [line.split()[1].decode() for line in header.splitlines() if line.startswith(b'parent ')]
                assert [parent['oid'] for parent in commit['parents']] == parents
                if commit['oid'] in boundaries and parents:
                    marked |= any(parent['boundary'] == 'shallow' for parent in commit['parents'])
            assert marked, 'Native history lost explicit shallow parent boundaries'
            print('PASS: native shallow history preserves raw object parents and explicit boundaries')
