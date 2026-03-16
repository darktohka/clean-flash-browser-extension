#!/usr/bin/env python3
"""
Three HTTP servers for URLLoader/URLRequest test suite.

Server A (port 3000): Main server — serves SWF, HTML, and same-origin test endpoints.
Server B (port 3001): Cross-origin WITH permissive crossdomain.xml.
Server C (port 3002): Cross-origin WITHOUT any crossdomain.xml.
"""

import http.server
import json
import os
import sys
import threading
import time
import urllib.parse

# ──────────────────────────────────────────────
# Shared endpoint logic
# ──────────────────────────────────────────────

SERVE_DIR = os.path.dirname(os.path.abspath(__file__))

CROSSDOMAIN_PERMISSIVE = b"""\
<?xml version="1.0"?>
<!DOCTYPE cross-domain-policy SYSTEM "http://www.adobe.com/xml/dtds/cross-domain-policy.dtd">
<cross-domain-policy>
    <site-control permitted-cross-domain-policies="all"/>
    <allow-access-from domain="*" />
    <allow-http-request-headers-from domain="*" headers="*"/>
</cross-domain-policy>
"""

CROSSDOMAIN_SUBDIR = b"""\
<?xml version="1.0"?>
<!DOCTYPE cross-domain-policy SYSTEM "http://www.adobe.com/xml/dtds/cross-domain-policy.dtd">
<cross-domain-policy>
    <allow-access-from domain="*" />
    <allow-http-request-headers-from domain="*" headers="*"/>
</cross-domain-policy>
"""


