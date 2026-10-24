"""Disposable real HTTPS/SSH Git remotes for local acceptance tests.

Passwords and server keys are generated per run; no corporate credentials,
global trust changes, or provider/SSO claims. Import with asyncssh and
cryptography installed in the test interpreter. Clients must trust ca_file
explicitly and use the isolated known_hosts path for SSH.
"""
from __future__ import annotations

import asyncio
import base64
import datetime as dt
import hmac
import http.server
import ipaddress
import os
from pathlib import Path
import secrets
import shutil
import ssl
import subprocess
import tempfile
import threading
import time
from urllib.parse import urlsplit

import asyncssh
from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.x509.oid import NameOID


class LiveRemotes:
    def __init__(self, workspace: Path):
        self.workspace = workspace.resolve()
        temp_parent = self.workspace / '.tmp'
        temp_parent.mkdir(exist_ok=True)
        if not temp_parent.resolve().is_relative_to(self.workspace):
            raise RuntimeError('Fixture parent escaped workspace')
        self._temp = tempfile.TemporaryDirectory(prefix='opengit-live-', dir=temp_parent)
        self.root = Path(self._temp.name).resolve()
        if not self.root.is_relative_to(temp_parent.resolve()):
            raise RuntimeError('Invalid owned fixture directory')
        self.username = 'fixture'
        self.password = secrets.token_urlsafe(32)
        self.key_passphrase = secrets.token_urlsafe(32)
        self._client_key = asyncssh.generate_private_key('ssh-ed25519')
        self.client_key_file = self.root / 'encrypted-client-key'
        self.client_key_file.write_bytes(self._client_key.export_private_key(
            'openssh', passphrase=self.key_passphrase))
        self.git = shutil.which('git')
        if not self.git:
            raise RuntimeError('Git is required for acceptance fixtures')
        self.env = {**os.environ, 'GIT_CONFIG_GLOBAL': os.devnull,
                    'GIT_CONFIG_SYSTEM': os.devnull, 'GIT_TERMINAL_PROMPT': '0'}
        self.source = self.root / 'source'
        self._git('init', '-b', 'main', str(self.source))
        (self.source / 'README.txt').write_text('OpenGit isolated transport fixture\n', encoding='utf-8')
        self._git('-C', str(self.source), 'add', '--', 'README.txt')
        self._git('-C', str(self.source), '-c', 'user.name=OpenGit Fixture',
                  '-c', 'user.email=fixture@invalid.example', 'commit', '-m', 'Fixture')
        self.repository = self.root / 'repo.git'
        self._git('clone', '--bare', '--', str(self.source), str(self.repository))
        self.ca_file = self.root / 'fixture-ca.pem'
        self.known_hosts = self.root / 'known_hosts'
        self.known_hosts.touch()
        self._http = None
        self._loop = None
        self._ssh = None
        self._threads = []
        self._agent = None
        self.allow_anonymous = False
        self._request_lock = threading.Lock()
        self._https_requests = 0

    def _git(self, *args):
        return subprocess.run([self.git, *args], env=self.env, check=True,
                              capture_output=True, creationflags=0x08000000 if os.name == 'nt' else 0)

    def start(self):
        key = ec.generate_private_key(ec.SECP256R1())
        name = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, 'OpenGit local fixture')])
        now = dt.datetime.now(dt.timezone.utc)
        cert = (x509.CertificateBuilder().subject_name(name).issuer_name(name)
                .public_key(key.public_key()).serial_number(x509.random_serial_number())
                .not_valid_before(now - dt.timedelta(minutes=5))
                .not_valid_after(now + dt.timedelta(days=1))
                .add_extension(x509.BasicConstraints(ca=True, path_length=0), critical=True)
                .add_extension(x509.SubjectAlternativeName([
                    x509.DNSName('localhost'), x509.IPAddress(ipaddress.ip_address('127.0.0.1'))]), critical=False)
                .sign(key, hashes.SHA256()))
        self.ca_file.write_bytes(cert.public_bytes(serialization.Encoding.PEM))
        key_path = self.root / 'transient-server-key.pem'
        key_path.write_bytes(key.private_bytes(serialization.Encoding.PEM,
                            serialization.PrivateFormat.PKCS8, serialization.NoEncryption()))
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.load_cert_chain(self.ca_file, key_path)
        key_path.unlink()  # Only the TLS server retains the synthetic key in memory.
        fixture = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass  # Never print auth headers, URLs, request bodies or credentials.

            def do_GET(self):
                self.serve_git()

            def do_POST(self):
                self.serve_git()

            def serve_git(self):
                with fixture._request_lock:
                    fixture._https_requests += 1
                expected = 'Basic ' + base64.b64encode(
                    (fixture.username + ':' + fixture.password).encode()).decode()
                if not fixture.allow_anonymous and not hmac.compare_digest(
                        self.headers.get('Authorization', ''), expected):
                    self.send_response(401)
                    self.send_header('WWW-Authenticate', 'Basic realm="OpenGit fixture"')
                    self.send_header('Content-Length', '0')
                    self.end_headers()
                    return
                parsed = urlsplit(self.path)
                if parsed.path not in ('/repo.git/info/refs', '/repo.git/git-upload-pack'):
                    self.send_error(404)
                    return
                length = int(self.headers.get('Content-Length', '0'))
                if not 0 <= length <= 8 * 1024 * 1024:
                    self.send_error(413)
                    return
                body = self.rfile.read(length)
                env = {**fixture.env, 'GIT_PROJECT_ROOT': str(fixture.root),
                       'GIT_HTTP_EXPORT_ALL': '1', 'PATH_INFO': parsed.path,
                       'QUERY_STRING': parsed.query, 'REQUEST_METHOD': self.command,
                       'CONTENT_TYPE': self.headers.get('Content-Type', ''),
                       'CONTENT_LENGTH': str(length), 'REMOTE_USER': fixture.username,
                       'REMOTE_ADDR': '127.0.0.1', 'SERVER_PROTOCOL': 'HTTP/1.1'}
                response = subprocess.run([fixture.git, 'http-backend'], input=body,
                    capture_output=True, env=env, timeout=30,
                    creationflags=0x08000000 if os.name == 'nt' else 0)
                header, separator, content = response.stdout.partition(b'\r\n\r\n')
                if not separator:
                    header, separator, content = response.stdout.partition(b'\n\n')
                if response.returncode or not separator:
                    self.send_error(500)
                    return
                fields = []
                status = 200
                for line in header.decode('ascii').splitlines():
                    k, v = line.split(':', 1)
                    if k.lower() == 'status':
                        status = int(v.strip().split()[0])
                    else:
                        fields.append((k, v.strip()))
                self.send_response(status)
                for k, v in fields:
                    self.send_header(k, v)
                self.send_header('Content-Length', str(len(content)))
                self.end_headers()
                self.wfile.write(content)

        self._http = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self._http.socket = context.wrap_socket(self._http.socket, server_side=True)
        self.https_url = f'https://127.0.0.1:{self._http.server_port}/repo.git'
        http_thread = threading.Thread(target=self._http.serve_forever, daemon=True)
        http_thread.start()
        self._threads.append(http_thread)
        self._loop = asyncio.new_event_loop()
        ready = threading.Event()
        failure = []

        class Server(asyncssh.SSHServer):
            def begin_auth(self, username):
                return not fixture.allow_anonymous

            def password_auth_supported(self):
                return True

            def public_key_auth_supported(self):
                return True

            def validate_public_key(self, username, key):
                return hmac.compare_digest(username, fixture.username) and hmac.compare_digest(
                    key.get_fingerprint(), fixture._client_key.get_fingerprint())

            def validate_password(self, username, password):
                return hmac.compare_digest(username, fixture.username) and hmac.compare_digest(password, fixture.password)

        async def serve(process):
            if process.command != "git-upload-pack '/repo.git'":
                process.exit(1)
                return
            child = await asyncio.create_subprocess_exec(fixture.git, 'upload-pack', str(fixture.repository),
                stdin=asyncio.subprocess.PIPE, stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.PIPE, env=fixture.env,
                creationflags=0x08000000 if os.name == 'nt' else 0)

            async def incoming():
                try:
                    while data := await process.stdin.read(65536):
                        child.stdin.write(data)
                        await child.stdin.drain()
                finally:
                    child.stdin.close()

            async def outgoing(reader, writer):
                while data := await reader.read(65536):
                    writer.write(data)

            try:
                input_task = asyncio.create_task(incoming())
                await asyncio.gather(outgoing(child.stdout, process.stdout),
                                     outgoing(child.stderr, process.stderr))
                code = await child.wait()
                input_task.cancel()
                await asyncio.gather(input_task, return_exceptions=True)
                process.exit(code)
            finally:
                if child.returncode is None:
                    child.kill()
                    await child.wait()

        async def setup_ssh():
            host_key = asyncssh.generate_private_key('ssh-ed25519')
            self.host_fingerprint = host_key.get_fingerprint()
            self.server_public_key = host_key.convert_to_public()
            self._ssh = await asyncssh.create_server(Server, '127.0.0.1', 0,
                server_host_keys=[host_key], process_factory=serve, encoding=None)
            self.ssh_url = f'ssh://{self.username}@127.0.0.1:{self._ssh.get_port()}/repo.git'

        def ssh_thread():
            asyncio.set_event_loop(self._loop)
            try:
                self._loop.run_until_complete(setup_ssh())
            except Exception as e:
                failure.append(type(e).__name__)
            finally:
                ready.set()
            if not failure:
                self._loop.run_forever()

        thread = threading.Thread(target=ssh_thread, daemon=True)
        thread.start()
        self._threads.append(thread)
        if not ready.wait(20) or failure:
            self.close()
            raise RuntimeError('SSH fixture could not start')
        return self

    def close(self):
        if self._agent and self._agent.poll() is None:
            self._agent.terminate()
            self._agent.wait(timeout=10)
        if self._http:
            self._http.shutdown()
            self._http.server_close()
        if self._loop and self._ssh:
            async def stop():
                self._ssh.close()
                await self._ssh.wait_closed()
            asyncio.run_coroutine_threadsafe(stop(), self._loop).result(timeout=10)
            self._loop.call_soon_threadsafe(self._loop.stop)
        for thread in self._threads:
            thread.join(timeout=5)
        # Verify the absolute owned target before TemporaryDirectory recursively cleans it.
        parent = (self.workspace / '.tmp').resolve()
        if self.root.parent != parent or not self.root.name.startswith('opengit-live-'):
            raise RuntimeError('Refusing cleanup outside owned fixture root')
        self._temp.cleanup()

    def start_agent(self):
        """Own a dedicated Git-for-Windows agent with only the generated fixture key."""
        git_root = Path(self.git).parent.parent
        agent = git_root / 'usr/bin/ssh-agent.exe'
        add = git_root / 'usr/bin/ssh-add.exe'
        ssh = git_root / 'usr/bin/ssh.exe'
        if not all(p.is_file() for p in (agent, add, ssh)):
            raise RuntimeError('Git-for-Windows agent test tools unavailable')
        socket_path = self.root / 'agent.sock'
        self._agent = subprocess.Popen([str(agent), '-D', '-a', socket_path.as_posix()],
            env=self.env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            creationflags=0x08000000 if os.name == 'nt' else 0)
        env = {**self.env, 'SSH_AUTH_SOCK': socket_path.as_posix()}
        for _ in range(80):
            if socket_path.exists():
                break
            if self._agent.poll() is not None:
                raise RuntimeError('Owned fixture agent failed to start')
            time.sleep(.1)
        transient = self.root / 'agent-loading-key'
        transient.write_bytes(self._client_key.export_private_key('openssh'))
        try:
            loaded = subprocess.run([str(add), str(transient)], env=env, capture_output=True,
                timeout=20, creationflags=0x08000000 if os.name == 'nt' else 0)
            if loaded.returncode:
                raise RuntimeError('Generated fixture key could not be loaded into owned agent')
        finally:
            transient.unlink(missing_ok=True)
        return ssh, env

    @property
    def https_requests(self):
        with self._request_lock:
            return self._https_requests

    def __enter__(self):
        return self.start()

    def __exit__(self, *_):
        self.close()
