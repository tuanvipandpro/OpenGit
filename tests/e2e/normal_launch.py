"""Launch production without WebDriver or a debugging-port environment."""
import ctypes
from ctypes import wintypes as w
import os
from pathlib import Path
import subprocess
import time
import uuid

ROOT = Path(__file__).resolve().parents[2]


if __name__ == '__main__':
    directory = ROOT / '.tmp' / ('normal-launch-' + uuid.uuid4().hex)
    directory.mkdir()
    env = dict(os.environ)
    for key in ('WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS', 'WEBVIEW2_PIPE_FOR_SCRIPT_DEBUGGER',
                'OPENGIT_E2E_DIALOGS_JSON', 'OPENGIT_INTERNAL_ASKPASS',
                'OPENGIT_ASKPASS_PIPE', 'OPENGIT_ASKPASS_NONCE'):
        env.pop(key, None)
    env.update(OPENGIT_DATA_DIR=str(directory / 'data'),
               WEBVIEW2_USER_DATA_FOLDER=str(directory / 'webview'))
    user = ctypes.WinDLL('user32', use_last_error=True)
    callback_type = ctypes.WINFUNCTYPE(w.BOOL, w.HWND, w.LPARAM)
    user.EnumWindows.argtypes = [callback_type, w.LPARAM]
    user.GetWindowThreadProcessId.argtypes = [w.HWND, ctypes.POINTER(w.DWORD)]
    user.GetWindowTextW.argtypes = [w.HWND, w.LPWSTR, ctypes.c_int]
    user.PostMessageW.argtypes = [w.HWND, w.UINT, w.WPARAM, w.LPARAM]
    with (directory / 'runtime.log').open('wb') as log:
        process = subprocess.Popen([str(ROOT / 'src-tauri/target/release/opengit.exe')],
                                   env=env, stdout=log, stderr=subprocess.STDOUT,
                                   creationflags=0x08000000)
        try:
            windows = []
            @callback_type
            def find_window(handle, _):
                owner = w.DWORD()
                user.GetWindowThreadProcessId(handle, ctypes.byref(owner))
                if owner.value == process.pid:
                    title = ctypes.create_unicode_buffer(512)
                    user.GetWindowTextW(handle, title, len(title))
                    if title.value == 'OpenGit':
                        windows.append(handle)
                return True
            deadline = time.monotonic() + 25
            while time.monotonic() < deadline and not windows:
                assert process.poll() is None, 'Production exited before creating its window'
                user.EnumWindows(find_window, 0)
                if not windows:
                    time.sleep(.2)
            assert windows, 'Production main window was not created'
            while time.monotonic() < deadline and not (directory / 'data/workspace.sqlite3').is_file():
                assert process.poll() is None, 'Production exited during initialization'
                time.sleep(.2)
            assert (directory / 'data/workspace.sqlite3').is_file()
            time.sleep(2)
            owner = w.DWORD()
            user.GetWindowThreadProcessId(windows[0], ctypes.byref(owner))
            assert owner.value == process.pid
            assert user.PostMessageW(windows[0], 0x0010, 0, 0)  # WM_CLOSE, owned GUI only.
            assert process.wait(timeout=15) == 0
        finally:
            if process.poll() is None:
                process.terminate()
                process.wait(timeout=10)
    contents = (directory / 'runtime.log').read_bytes()
    assert b'DevTools listening' not in contents, 'Unexpected debug listener on ordinary launch'
    print('PASS: ordinary production launch/window/database/graceful close; no DevTools listener log')
    print('Runtime diagnostics:', directory / 'runtime.log')
