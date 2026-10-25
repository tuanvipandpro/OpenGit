"""Read-only observers for native history UI acceptance.

The Worker wrapper forwards to the actual browser constructor. It records
construction/events without supplying any worker implementation or responses.
SQLite observation avoids advancing the renderer's scoped IPC generation.
"""
import json
from contextlib import closing
from pathlib import Path
import sqlite3


def observe_workers(desktop):
    desktop.script('''
        if(window.__nativeHistoryWorkers)throw new Error('Probe already installed');
        const result={urls:[],messages:0,errors:0};
        window.__nativeHistoryWorkers=result;
        const ActualWorker=window.Worker;
        window.Worker=new Proxy(ActualWorker,{construct(target,args){
            const worker=Reflect.construct(target,args);
            result.urls.push(String(args[0]));
            worker.addEventListener('message',()=>result.messages++);
            worker.addEventListener('error',()=>result.errors++);
            worker.addEventListener('messageerror',()=>result.errors++);
            return worker;
        }});
    ''')


def worker_observation(desktop):
    return desktop.script('return window.__nativeHistoryWorkers')


def stored_view(data: Path, repo_id: str):
    """Observe only the owned profile; never change its schema or preferences."""
    database = (data / 'workspace.sqlite3').resolve()
    if not database.is_file():
        return None
    try:
        with closing(sqlite3.connect(database.as_uri() + '?mode=ro', uri=True, timeout=1)) as connection:
            row = connection.execute('SELECT revision,value FROM repository_views WHERE repo_id=?',
                                     (repo_id,)).fetchone()
        return {'revision': row[0], 'view': json.loads(row[1])} if row else None
    except sqlite3.OperationalError:
        # Startup/schema creation or a transient writer can still be pending.
        # Acceptance's bounded wait fails if this never becomes observable.
        return None
