"""Native Tauri/WebView2 smoke via tauri-driver; no mocked browser runtime."""
from __future__ import annotations

import argparse
import base64
import json
import os
from pathlib import Path
import socket
import subprocess
import time
import urllib.error
import urllib.request
import uuid


def free_port():
    with socket.socket() as s:
        s.bind(('127.0.0.1', 0))
        return s.getsockname()[1]


class DesktopSession:
    def __init__(self, root: Path, executable: Path, extra_env=None):
        self.root = root.resolve()
        self.executable = executable.resolve()
        self.directory = self.root / '.tmp' / ('desktop-' + uuid.uuid4().hex)
        self.directory.mkdir(parents=True)
        self.port, native_port = free_port(), free_port()
        self.base = f'http://127.0.0.1:{self.port}'
        self.session = None
        self.log = (self.directory / 'driver.log').open('wb')
        env = {**os.environ, 'WEBVIEW2_USER_DATA_FOLDER': str(self.directory / 'webview'),
               **(extra_env or {})}
        # None explicitly removes inherited switches (Git's NO_VERIFY is
        # enabled by mere presence, even when its text value is "false").
        env = {key: value for key, value in env.items() if value is not None}
        self.process = subprocess.Popen([
            str(self.root / '.tools/cargo/bin/tauri-driver.exe'), '--port', str(self.port),
            '--native-port', str(native_port), '--native-driver',
            str(self.root / '.tools/webdriver/msedgedriver.exe')], env=env,
            stdout=self.log, stderr=subprocess.STDOUT,
            creationflags=0x08000000 if os.name == 'nt' else 0)
        try:
            for _ in range(100):
                try:
                    self.request('GET', '/status')
                    break
                except (OSError, urllib.error.URLError):
                    if self.process.poll() is not None:
                        raise RuntimeError('Native driver exited; inspect owned driver.log')
                    time.sleep(.2)
            value = self.request('POST', '/session', {'capabilities': {'alwaysMatch': {
                'tauri:options': {'application': str(executable.resolve())}}}})
            self.session = value['sessionId']
            self.command('POST', '/timeouts', {'script': 30000, 'pageLoad': 30000})
        except BaseException:
            self.close()
            raise

    def request(self, method, path, data=None):
        payload = json.dumps(data).encode() if data is not None else None
        req = urllib.request.Request(self.base + path, data=payload,
                                     headers={'Content-Type': 'application/json'}, method=method)
        try:
            with urllib.request.urlopen(req, timeout=60) as response:
                return json.load(response)['value']
        except urllib.error.HTTPError as e:
            # Driver errors can include page data; preserve details only in the owned test artifact.
            (self.directory / 'webdriver-error.json').write_bytes(e.read())
            raise RuntimeError('WebDriver request failed; inspect owned error artifact') from None

    def command(self, method, path, data=None):
        return self.request(method, f'/session/{self.session}{path}', data)

    def script(self, script, *args):
        return self.command('POST', '/execute/sync', {'script': script, 'args': list(args)})

    def invoke(self, command, request):
        return self.command('POST', '/execute/async', {
            'script': 'const done=arguments[arguments.length-1]; '
                      'window.__TAURI_INTERNALS__.invoke(arguments[0],{request:arguments[1]})'
                      '.then(done,()=>done({transportRejected:true}));',
            'args': [command, request]})

    def fill(self, selector, text):
        element = self.command('POST', '/element', {'using': 'css selector', 'value': selector})
        element_id = element['element-6066-11e4-a52e-4f735466cecf']
        self.command('POST', f'/element/{element_id}/clear', {})
        self.command('POST', f'/element/{element_id}/value', {'text': text})

    def click(self, selector):
        element = self.command('POST', '/element', {'using': 'css selector', 'value': selector})
        element_id = element['element-6066-11e4-a52e-4f735466cecf']
        self.command('POST', f'/element/{element_id}/click', {})

    def screenshot(self, name='desktop.png'):
        path = self.directory / name
        path.write_bytes(base64.b64decode(self.command('GET', '/screenshot')))
        return path

    def crash_application(self):
        """Terminate only this driver's uniquely identified app descendant."""
        import ctypes
        from ctypes import wintypes as w
        if os.name != 'nt':
            raise RuntimeError('Owned native crash probe requires Windows')

        class Entry(ctypes.Structure):
            _fields_ = [('size', w.DWORD), ('usage', w.DWORD), ('pid', w.DWORD),
                        ('heap', ctypes.c_size_t), ('module', w.DWORD), ('threads', w.DWORD),
                        ('parent', w.DWORD), ('priority', w.LONG), ('flags', w.DWORD),
                        ('name', w.WCHAR * 260)]

        kernel = ctypes.WinDLL('kernel32', use_last_error=True)
        kernel.CreateToolhelp32Snapshot.argtypes = [w.DWORD, w.DWORD]
        kernel.CreateToolhelp32Snapshot.restype = w.HANDLE
        kernel.Process32FirstW.argtypes = kernel.Process32NextW.argtypes = [w.HANDLE, ctypes.POINTER(Entry)]
        kernel.OpenProcess.argtypes = [w.DWORD, w.BOOL, w.DWORD]
        kernel.OpenProcess.restype = w.HANDLE
        kernel.QueryFullProcessImageNameW.argtypes = [w.HANDLE, w.DWORD, w.LPWSTR, ctypes.POINTER(w.DWORD)]
        kernel.TerminateProcess.argtypes = [w.HANDLE, w.UINT]
        kernel.CloseHandle.argtypes = [w.HANDLE]
        snapshot = kernel.CreateToolhelp32Snapshot(2, 0)
        if snapshot == ctypes.c_void_p(-1).value:
            raise RuntimeError('Cannot enumerate owned app process')
        processes = {}
        entry = Entry()
        entry.size = ctypes.sizeof(entry)
        try:
            found = kernel.Process32FirstW(snapshot, ctypes.byref(entry))
            while found:
                processes[entry.pid] = (entry.parent, entry.name)
                found = kernel.Process32NextW(snapshot, ctypes.byref(entry))
        finally:
            kernel.CloseHandle(snapshot)
        descendants = {self.process.pid}
        while True:
            expanded = descendants | {pid for pid, (parent, _) in processes.items() if parent in descendants}
            if expanded == descendants:
                break
            descendants = expanded
        candidates = [pid for pid in descendants if pid in processes and
                      processes[pid][1].casefold() == self.executable.name.casefold()]
        # Askpass is the same executable, nested below the owned GUI process.
        # Select only the outermost candidate; retain the exact-image check below.
        def has_application_ancestor(pid):
            visited = set()
            parent = processes[pid][0]
            while parent in processes and parent not in visited:
                if parent in candidates:
                    return True
                visited.add(parent)
                parent = processes[parent][0]
            return False
        candidates = [pid for pid in candidates if not has_application_ancestor(pid)]
        if len(candidates) != 1:
            raise RuntimeError('Refusing crash without a unique owned application descendant')
        handle = kernel.OpenProcess(0x1001, False, candidates[0])
        if not handle:
            raise RuntimeError('Cannot open owned application process')
        try:
            image = ctypes.create_unicode_buffer(32768)
            count = w.DWORD(len(image))
            if not kernel.QueryFullProcessImageNameW(handle, 0, image, ctypes.byref(count)):
                raise RuntimeError('Cannot verify owned application image')
            if Path(image.value).resolve() != self.executable:
                raise RuntimeError('Refusing crash of an unexpected application image')
            if not kernel.TerminateProcess(handle, 97):
                raise RuntimeError('Owned crash injection failed')
        finally:
            kernel.CloseHandle(handle)

    def close(self):
        if self.session:
            try:
                self.command('DELETE', '')
            except Exception:
                pass
            self.session = None
        if self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=10)
        self.log.close()

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--executable', default='src-tauri/target/release/opengit.exe')
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[2]
    with DesktopSession(root, root / args.executable) as desktop:
        for _ in range(80):
            if desktop.script('return document.body.innerText.includes("OpenGit")'):
                break
            time.sleep(.25)
        assert desktop.script('return document.body.innerText.includes("OpenGit")')
        environment = desktop.invoke('get_environment', {})
        assert environment.get('ok') is True and environment['value']['appVersion'] == '0.1.0'
        assert environment['value']['contractVersion'] == '1'
        invalid = desktop.invoke('get_environment', {'unexpected': 'synthetic-canary'})
        assert invalid.get('transportRejected') or invalid.get('ok') is False
        screenshot = desktop.screenshot()
        print('PASS: native Windows app, WebView2 rendering, typed IPC, invalid request rejection')
        print('Screenshot:', screenshot)
