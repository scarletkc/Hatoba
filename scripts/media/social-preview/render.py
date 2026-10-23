"""Render the repository's social preview from index.html.

usage: python scripts/media/social-preview/render.py [--out FILE]

Needs Playwright's Chromium and `pnpm install` (the page loads the font from the website's
node_modules). Writes docs/media/social-preview.png, which goes to GitHub by hand: the repository's
Settings, General, Social preview.
"""
import argparse
import functools
import http.server
import mimetypes
import threading
from pathlib import Path

from playwright.sync_api import sync_playwright

ROOT = Path(__file__).resolve().parents[3]
PAGE = "scripts/media/social-preview/index.html"
W, H = 1280, 640

for ext, kind in ((".svg", "image/svg+xml"), (".woff2", "font/woff2")):
    mimetypes.add_type(kind, ext)


class Quiet(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *args):
        pass


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", type=Path, default=ROOT / "docs/media/social-preview.png")
    args = ap.parse_args()

    # Served over HTTP rather than opened from disk, where the sea's masks fail the CORS check.
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Quiet, directory=str(ROOT)))
    threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        with sync_playwright() as p:
            browser = p.chromium.launch(args=["--force-device-scale-factor=1", "--hide-scrollbars", "--font-render-hinting=none"])
            page = browser.new_page(viewport={"width": W, "height": H}, device_scale_factor=1)
            page.goto(f"http://127.0.0.1:{server.server_port}/{PAGE}", wait_until="load")
            page.evaluate("document.fonts.ready")
            page.screenshot(path=str(args.out))
            browser.close()
    finally:
        server.shutdown()
    print(args.out)


if __name__ == "__main__":
    main()
