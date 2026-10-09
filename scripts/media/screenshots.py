"""Capture the README screenshots into docs/media from the frontend's mock backend.

usage: python scripts/media/screenshots.py [--out DIR] [--url URL]

Needs `pnpm dev` running; --url defaults to http://localhost:1420.
"""
import argparse
import re
import time
from pathlib import Path

from playwright.sync_api import sync_playwright

ROOT = Path(__file__).resolve().parents[2]
SCALE = 2
PROMPT = "Users are getting 502s from nginx on this box. Find out why and fix it."
PREFS = 'localStorage.setItem("hatoba.mock.prefs", JSON.stringify({ ai_panel_width: 440 }));'


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", type=Path, default=ROOT / "docs" / "media")
    ap.add_argument("--url", default="http://localhost:1420")
    args = ap.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)

    with sync_playwright() as p:
        # A real device scale factor: under Playwright's DPR emulation xterm's WebGL canvas stays at 1x.
        browser = p.chromium.launch(args=[f"--force-device-scale-factor={SCALE}", "--window-size=1440,900"])

        def page(query=""):
            ctx = browser.new_context(no_viewport=True, locale="en-US", color_scheme="dark")
            ctx.add_init_script(PREFS)
            pg = ctx.new_page()
            pg.goto(args.url + "/" + query)
            pg.wait_for_timeout(1200)
            return pg

        def shot(pg, name):
            pg.mouse.move(-10, -10)
            pg.wait_for_timeout(400)
            pg.screenshot(path=args.out / f"{name}.png")
            print("saved", args.out / f"{name}.png")

        # AI: the approval card for the fix, then the finished answer
        pg = page("?ai=showcase")
        pg.get_by_text("prod-api-tokyo", exact=True).first.dblclick()
        pg.wait_for_timeout(1500)
        pg.get_by_label("Show AI Panel (Ctrl+Shift+A)").click()
        box = pg.locator("textarea").last
        box.fill(PROMPT)
        box.press("Enter")
        runs = 0
        deadline = time.time() + 60
        while time.time() < deadline and not pg.get_by_text("Fixed.", exact=True).count():
            run = pg.get_by_role("button", name=re.compile(r"^Run$"))
            if run.count() and run.first.is_visible():
                pg.wait_for_timeout(500)
                if runs == 2:  # the sed fix
                    shot(pg, "ai-approval")
                run.first.click()
                runs += 1
            pg.wait_for_timeout(200)
        pg.wait_for_timeout(5000)  # the answer finishes streaming
        shot(pg, "ai-assistant")

        # Sync: the status page with its devices
        pg = page()
        pg.get_by_text("Cloud Sync", exact=True).first.click()
        pg.wait_for_timeout(1000)
        shot(pg, "sync")

        # Sync: the in-app deployment, finished
        pg = page("?sync=none")
        pg.get_by_text("Cloud Sync", exact=True).first.click()
        pg.wait_for_timeout(600)
        pg.get_by_role("button", name="Continue").click()
        pg.locator("input[type=password]").first.fill("demo-cloudflare-api-token-0000")
        pg.get_by_role("button", name="Continue").click()
        pg.get_by_role("button", name="Deploy", exact=True).click()
        pg.get_by_text("Sync Worker Deployed").wait_for(timeout=30000)
        pg.wait_for_timeout(600)
        shot(pg, "sync-deploy")

        browser.close()


if __name__ == "__main__":
    main()
