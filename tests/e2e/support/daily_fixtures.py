"""Owned local remotes and publication states for daily-action native checks.

All Git writes inherit HistoryFixture's path boundary and argument-array runner.
This creates real refs/objects; it does not replace application IPC or results.
"""
from pathlib import Path

from history_fixtures import HistoryFixture


class DailyFixture(HistoryFixture):
    def __init__(self, workspace: Path):
        super().__init__(workspace, 40)
        self.remote = self.root / 'origin.git'
        self.peer = self.root / 'peer'

    def create(self):
        super().create()
        self.ancestor = self.tips['main']
        self.run('clone', '--bare', '--', str(self.source), str(self.remote))
        self.run('update-ref', 'refs/heads/stale', self.ancestor, cwd=self.remote)
        self.run('remote', 'add', 'origin', str(self.remote))
        self.run('fetch', '--no-tags', 'origin')
        self.run('branch', '--set-upstream-to=origin/main', 'main')
        self.run('update-ref', '-d', 'refs/heads/stale', cwd=self.remote)
        self.commit_file(self.source, 'local-only.txt', 'Local-only native fixture\n',
                         'Local-only publication context')
        self.local_only = self.run('rev-parse', 'HEAD').decode().strip()
        self.run('branch', 'local/same-tip', self.local_only)
        self.run('clone', '--', str(self.remote), str(self.peer))
        self.run('config', 'user.name', 'OpenGit Fixture', cwd=self.peer)
        self.run('config', 'user.email', 'fixture@invalid.example', cwd=self.peer)
        assert self.local_only != self.ancestor
        assert self.run('rev-parse', 'refs/remotes/origin/stale').decode().strip() == self.ancestor
        assert self.run('for-each-ref', '--format=%(refname)', 'refs/heads/stale',
                        cwd=self.remote) == b''
        return self

    def commit_file(self, location: Path, name: str, contents: str, message: str):
        target = (location / name).resolve()
        if not target.is_relative_to(self.root) or not target.is_relative_to(location.resolve()):
            raise RuntimeError('Refusing file outside owned repository')
        target.write_text(contents, encoding='utf-8')
        self.run('add', '--', name, cwd=location)
        self.run('commit', '-m', message, cwd=location)
        return self.run('rev-parse', 'HEAD', cwd=location).decode().strip()

    def advance_remote(self):
        """Create divergence and a new branch without touching app worktree/index."""
        self.remote_tip = self.commit_file(self.peer, 'remote-only.txt',
                                          'Remote-only native fixture\n',
                                          'Remote-only fetch context')
        self.run('push', 'origin', 'HEAD:refs/heads/main', cwd=self.peer)
        self.run('push', 'origin', 'HEAD:refs/heads/new-on-remote', cwd=self.peer)
        assert self.run('rev-parse', 'refs/heads/main', cwd=self.remote).decode().strip() == self.remote_tip
        assert self.run('rev-parse', 'refs/remotes/origin/main').decode().strip() == self.ancestor
        return self.remote_tip

    def containing_branches(self, oid: str):
        return sorted(self.run('for-each-ref', '--contains=' + oid,
                               '--format=%(refname)%00%(symref)',
                               'refs/heads/', 'refs/remotes/').decode().splitlines())

    def protected_source(self):
        """Fetch must preserve these independently observed local values."""
        return {'status': self.run('status', '--porcelain=v2', '-z'),
                'index': (self.source / '.git/index').read_bytes(),
                'head': self.run('rev-parse', 'HEAD'),
                'tags': self.run('for-each-ref', '--format=%(refname) %(objectname)', 'refs/tags/'),
                'localFile': (self.source / 'local-only.txt').read_bytes()}
