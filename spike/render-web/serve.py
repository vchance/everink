#!/usr/bin/env python3
"""Serves www/ on 127.0.0.1 so that the page is cross-origin isolated.

    python3 spike/render-web/serve.py [port]      (default 8765)

A page that is not isolated gets a clock rounded to 0.1 ms in Chrome; an isolated one gets 0.005 ms. The page
prints which it has. Nothing else differs from `python3 -m http.server`.
"""
import functools
import http.server
import pathlib
import sys


class Handler(http.server.SimpleHTTPRequestHandler):
    def end_headers(self):
        self.send_header("Cross-Origin-Opener-Policy", "same-origin")
        self.send_header("Cross-Origin-Embedder-Policy", "require-corp")
        self.send_header("Cache-Control", "no-store")
        super().end_headers()


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8765
    www = pathlib.Path(__file__).resolve().parent / "www"
    handler = functools.partial(Handler, directory=str(www))
    with http.server.ThreadingHTTPServer(("127.0.0.1", port), handler) as server:
        print(f"serving {www} at http://127.0.0.1:{port}/")
        server.serve_forever()
