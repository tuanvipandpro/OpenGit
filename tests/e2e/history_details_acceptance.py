"""Native S4 API checks, to run after actual details/view handlers are built.

This service-level test does not establish inspector/Monaco UI acceptance.
All Git writes and the external-diff canary remain inside an owned fixture.
"""
from pathlib import Path
import json
import os
import sys

from desktop_smoke import DesktopSession
from history_acceptance import open_history, value
from workspace_acceptance import body_contains, click_text, wait_for, workspace

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'tests/e2e/support'))
from history_fixtures import HistoryFixture


def commit(desktop, session, oid):
    result = value(desktop.invoke('search_history', {'context': session['context'],
        'sessionId': session['sessionId'], 'query': oid, 'cursor': None, 'pageSize': 100}))
    return next(hit['commit'] for hit in result['commits'] if hit['commit']['oid'] == oid)


def details(desktop, session, selected, base):
    return value(desktop.invoke('get_commit_details', {'context': session['context'],
        'commitHandle': selected['handle'], 'base': base}))


def diff(desktop, session, detail, path):
    file = next(file for file in detail['files'] if file['pathDisplay'] == path)
    result = value(desktop.invoke('get_commit_diff', {'context': session['context'],
        'fileHandle': file['handle']}))
    assert result['context'] == session['context'] and result['fileHandle'] == file['handle']
    return result['content']


if __name__ == '__main__':
    executable = Path(os.environ.get('OPENGIT_E2E_EXECUTABLE',
                                   str(ROOT / 'src-tauri/target/debug/opengit.exe')))
    with HistoryFixture(ROOT, 650) as fixture:
        marker = fixture.root / 'external-diff-executed'
        canary = fixture.root / 'external_diff_canary.py'
        canary.write_text('from pathlib import Path\nPath(' + repr(str(marker)) + ').touch()\n')
        command = '"' + sys.executable.replace('\\', '/') + '" "' + str(canary).replace('\\', '/') + '"'
        fixture.run('config', 'diff.external', command)
        fixture.run('config', 'diff.fixture.textconv', command)
        (fixture.source / '.gitattributes').write_text('*.txt diff=fixture\n', encoding='utf-8')
        before = fixture.snapshot()
        env = {'OPENGIT_DATA_DIR': str(fixture.root / 'details-data'),
               'GIT_CONFIG_GLOBAL': os.devnull, 'GIT_CONFIG_SYSTEM': os.devnull,
               'GIT_EXTERNAL_DIFF': command,
               'OPENGIT_E2E_DIALOGS_JSON': json.dumps([str(fixture.source)])}
        with DesktopSession(ROOT, executable, env) as desktop:
            wait_for(lambda: body_contains(desktop, 'Your repositories'), 'workspace')
            click_text(desktop, 'Open repository')
            wait_for(lambda: len(workspace(desktop)['repositories']) == 1, 'registered repo')
            selection = workspace(desktop)['activeSelection']
            session = open_history(desktop, selection, 100)
            root_oid = fixture.run('rev-list', '--max-parents=0', 'main').decode().strip()
            root = details(desktop, session, commit(desktop, session, root_oid), {'kind': 'root'})
            assert root['commit']['oid'] == root_oid and 'DeepNeedle' in root['message']
            readme = diff(desktop, session, root, 'README.txt')
            assert readme['kind'] == 'text' and readme['original'] == ''
            assert readme['modified'] == 'History fixture revision 1\n'
            assert diff(desktop, session, root, 'binary.dat') == {'kind': 'unavailable', 'reason': 'binary'}
            assert diff(desktop, session, root, 'unicode Ω.txt')['modified'] == 'Unicode fixture\n'
            assert diff(desktop, session, root, 'link.txt')['modified'] == 'README.txt'

            fifth_oid = fixture.run('rev-list', '--reverse', '--first-parent', 'main').decode().splitlines()[4]
            fifth = details(desktop, session, commit(desktop, session, fifth_oid), {'kind': 'parent', 'index': 0})
            whitespace = diff(desktop, session, fifth, 'whitespace.txt')
            assert whitespace['original'] == 'Same line\n' and whitespace['modified'] == 'Same line  \n'
            fourth_oid = fixture.run('rev-parse', fifth_oid + '^').decode().strip()
            renamed = details(desktop, session, commit(desktop, session, fourth_oid), {'kind': 'parent', 'index': 0})
            assert any(file['kind'] == 'renamed' and file['pathDisplay'] == 'renamed Ω.txt'
                       and file['previousPathDisplay'] == 'unicode Ω.txt' for file in renamed['files'])
            third_oid = fixture.run('rev-parse', fourth_oid + '^').decode().strip()
            deleted = details(desktop, session, commit(desktop, session, third_oid), {'kind': 'parent', 'index': 0})
            removed = diff(desktop, session, deleted, 'delete-me.txt')
            assert removed['original'] == 'Delete fixture\n' and removed['modified'] == ''

            merge = commit(desktop, session, fixture.tips['main'])
            assert len(merge['parents']) == 3
            for index, parent in enumerate(merge['parents']):
                detail = details(desktop, session, merge, {'kind': 'parent', 'index': index})
                expected = fixture.run('diff-tree', '--no-ext-diff', '--no-textconv', '--no-commit-id',
                    '--name-only', '-r', '-z', parent['oid'], merge['oid']).decode().split('\0')
                assert {file['pathDisplay'] for file in detail['files']} == set(filter(None, expected))
            invalid = desktop.invoke('get_commit_diff', {'context': session['context'], 'fileHandle': 'f' * 64})
            assert invalid.get('ok') is False and invalid['error']['code'] == 'InvalidHandle'
            assert not marker.exists(), 'External diff/textconv executed during safe history reads'
            assert fixture.snapshot() == before, 'History details changed refs/index/status'
            print('PASS: native root/Unicode/symlink/binary/whitespace/rename/merge-parent diffs; no external driver')

            scope = session['context']['scope']
            state = value(desktop.invoke('get_repository_view', {'scope': scope}))
            anchor_oid = fixture.run('rev-parse', 'main~50').decode().strip()
            saved_view = {**state['view'], 'filter': {'hiddenRefs': [], 'soloRefs': ['refs/heads/main']},
                          'selectedOid': fifth_oid, 'anchorOid': anchor_oid, 'anchorOffset': 11,
                          'diffLayout': 'sideBySide'}
            saved = value(desktop.invoke('save_repository_view', {'scope': scope,
                'expectedRevision': state['viewRevision'], 'view': saved_view}))
            assert saved['view'] == saved_view
            stale = desktop.invoke('save_repository_view', {'scope': scope,
                'expectedRevision': state['viewRevision'], 'view': state['view']})
            assert stale.get('ok') is False
            assert value(desktop.invoke('get_repository_view', {'scope': scope}))['view'] == saved_view
        with DesktopSession(ROOT, executable, {**env, 'OPENGIT_E2E_DIALOGS_JSON': '[]'}) as reopened:
            wait_for(lambda: len(workspace(reopened)['repositories']) == 1, 'durable workspace')
            restored = value(reopened.invoke('get_repository_view', {'scope': scope}))
            assert restored['view'] == saved_view and restored['viewRevision'] == saved['viewRevision']
            print('PASS: native view CAS/stale rejection and SQLite preferences after app restart')