def make_handler(server_name, port, serve_crossdomain):
    """Factory that creates a handler class per-server."""

    class Handler(http.server.BaseHTTPRequestHandler):
        server_version = f"FlashTestServer-{server_name}"

        def log_message(self, fmt, *args):
            print(f"  [{server_name}:{port}] {fmt % args}")

        # ── Routing ──

        def do_GET(self):
            self._route("GET")

        def do_POST(self):
            self._route("POST")

        def do_OPTIONS(self):
            # Preflight for custom headers
            self.send_response(200)
            self.send_header("Access-Control-Allow-Origin", "*")
            self.send_header("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
            self.send_header("Access-Control-Allow-Headers", "*")
            self.end_headers()

        def _route(self, method):
            parsed = urllib.parse.urlparse(self.path)
            path = parsed.path
            query = urllib.parse.parse_qs(parsed.query)

            # Read POST body
            body = b""
            if method == "POST":
                length = int(self.headers.get("Content-Length", 0))
                if length > 0:
                    body = self.rfile.read(length)

            # ── crossdomain.xml ──
            if path == "/crossdomain.xml":
                if serve_crossdomain:
                    self._respond(200, CROSSDOMAIN_PERMISSIVE, "application/xml")
                else:
                    self._respond(404, b"No crossdomain.xml", "text/plain")
                return

            if path == "/subdir/crossdomain.xml":
                if serve_crossdomain:
                    self._respond(200, CROSSDOMAIN_SUBDIR, "application/xml")
                else:
                    self._respond(404, b"Not found", "text/plain")
                return

            # ── Static files (Server A only, or any server for non-SWF) ──
            if path == "/" and port == 3000:
                self._serve_file("test.html", "text/html")
                return
            if path == "/teststick" and port == 3000:
                self._serve_file("teststick.html", "text/html")
                return
            if path == "/filechooser" and port == 3000:
                self._serve_file("filechooser.html", "text/html")
                return
            if path == "/cursorlock" and port == 3000:
                self._serve_file("cursorlock.html", "text/html")
                return
            if path == "/fullscreen" and port == 3000:
                self._serve_file("fullscreen.html", "text/html")
                return
            if path == "/stage3d" and port == 3000:
                self._serve_file("stage3d.html", "text/html")
                return
            if path == "/URLLoaderTests.swf" and port == 3000:
                self._serve_file("URLLoaderTests.swf", "application/x-shockwave-flash")
                return
            if path == "/FileChooserTests.swf" and port == 3000:
                self._serve_file("FileChooserTests.swf", "application/x-shockwave-flash")
                return
            if path == "/CursorLockTests.swf" and port == 3000:
                self._serve_file("CursorLockTests.swf", "application/x-shockwave-flash")
                return
            if path == "/FullscreenTests.swf" and port == 3000:
                self._serve_file("FullscreenTests.swf", "application/x-shockwave-flash")
                return
            if path == "/Stage3DTests.swf" and port == 3000:
                self._serve_file("Stage3DTests.swf", "application/x-shockwave-flash")
                return
            # Serve LoadableChild.swf from all servers (for cross-origin SWF loading tests)
            if path == "/LoadableChild.swf":
                self._serve_file("LoadableChild.swf", "application/x-shockwave-flash")
                return

            # ── Test endpoints ──
            if path == "/text":
                self._respond(200, b"Hello, World!", "text/plain")
            elif path == "/tiny":
                self._respond(200, b"ok", "text/plain")
            elif path == "/json":
                self._respond(200, json.dumps({"status": "ok", "server": server_name}).encode(), "application/json")
            elif path == "/xml":
                self._respond(200, b"<root><msg>ok</msg><server>" + server_name.encode() + b"</server></root>", "text/xml")
            elif path == "/variables":
                self._respond(200, b"name=Flash&version=32&features=a,b,c", "application/x-www-form-urlencoded")
            elif path == "/binary":
                data = bytes(range(256))
                self._respond(200, data, "application/octet-stream")
            elif path == "/echo":
                self._handle_echo(method, query, body)
            elif path == "/echo-vars":
                self._handle_echo_vars(method, query, body)
            elif path.startswith("/status/"):
                self._handle_status(path)
            elif path == "/redirect":
                self.send_response(302)
                self.send_header("Location", f"http://localhost:{port}/text")
                self.send_header("Access-Control-Allow-Origin", "*")
                self.end_headers()
            elif path == "/slow":
                time.sleep(3)
                self._respond(200, b"Slow response complete", "text/plain")
            elif path == "/large":
                # ~1MB of data
                chunk = b"A" * 1000
                data = chunk * 1024  # 1,024,000 bytes
                self._respond(200, data, "text/plain")
            elif path == "/empty":
                self._respond(200, b"", "text/plain")
            elif path == "/invalid-variables":
                # Malformed URL-encoded data
                self._respond(200, b"not&valid=&=broken&=&a", "application/x-www-form-urlencoded")
            elif path == "/subdir/data":
                self._respond(200, b"subdir data OK", "text/plain")
            else:
                self._respond(404, b"Not Found: " + path.encode(), "text/plain")

        # ── Helpers ──

        def _respond(self, code, data, content_type):
            self.send_response(code)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(data)))
            self.send_header("Access-Control-Allow-Origin", "*")
            self.end_headers()
            self.wfile.write(data)

        def _serve_file(self, filename, content_type):
            filepath = os.path.join(SERVE_DIR, filename)
            if os.path.isfile(filepath):
                with open(filepath, "rb") as f:
                    data = f.read()
                self._respond(200, data, content_type)
            else:
                self._respond(404, f"File not found: {filename}".encode(), "text/plain")

        def _handle_echo(self, method, query, body):
            headers_dict = {}
            for key in self.headers:
                headers_dict[key] = self.headers[key]

            response = {
                "method": method,
                "query": query,
                "headers": headers_dict,
                "body": body.decode("utf-8", errors="replace"),
                "contentType": self.headers.get("Content-Type", ""),
                "server": server_name,
            }
            data = json.dumps(response, indent=2).encode()
            self._respond(200, data, "application/json")

        def _handle_echo_vars(self, method, query, body):
            """Echo back POST variables as URL-encoded form."""
            if body:
                # Parse incoming vars and re-encode them
                try:
                    params = urllib.parse.parse_qs(body.decode("utf-8"), keep_blank_values=True)
                    # Flatten single-value lists
                    flat = {}
                    for k, v in params.items():
                        flat[k] = v[0] if len(v) == 1 else v
                    encoded = urllib.parse.urlencode(flat)
                    self._respond(200, encoded.encode(), "application/x-www-form-urlencoded")
                except Exception as e:
                    self._respond(200, body, "application/x-www-form-urlencoded")
            else:
                self._respond(200, b"", "application/x-www-form-urlencoded")

        def _handle_status(self, path):
            try:
                code = int(path.split("/status/")[1])
            except (ValueError, IndexError):
                code = 400
            reason_map = {
                200: "OK", 201: "Created", 204: "No Content",
                301: "Moved Permanently", 302: "Found", 304: "Not Modified",
                400: "Bad Request", 401: "Unauthorized", 403: "Forbidden",
                404: "Not Found", 405: "Method Not Allowed",
                500: "Internal Server Error", 502: "Bad Gateway", 503: "Service Unavailable",
            }
            body = f"Status {code}: {reason_map.get(code, 'Unknown')}".encode()
            if code == 204:
                # 204 must not have body
                self.send_response(204)
                self.send_header("Access-Control-Allow-Origin", "*")
                self.end_headers()
            else:
                self._respond(code, body, "text/plain")

    return Handler


# ──────────────────────────────────────────────
# Server startup
# ──────────────────────────────────────────────

def run_server(name, port, serve_crossdomain):
    handler = make_handler(name, port, serve_crossdomain)
    server = http.server.HTTPServer(("0.0.0.0", port), handler)
    print(f"[{name}] Listening on http://localhost:{port} (crossdomain={'YES' if serve_crossdomain else 'NO'})")
    server.serve_forever()


def main():
    servers = [
        ("A", 3000, False),  # Main server; crossdomain not needed for same-origin
        ("B", 3001, True),   # Cross-origin WITH crossdomain.xml
        ("C", 3002, False),  # Cross-origin WITHOUT crossdomain.xml
    ]

    threads = []
    for name, port, xd in servers:
        t = threading.Thread(target=run_server, args=(name, port, xd), daemon=True)
        t.start()
        threads.append(t)

    print("\nAll servers running. Press Ctrl+C to stop.\n")
    print(f"  Open http://localhost:3000/ in a Flash-capable browser.\n")

    try:
        while True:
            time.sleep(1)
    except KeyboardInterrupt:
        print("\nShutting down.")
        sys.exit(0)


if __name__ == "__main__":
    main()
