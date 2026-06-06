#!/usr/bin/env python3
"""Move trunk's inline WASM bootstrap into an external module file.

Trunk emits its bootstrap as an INLINE ``<script type="module">``. The app's
strict CSP (``script-src 'self'`` — no ``'unsafe-inline'``, added in willow #175)
blocks inline scripts, so when the trunk ``dist`` is served as static files (the
infra deploy) the bootstrap never runs, ``init()`` never fires, and the page is
stuck on "Loading Willow…". (Dev/e2e use ``trunk serve``, and the old live site is
a pre-CSP build, so neither hit this.)

An *external* same-origin module script is permitted by ``script-src 'self'`` with
no CSP weakening, so we lift the inline bootstrap verbatim into ``trunk-bootstrap.js``
and reference it by ``src``. Behaviour is identical (module scripts support the
top-level ``await`` the bootstrap uses).

Usage: ``externalize-bootstrap.py <path-to-dist/index.html>``
"""
import pathlib
import re
import sys

index = pathlib.Path(sys.argv[1])
html = index.read_text()

m = re.search(r'<script type="module">(.*?)</script>', html, re.S)
if not m:
    sys.exit(f"externalize-bootstrap: no inline module script in {index}")

(index.parent / "trunk-bootstrap.js").write_text(m.group(1))
html = html.replace(
    m.group(0),
    '<script type="module" src="/trunk-bootstrap.js"></script>',
    1,
)
index.write_text(html)
print("externalize-bootstrap: inline bootstrap -> /trunk-bootstrap.js")
