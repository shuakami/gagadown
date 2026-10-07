#!/usr/bin/env python3
"""Range-capable test server. Per-connection throttle simulates CDNs that cap each TCP stream.
usage: testserver.py PORT SIZE_MB PER_CONN_KBPS [FAIL_RATE]"""
import os, sys, time, random, hashlib, socket
from http.server import ThreadingHTTPServer, BaseHTTPRequestHandler

port, size_mb, kbps = int(sys.argv[1]), int(sys.argv[2]), int(sys.argv[3])
fail = float(sys.argv[4]) if len(sys.argv) > 4 else 0.0
random.seed(1)
DATA = random.randbytes(size_mb * 1024 * 1024)
print("sha256", hashlib.sha256(DATA).hexdigest(), flush=True)

class H(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    def log_message(self, *a): pass
    def do_GET(self):
        total = len(DATA)
        rng = self.headers.get("Range")
        start, end = 0, total - 1
        if self.path.startswith("/norange"):
            rng = None
        if rng and rng.startswith("bytes="):
            a, b = rng[6:].split("-")
            start = int(a); end = int(b) if b else total - 1
            end = min(end, total - 1)
            self.send_response(206)
            self.send_header("Content-Range", f"bytes {start}-{end}/{total}")
        else:
            self.send_response(200)
        self.send_header("Content-Length", str(end - start + 1))
        self.send_header("Content-Type", "application/octet-stream")
        self.send_header("ETag", '"fixed-etag"')
        self.send_header("Content-Disposition", 'attachment; filename="test.bin"')
        self.end_headers()
        pos, step = start, 16384
        delay = step / (kbps * 1024) if kbps > 0 else 0
        try:
            while pos <= end:
                if fail and random.random() < fail:
                    self.close_connection = True
                    self.connection.shutdown(socket.SHUT_RDWR); return
                n = min(step, end - pos + 1)
                self.wfile.write(DATA[pos:pos + n]); pos += n
                if delay: time.sleep(delay)
        except (BrokenPipeError, ConnectionResetError):
            pass

ThreadingHTTPServer.daemon_threads = True
ThreadingHTTPServer(("127.0.0.1", port), H).serve_forever()
