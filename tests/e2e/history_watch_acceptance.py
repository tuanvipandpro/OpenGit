"""Native notification/coordinator convergence, including real lost-subscription fallback.

The observer uses Tauri's installed event protocol and never supplies events.
The missed-event case unwatches an actual acknowledged subscription,
then verifies that the ordinary 30-second fallback observes the external edit.
"""
import json
import os
from pathlib import Path
import sys
import time

from desktop_smoke import DesktopSession
from history_ui_acceptance import idle
from refresh_acceptance import observation
from workspace_acceptance import body_contains, click_text, wait_for, workspace

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'tests/e2e/support'))
from history_fixtures import HistoryFixture

EVENT = 'opengit://repository-changed'


def observe_events(desktop):
    result = desktop.command('POST', '/execute/async', {'script': '''
        const done=arguments[arguments.length-1];
        const data={total:0,events:[]};window.__nativeRepositoryEvents=data;
        const handler=window.__TAURI_INTERNALS__.transformCallback(event=>{
            data.total++;data.events.push(event.payload);if(data.events.length>256)data.events.shift();
        });
        window.__TAURI_INTERNALS__.invoke('plugin:event|listen',{
            event:arguments[0],target:{kind:'Any'},handler
        }).then(id=>done({ok:true,id}),()=>done({ok:false}));
    ''', 'args': [EVENT]})
    assert result.get('ok'), 'Native event observer could not register'
    return result['id']


def observe_watch_acks(desktop):
    assert desktop.script('''
        const map=window.__TAURI_INTERNALS__.callbacks, original=map.set;
        const state={acks:[]};window.__nativeWatchAcks=state;
        const wrapper=function(id,callback){return Reflect.apply(original,this,[id,function(...args){
            try {
                const result=args[0],v=result?.value;
                if(result?.ok===true&&v&&Object.keys(v).sort().join(',')==='scope,sequence,status,subscriptionId'
                   &&['watching','fallback'].includes(v.status)) {
                    state.acks.push({scope:{...v.scope},subscriptionId:v.subscriptionId,sequence:v.sequence,status:v.status});
                    if(state.acks.length>256)state.acks.shift();
                }
            } catch {}
            return Reflect.apply(callback,this,args);
        }]);};
        map.set=wrapper;state.restore=()=>{if(map.set===wrapper)map.set=original;};
        return map.set===wrapper;
    '''), 'Installed SDK callback observer could not be attached'


def events(desktop):
    return desktop.script('return window.__nativeRepositoryEvents')


def await_event(desktop, previous, selection=None):
    def arrived():
        current = events(desktop)
        count = current['total'] - previous
        acks = desktop.script('return window.__nativeWatchAcks.acks')
        return count > 0 and any(
            (selection is None or event['scope']['repoId'] == selection['repoId']
             and event['scope']['worktreeId'] == selection['worktreeId'])
            and any(ack['scope'] == event['scope'] and ack['subscriptionId'] == event['subscriptionId'] for ack in acks)
            for event in current['events'][-count:])
    wait_for(arrived, 'actual path-free native notification', timeout=15)


def stop_current_subscription(desktop, selection):
    candidates = desktop.script('return window.__nativeWatchAcks.acks').copy()
    candidates = [ack for ack in candidates if ack['scope']['repoId'] == selection['repoId']
                  and ack['scope']['worktreeId'] == selection['worktreeId']]
    assert candidates, 'No real active-selection watch ACK was observed'
    subscription = candidates[-1]
    stopped = desktop.invoke('unwatch_repository', {'subscriptionId': subscription['subscriptionId']})
    assert stopped.get('ok'), 'Could not stop the actual acknowledged subscription'


