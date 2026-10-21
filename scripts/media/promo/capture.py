"""Capture the app's real UI for the promo: each state as a self-contained HTML snapshot.

usage: python scripts/media/promo/capture.py [--url URL]

Needs `pnpm dev` running (--url defaults to http://localhost:1420). Drives the frontend's mock backend
like screenshots.py does and saves the DOM of every state the film shows, with the app's styles
inlined, to scripts/media/promo/out/snapshots/<lang>/<state>.html. The film loads them into iframes
and animates the real elements, so the product shots match the app exactly.
"""
import argparse
import re
import time
from pathlib import Path

from playwright.sync_api import sync_playwright

ROOT = Path(__file__).resolve().parents[3]
OUT = Path(__file__).resolve().parent / "out" / "snapshots"
PREFS = 'localStorage.setItem("hatoba.mock.prefs", JSON.stringify({ ai_panel_width: 440 }));'

LOCALES = {
    "en": {
        "browser": "en-US",
        "prompt": "nginx keeps returning 502. Find out why and fix it.",
        "show_ai": "Show AI Panel",
        "run": "Run",
        "fixed": "Fixed.",
        "dashboard": "Dashboard",
        "all": "All Hosts",
    },
    "zh": {
        "browser": "zh-CN",
        "prompt": "nginx 一直返回 502，找出原因并修好它。",
        "show_ai": "显示 AI 面板",
        "run": "运行",
        "fixed": "修好了。",
        "dashboard": "仪表盘",
        "all": "全部主机",
    },
}

# Serializes the live document: scripts dropped, form values and scroll offsets kept as attributes,
# images inlined, and style sheets that came from <link> tags inlined.
SERIALIZE = r"""
async () => {
  const dataUrl = async (url) => {
    const blob = await (await fetch(url)).blob();
    return await new Promise((done) => { const r = new FileReader(); r.onload = () => done(r.result); r.readAsDataURL(blob); });
  };
  for (const e of document.querySelectorAll('input, textarea')) e.setAttribute('data-value', e.value);
  for (const e of document.querySelectorAll('*')) {
    if (e.scrollTop || e.scrollLeft) e.setAttribute('data-scroll', `${e.scrollLeft},${e.scrollTop}`);
  }
  const doc = document.documentElement.cloneNode(true);
  doc.querySelectorAll('script').forEach((s) => s.remove());
  for (const img of doc.querySelectorAll('img[src]')) {
    const src = img.getAttribute('src');
    if (!src.startsWith('data:')) img.setAttribute('src', await dataUrl(new URL(src, location.href)));
  }
  for (const link of doc.querySelectorAll('link[rel=stylesheet]')) {
    const style = document.createElement('style');
    style.textContent = await (await fetch(new URL(link.getAttribute('href'), location.href))).text();
    link.replaceWith(style);
  }
  return '<!doctype html>' + doc.outerHTML;
}
"""


def fix_urls(html):
    """Points the app's font URLs at the same files under the repo, which render.py serves."""

    def repl(m):
        url = m.group(2)
        k = url.find("node_modules/")
        if "Phosphor" in url and k >= 0:
            tail = url[k:].split("?")[0]
            tail = re.sub(r"node_modules/\.pnpm/[^/]+/node_modules/", "node_modules/", tail)
            return f"url({m.group(1)}/apps/desktop/{tail}{m.group(1)})"
        return m.group(0)

    return re.sub(r"""url\((['"]?)([^'")]+)\1\)""", repl, html)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--url", default="http://localhost:1420")
    args = ap.parse_args()

    with sync_playwright() as p:
        browser = p.chromium.launch(args=["--force-device-scale-factor=1"])
        for lang, label in LOCALES.items():
            out = OUT / lang
            out.mkdir(parents=True, exist_ok=True)

            def page(query="", width=1240):
                ctx = browser.new_context(viewport={"width": width, "height": 660}, locale=label["browser"], color_scheme="dark")
                ctx.add_init_script(PREFS)
                pg = ctx.new_page()
                pg.goto(args.url + "/" + query)
                pg.wait_for_timeout(1500)
                return pg

            def save(pg, name):
                pg.mouse.move(-10, -10)
                pg.wait_for_timeout(400)
                (out / f"{name}.html").write_text(fix_urls(pg.evaluate(SERIALIZE)), encoding="utf-8")
                print(lang, name)

            pg = page()
            pg.get_by_text(label["all"], exact=True).first.click()
            pg.wait_for_timeout(800)
            save(pg, "hosts")
            pg.get_by_text("prod-api-tokyo", exact=True).first.dblclick()
            pg.wait_for_timeout(2500)
            save(pg, "term")
            pg.get_by_text("SFTP", exact=True).first.click()
            pg.wait_for_timeout(1500)
            save(pg, "sftp")

            pg = page("?ai=showcase", 1400)
            pg.get_by_text("prod-api-tokyo", exact=True).first.dblclick()
            pg.wait_for_timeout(1500)
            pg.get_by_label(re.compile("^" + re.escape(label["show_ai"]))).click()
            pg.wait_for_timeout(600)
            for old in out.glob("ai-*.html"):
                old.unlink()
            save(pg, "ai-empty")
            box = pg.locator("textarea").last
            box.fill(label["prompt"])
            box.press("Enter")
            runs = 0
            deadline = time.time() + 60
            while time.time() < deadline and not pg.get_by_text(label["fixed"], exact=True).count():
                run = pg.get_by_role("button", name=re.compile("^" + re.escape(label["run"]) + "$"))
                if run.count() and run.first.is_visible():
                    pg.wait_for_timeout(600)
                    save(pg, f"ai-{runs}")
                    run.first.click()
                    runs += 1
                pg.wait_for_timeout(200)
            # promo.js plays exactly these stages: four approvals (RUNS), then the summary.
            if not pg.get_by_text(label["fixed"], exact=True).count():
                raise SystemExit(f"{lang}: the assistant didn't finish its reply within a minute")
            if runs != 4:
                raise SystemExit(f"{lang}: the conversation asked for {runs} approvals, and promo.js plays four")
            pg.wait_for_timeout(5000)
            save(pg, "ai-done")

            pg = page()
            pg.get_by_text(label["dashboard"], exact=True).first.click()
            pg.wait_for_timeout(4000)
            save(pg, "dash")
        browser.close()
    (OUT / "README.txt").write_text("Generated by capture.py from the app's mock frontend.\n")


if __name__ == "__main__":
    main()
