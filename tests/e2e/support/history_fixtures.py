"""Owned Git history for native acceptance and reproducible measurements.

No server or credentials. All writes and cleanup remain below the unique
workspace .tmp directory, never the developer's repository.
"""
from __future__ import annotations

import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tempfile
import time


class HistoryFixture:
    def __init__(self, workspace: Path, commits: int = 650):
        if not 40 <= commits <= 100_000:
            raise ValueError('Fixture size outside the supported test range')
        self.workspace = workspace.resolve()
        temporary = self.workspace / '.tmp'
        temporary.mkdir(exist_ok=True)
        self.root = Path(tempfile.mkdtemp(prefix='opengit-history-', dir=temporary)).resolve()
        self.source = self.root / 'history'
        self.git = shutil.which('git')
        if not self.git:
            raise RuntimeError('Git is required')
        self.env = {**os.environ, 'GIT_CONFIG_GLOBAL': os.devnull,
                    'GIT_CONFIG_SYSTEM': os.devnull, 'GIT_CONFIG_COUNT': '0',
                    'GIT_AUTHOR_NAME': 'OpenGit Fixture',
                    'GIT_AUTHOR_EMAIL': 'fixture@invalid.example',
                    'GIT_COMMITTER_NAME': 'OpenGit Fixture',
                    'GIT_COMMITTER_EMAIL': 'fixture@invalid.example',
                    'GIT_TERMINAL_PROMPT': '0'}
        self.commits = commits
        self.created_seconds = None

    def run(self, *arguments: str, cwd: Path | None = None, data: bytes | None = None) -> bytes:
        location = (cwd or self.source).resolve()
        if not location.is_relative_to(self.root):
            raise RuntimeError('Refusing Git command outside the owned fixture')
        result = subprocess.run([self.git, '-C', str(location), '-c', 'commit.gpgSign=false',
                                 *arguments], env=self.env, input=data, capture_output=True,
                                timeout=90, creationflags=0x08000000 if os.name == 'nt' else 0)
        if result.returncode:
            # Never include command payloads in diagnostics.
            raise RuntimeError('Owned fixture Git command failed: ' + arguments[0])
        return result.stdout

    @staticmethod
    def _inline(path: str, contents: bytes, mode: str = '100644') -> bytes:
        quoted = ('"' + ''.join(chr(byte) if 32 <= byte < 127 and chr(byte) not in '\\"'
                                else f'\\{byte:03o}' for byte in path.encode()) + '"')
        return f'M {mode} inline {quoted}\ndata {len(contents)}\n'.encode() + contents + b'\n'

    @staticmethod
    def _commit(ref: str, mark: int, message: str, modifications: bytes,
                parent: int | None = None, merges: tuple[int, ...] = (), author='OpenGit Fixture') -> bytes:
        body = message.encode()
        # Deliberately nonmonotonic dates: date sorting cannot substitute for DAG order.
        timestamp = 1_700_000_000 + (mark * 37) % 113
        value = (f'commit {ref}\nmark :{mark}\nauthor {author} <fixture@invalid.example> '
                 f'{timestamp} +0000\ncommitter OpenGit Fixture <fixture@invalid.example> '
                 f'{timestamp} +0000\ndata {len(body)}\n').encode() + body + b'\n'
        if parent is not None:
            value += f'from :{parent}\n'.encode()
        for merge in merges:
            value += f'merge :{merge}\n'.encode()
        return value + modifications + b'\n'

    def create(self):
        started = time.perf_counter()
        self.source.mkdir()
        self.run('init', '-b', 'main', '--object-format=sha1')
        stream = bytearray()
        for index in range(1, self.commits + 1):
            message = 'DeepNeedle earliest history' if index == 1 else f'History fixture commit {index:06}'
            author = 'Deep Search Fixture' if index == 2 else 'OpenGit Fixture'
            changes = self._inline('README.txt', f'History fixture revision {index}\n'.encode())
            if index == 1:
                changes += self._inline('binary.dat', b'\x00\x01owned binary fixture\xff')
                changes += self._inline('unicode Ω.txt', 'Unicode fixture\n'.encode())
                changes += self._inline('delete-me.txt', b'Delete fixture\n')
                changes += self._inline('whitespace.txt', b'Same line\n')
                changes += self._inline('link.txt', b'README.txt', '120000')
            if index == 3:
                changes += b'D delete-me.txt\n'
            if index == 4:
                changes += b'R "unicode \\316\\251.txt" "renamed \\316\\251.txt"\n'
            if index == 5:
                changes += self._inline('whitespace.txt', b'Same line  \n')
            stream.extend(self._commit('refs/heads/main', index, message, changes,
                                       index - 1 if index > 1 else None, author=author))
        split = self.commits // 2
        feature, other, merged = self.commits + 1, self.commits + 2, self.commits + 3
        stream.extend(self._commit('refs/heads/feature/alpha', feature, 'FeatureOutsideSolo needle',
                                   self._inline('feature.txt', b'Feature change\n'), split))
        stream.extend(self._commit('refs/heads/feature/beta', other, 'Second side branch',
                                   self._inline('other.txt', b'Other change\n'), split - 3))
        stream.extend(self._commit('refs/heads/main', merged, 'Octopus merge fixture',
                                   self._inline('feature.txt', b'Feature change\n') +
                                   self._inline('other.txt', b'Other change\n'),
                                   self.commits, (feature, other)))
        stream.extend(self._commit('refs/heads/topic/unmerged', self.commits + 4,
                                   'UnmergedNeedle outside visible history',
                                   self._inline('unmerged.txt', b'Unmerged change\n'), split))
        stream.extend(self._commit('refs/heads/orphan/root', self.commits + 5,
                                   'Unrelated root fixture',
                                   self._inline('orphan.txt', b'Unrelated history\n')))
        stream.extend(b'done\n')
        self.run('fast-import', '--quiet', data=bytes(stream))
        self.run('checkout', '-f', 'main')
        self.run('tag', '-a', 'v-fixture', '-m', 'Owned annotated tag', 'main~5')
        self.run('update-ref', 'refs/remotes/origin/main', 'main')
        self.run('symbolic-ref', 'refs/remotes/origin/HEAD', 'refs/remotes/origin/main')
        self.run('config', 'user.name', 'OpenGit Fixture')
        self.run('config', 'user.email', 'fixture@invalid.example')
        self.tips = {name: self.run('rev-parse', name).decode().strip()
                     for name in ('main', 'feature/alpha', 'feature/beta', 'topic/unmerged',
                                  'orphan/root', 'v-fixture^{}')}
        self.oracle = self.run('rev-list', '--topo-order', '--parents', 'main',
                               'feature/alpha', 'feature/beta', 'topic/unmerged',
                               'orphan/root').decode().splitlines()
        assert len(self.oracle) == self.commits + 5
        self.run('fsck', '--no-reflogs')
        self.created_seconds = time.perf_counter() - started
        return self

    def linked_worktree(self) -> Path:
        path = self.root / 'linked'
        self.run('worktree', 'add', '--detach', str(path), 'main~10')
        return path

    def shallow_clone(self) -> Path:
        path = self.root / 'shallow'
        self.run('clone', '--depth', '5', '--no-local', '--', self.source.as_uri(), str(path))
        assert self.run('rev-parse', '--is-shallow-repository', cwd=path).strip() == b'true'
        return path

    def snapshot(self):
        return (self.run('for-each-ref', '--format=%(refname) %(objectname)'),
                self.run('status', '--porcelain=v2', '-z'),
                (self.source / '.git/index').read_bytes())

    def close(self):
        target = self.root.resolve()
        parent = (self.workspace / '.tmp').resolve()
        if target.parent != parent or not target.name.startswith('opengit-history-'):
            raise RuntimeError('Refusing fixture cleanup outside owned root')
        if target.exists():
            def retry_readonly(function, name, failure):
                failure = failure[1] if isinstance(failure, tuple) else failure
                path = Path(name).resolve()
                if not isinstance(failure, PermissionError) or not path.is_relative_to(target):
                    raise failure
                path.chmod(path.stat().st_mode | stat.S_IWRITE)
                function(name)
            option = {'onexc' if sys.version_info >= (3, 12) else 'onerror': retry_readonly}
            shutil.rmtree(target, **option)

    def __enter__(self):
        try:
            return self.create()
        except BaseException:
            self.close()
            raise

    def __exit__(self, *_):
        self.close()
