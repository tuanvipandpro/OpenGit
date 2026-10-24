"""Owned loopback CONNECT proxy which always rejects authentication.

No request payload/header/URL is logged; only a request count is exposed.
"""
import socketserver
import threading


class RejectingProxy:
    def __init__(self):
        self._count = 0
        self._lock = threading.Lock()
        fixture = self

        class Handler(socketserver.BaseRequestHandler):
            def handle(self):
                self.request.settimeout(5)
                request = bytearray()
                while b'\r\n\r\n' not in request and len(request) < 16384:
                    piece = self.request.recv(min(4096, 16384 - len(request)))
                    if not piece:
                        return
                    request.extend(piece)
                if request.startswith(b'CONNECT '):
                    with fixture._lock:
                        fixture._count += 1
                self.request.sendall(b'HTTP/1.1 407 Proxy Authentication Required\r\n'
                    b'Proxy-Authenticate: Basic realm="OpenGit isolated fixture"\r\n'
                    b'Content-Length: 0\r\nConnection: close\r\n\r\n')

        class Server(socketserver.ThreadingTCPServer):
            daemon_threads = True

        self.server = Server(('127.0.0.1', 0), Handler)
        self.url = f'http://127.0.0.1:{self.server.server_address[1]}'
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)

    @property
    def connect_requests(self):
        with self._lock:
            return self._count

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, *_):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=5)