if __name__ == '__main__':
    executable = Path(os.environ.get('OPENGIT_E2E_EXECUTABLE',
                                   str(ROOT / 'src-tauri/target/debug/opengit.exe')))
    with HistoryFixture(ROOT, 650) as fixture:
        linked = fixture.linked_worktree()
        env = {'OPENGIT_DATA_DIR': str(fixture.root / 'watch-data'),
               'GIT_CONFIG_GLOBAL': os.devnull, 'GIT_CONFIG_SYSTEM': os.devnull,
               'OPENGIT_E2E_DIALOGS_JSON': json.dumps([str(fixture.source), str(linked)])}
        with DesktopSession(ROOT, executable, env) as desktop:
            wait_for(lambda: body_contains(desktop, 'Your repositories'), 'workspace')
            observer_id = observe_events(desktop)
            observe_watch_acks(desktop)
            click_text(desktop, 'Open repository')
            wait_for(lambda: idle(desktop), 'initial history', timeout=60)
            click_text(desktop, 'Overview')
            wait_for(lambda: body_contains(desktop, 'Working directory is clean'), 'clean Overview')
            before = events(desktop)['total']
            (fixture.source / 'README.txt').write_text('External worktree notification\n', encoding='utf-8')
            expected = fixture.snapshot()
            await_event(desktop, before)
            wait_for(lambda: body_contains(desktop, 'README.txt'), 'Overview converges after file notification', timeout=15)
            assert fixture.snapshot() == expected
            print('PASS: real worktree notification refreshes Overview without manual refresh', flush=True)

            click_text(desktop, 'History')
            before = events(desktop)['total']
            fixture.run('add', '--', 'README.txt')
            fixture.run('commit', '-m', 'WatchNeedle external commit')
            expected = fixture.snapshot()
            await_event(desktop, before)
            wait_for(lambda: desktop.script('return [...document.querySelectorAll(".history-commit-row")].some(e=>e.textContent.includes("WatchNeedle external commit"))'),
                     'History converges after external commit', timeout=20)
            assert fixture.snapshot() == expected
            print('PASS: real Git/ref notifications refresh the graph', flush=True)

            original_selection = workspace(desktop)['activeSelection']
            click_text(desktop, 'Open repository')
            wait_for(lambda: workspace(desktop)['activeSelection'] != original_selection, 'linked selection')
            wait_for(lambda: body_contains(desktop, 'HEAD detached'), 'linked history', timeout=60)
            selection = workspace(desktop)['activeSelection']
            before = events(desktop)['total']
            fixture.run('update-ref', 'refs/heads/watch/common-dir', 'main')
            await_event(desktop, before, selection)
            wait_for(lambda: body_contains(desktop, 'watch/common-dir'), 'linked common-directory refs', timeout=20)
            before = events(desktop)['total']
            for index in range(20):
                fixture.run('update-ref', f'refs/heads/watch/burst-{index:02}', 'main')
            await_event(desktop, before, selection)
            wait_for(lambda: body_contains(desktop, 'watch/burst-19'), 'atomic ref burst convergence', timeout=20)
            print('PASS: linked worktree common-directory and atomic ref burst converge', flush=True)

            click_text(desktop, 'Overview')
            wait_for(lambda: body_contains(desktop, 'Working directory is clean'), 'linked Overview')
            # Observe a quiet periodic run before removing the live source of
            # hints, leaving almost a complete fallback interval for the check.
            time.sleep(3)
            prior = observation(desktop)
            wait_for(lambda: observation(desktop) != prior, 'periodic fallback while watches are healthy', timeout=35)
            wait_for(lambda: desktop.script('return !document.querySelector("button[aria-label=\\"Refresh repository\\"]").disabled'),
                     'coordinator finished both inspection/history phases', timeout=20)
            stop_current_subscription(desktop, selection)
            baseline_count = events(desktop)['total']
            missed_started = time.perf_counter()
            (linked / 'README.txt').write_text('Missed notification requires periodic fallback\n', encoding='utf-8')
            expected = fixture.snapshot()
            expected_linked = fixture.run('status', '--porcelain=v2', '-z', cwd=linked)
            time.sleep(3)
            assert events(desktop)['total'] == baseline_count, 'Missed-event control still received notifications'
            assert not body_contains(desktop, 'README.txt'), 'Missed-event control refreshed before fallback'
            wait_for(lambda: body_contains(desktop, 'README.txt'), '30-second fallback repairs the missed notification', timeout=35)
            recovery_seconds = time.perf_counter() - missed_started
            assert 20 <= recovery_seconds <= 40, f'Unexpected periodic recovery timing: {recovery_seconds:.3f}s'
            assert fixture.snapshot() == expected
            assert fixture.run('status', '--porcelain=v2', '-z', cwd=linked) == expected_linked
            for event in events(desktop)['events']:
                assert set(event) == {'scope', 'subscriptionId', 'sequence', 'reason'}
                assert set(event['scope']) == {'repoId', 'worktreeId', 'generation'}
                assert event['reason'] in ('changed', 'overflow', 'unavailable')
                assert isinstance(event['sequence'], str) and event['sequence'].isdigit()
            desktop.script('window.__TAURI_EVENT_PLUGIN_INTERNALS__.unregisterListener(arguments[0],arguments[1])', EVENT, observer_id)
            result = desktop.command('POST', '/execute/async', {'script': '''
                const done=arguments[arguments.length-1];
                window.__TAURI_INTERNALS__.invoke('plugin:event|unlisten',{event:arguments[0],eventId:arguments[1]})
                    .then(()=>done(true),()=>done(false));
            ''', 'args': [EVENT, observer_id]})
            assert result is True
            desktop.script('window.__nativeWatchAcks.restore()')
            print(f'PASS: actual lost-subscription negative control recovers in {recovery_seconds:.3f}s through 30-second fallback; events are path-free', flush=True)
