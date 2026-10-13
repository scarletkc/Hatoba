"""Capture the website's screenshots from the frontend's mock backend.

usage: python scripts/media/screenshots.py [--locale en|zh-CN|ja] [--out DIR] [--url URL]

Needs `pnpm dev` running; --url defaults to http://localhost:1420. The English screenshots go to
docs/media and the Chinese and Japanese ones to apps/site/src/assets/screenshots/<locale>, where
the website's pages in each language use them.
"""
import argparse
import re
import time
from pathlib import Path

from playwright.sync_api import sync_playwright

ROOT = Path(__file__).resolve().parents[2]
SCALE = 2
PREFS = 'localStorage.setItem("hatoba.mock.prefs", JSON.stringify({ ai_panel_width: 440 }));'

# The browser language that makes the app pick each locale, and the labels the script clicks and
# waits for, as apps/desktop/src/i18n/locales has them. `fixed` starts the last reply of the
# `?ai=showcase` conversation in apps/desktop/src/ipc/mock/ai.ts.
LOCALES = {
    "en": {
        "browser": "en-US",
        "prompt": "Users are getting 502s from nginx on this box. Find out why and fix it.",
        "show_ai": "Show AI Panel",
        "run": "Run",
        "fixed": "Fixed.",
        "dashboard": "Dashboard",
        "sync": "Cloud Sync",
        "continue": "Continue",
        "deploy": "Deploy",
        "deployed": "Sync Worker Deployed",
    },
    "zh-CN": {
        "browser": "zh-CN",
        "prompt": "这台机器上的 nginx 一直返回 502，找出原因并修好它。",
        "show_ai": "显示 AI 面板",
        "run": "运行",
        "fixed": "修好了。",
        "dashboard": "仪表盘",
        "sync": "云同步",
        "continue": "继续",
        "deploy": "部署",
        "deployed": "同步 Worker 已部署",
    },
    "ja": {
        "browser": "ja-JP",
        "prompt": "nginx が 502 を返しています。原因を調べて直してください。",
        "show_ai": "AI パネルを表示",
        "run": "実行",
        "fixed": "修正しました。",
        "dashboard": "ダッシュボード",
        "sync": "クラウド同期",
        "continue": "続ける",
        "deploy": "デプロイ",
        "deployed": "同期 Worker をデプロイしました",
    },
}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--locale", choices=LOCALES, default="en")
    ap.add_argument("--out", type=Path)
    ap.add_argument("--url", default="http://localhost:1420")
    args = ap.parse_args()
    label = LOCALES[args.locale]
    out = args.out or (ROOT / "docs" / "media" if args.locale == "en" else ROOT / "apps" / "site" / "src" / "assets" / "screenshots" / args.locale)
    out.mkdir(parents=True, exist_ok=True)

    with sync_playwright() as p:
        # A real device scale factor: under Playwright's DPR emulation xterm's WebGL canvas stays at 1x.
        browser = p.chromium.launch(args=[f"--force-device-scale-factor={SCALE}", "--window-size=1440,900"])

        def page(query=""):
            ctx = browser.new_context(no_viewport=True, locale=label["browser"], color_scheme="dark")
            ctx.add_init_script(PREFS)
            pg = ctx.new_page()
            pg.goto(args.url + "/" + query)
            pg.wait_for_timeout(1200)
            return pg

        def shot(pg, name):
            pg.mouse.move(-10, -10)
            pg.wait_for_timeout(400)
            pg.screenshot(path=out / f"{name}.png")
            print("saved", out / f"{name}.png")

        # AI: the approval card for the fix, then the finished answer
        pg = page("?ai=showcase")
        pg.get_by_text("prod-api-tokyo", exact=True).first.dblclick()
        pg.wait_for_timeout(1500)
        pg.get_by_label(re.compile("^" + re.escape(label["show_ai"]))).click()
        box = pg.locator("textarea").last
        box.fill(label["prompt"])
        box.press("Enter")
        runs = 0
        deadline = time.time() + 60
        while time.time() < deadline and not pg.get_by_text(label["fixed"], exact=True).count():
            run = pg.get_by_role("button", name=re.compile("^" + re.escape(label["run"]) + "$"))
            if run.count() and run.first.is_visible():
                pg.wait_for_timeout(500)
                if runs == 2:  # the sed fix
                    shot(pg, "ai-approval")
                run.first.click()
                runs += 1
            pg.wait_for_timeout(200)
        pg.wait_for_timeout(5000)  # the answer finishes streaming
        shot(pg, "ai-assistant")

        # Dashboard: every host's status and resource usage, once the first readings are in
        pg = page()
        pg.get_by_text(label["dashboard"], exact=True).first.click()
        pg.wait_for_timeout(4000)
        shot(pg, "dashboard")

        # Sync: the in-app deployment, finished
        pg = page("?sync=none")
        pg.get_by_text(label["sync"], exact=True).first.click()
        pg.wait_for_timeout(600)
        pg.get_by_role("button", name=label["continue"]).click()
        pg.locator("input[type=password]").first.fill("demo-cloudflare-api-token-0000")
        pg.get_by_role("button", name=label["continue"]).click()
        pg.get_by_role("button", name=label["deploy"], exact=True).click()
        pg.get_by_text(label["deployed"]).wait_for(timeout=30000)
        pg.wait_for_timeout(600)
        shot(pg, "sync-deploy")

        browser.close()


if __name__ == "__main__":
    main()
