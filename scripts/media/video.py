"""Record the demo video into docs/media from the frontend's mock backend.

usage: python scripts/media/video.py [--out DIR] [--url URL]

Needs `pnpm dev` running (--url defaults to http://localhost:1420) and ffmpeg with libx264 and
libwebp on PATH. Writes hatoba-demo.mp4 (with the music from music.py) and hatoba-demo.webp
(silent, for the README).
"""
import argparse
import base64
import json
import re
import subprocess
import tempfile
import time
from pathlib import Path

from playwright.sync_api import sync_playwright

import music

ROOT = Path(__file__).resolve().parents[2]
W, H = 1440, 900
SPEED = 1.2
PROMPT = "Users are getting 502s from nginx on this box. Find out why and fix it."

# A cursor that follows the mouse (headless screencasts have none), click ripples, captions, and
# full-screen title cards. Runs before the app so the AI panel opens at a readable width.
OVERLAY = r"""
(() => {
  const LOGO = __LOGO__;
  localStorage.setItem("hatoba.mock.prefs", JSON.stringify({ ai_panel_width: 440 }));
  const css = `
  #__cur{position:fixed;left:0;top:0;width:22px;height:22px;z-index:2147483647;pointer-events:none;transform:translate(-100px,-100px)}
  #__cur svg{filter:drop-shadow(0 1px 2px rgba(0,0,0,.6))}
  .__ripple{position:fixed;width:34px;height:34px;margin:-17px 0 0 -17px;border-radius:50%;border:2px solid rgba(80,160,255,.9);z-index:2147483646;pointer-events:none;animation:__rip .5s ease-out forwards}
  @keyframes __rip{from{transform:scale(.3);opacity:1}to{transform:scale(1.4);opacity:0}}
  #__cap{position:fixed;left:50%;bottom:44px;transform:translate(-50%,12px);z-index:2147483645;pointer-events:none;opacity:0;transition:opacity .35s ease,transform .35s ease;
    background:rgba(14,15,18,.92);color:#fff;border:1px solid rgba(255,255,255,.12);border-radius:14px;padding:14px 24px;max-width:900px;text-align:center;
    font:600 22px/1.35 "Segoe UI Variable Display","Segoe UI",system-ui,sans-serif;box-shadow:0 12px 40px rgba(0,0,0,.45)}
  #__cap small{display:block;font-weight:400;font-size:15px;color:rgba(255,255,255,.7);margin-top:4px}
  #__cap.on{opacity:1;transform:translate(-50%,0)}
  #__card{position:fixed;inset:0;z-index:2147483646;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:18px;
    background:radial-gradient(1200px 700px at 50% 40%,#1f2a3d 0%,#111215 70%);color:#fff;opacity:0;transition:opacity .6s ease;pointer-events:none;
    font-family:"Segoe UI Variable Display","Segoe UI",system-ui,sans-serif}
  #__card.on{opacity:1}
  #__card svg{width:120px;height:120px}
  #__card h1{margin:0;font-size:56px;font-weight:700;letter-spacing:-.5px}
  #__card p{margin:0;font-size:22px;color:rgba(255,255,255,.75);text-align:center;max-width:820px;line-height:1.45}
  #__card code{font:500 18px "Cascadia Mono",Consolas,monospace;color:#7ab7ff;margin-top:6px}
  `;
  const boot = () => {
    const style = document.createElement("style");
    style.textContent = css;
    document.head.appendChild(style);
    const cur = document.createElement("div");
    cur.id = "__cur";
    cur.innerHTML = '<svg width="22" height="22" viewBox="0 0 24 24"><path d="M4 2l15 11.5-6.6.9 3.9 7.4-3 1.5-3.9-7.5L4 20z" fill="#fff" stroke="#000" stroke-width="1.3" stroke-linejoin="round"/></svg>';
    const cap = document.createElement("div");
    cap.id = "__cap";
    const card = document.createElement("div");
    card.id = "__card";
    document.body.append(cur, cap, card);
    addEventListener("mousemove", (e) => { cur.style.transform = `translate(${e.clientX - 3}px,${e.clientY - 2}px)`; }, true);
    addEventListener("mousedown", (e) => {
      const r = document.createElement("div");
      r.className = "__ripple";
      r.style.left = e.clientX + "px";
      r.style.top = e.clientY + "px";
      document.body.appendChild(r);
      setTimeout(() => r.remove(), 600);
    }, true);
    window.__caption = (text, sub) => {
      if (!text) return cap.classList.remove("on");
      cap.innerHTML = text + (sub ? `<small>${sub}</small>` : "");
      cap.classList.add("on");
    };
    window.__card = (html) => {
      cur.style.visibility = html ? "hidden" : "visible";
      if (!html) return card.classList.remove("on");
      card.innerHTML = LOGO + html;
      card.classList.add("on");
    };
  };
  if (document.readyState === "loading") addEventListener("DOMContentLoaded", boot);
  else boot();
})();
"""


def record(url):
    """Plays the demo and returns its screencast frames as (timestamp, jpeg bytes)."""
    logo = (ROOT / "apps" / "desktop" / "src-tauri" / "app-icon.svg").read_text(encoding="utf-8")
    frames = []
    with sync_playwright() as p:
        browser = p.chromium.launch()
        ctx = browser.new_context(viewport={"width": W, "height": H}, locale="en-US", color_scheme="dark")
        ctx.add_init_script(OVERLAY.replace("__LOGO__", json.dumps(logo)))
        pg = ctx.new_page()
        pg.goto(url + "/?ai=showcase&sync=none")
        pg.wait_for_timeout(1500)
        pg.evaluate("window.__card(`<h1>Hatoba</h1><p>An open-source SSH client with an AI assistant<br>and end-to-end encrypted sync</p>`)")
        pg.wait_for_timeout(700)

        # CDP screencast frames are much sharper than Playwright's own video recording.
        cdp = ctx.new_cdp_session(pg)

        def on_frame(ev):
            frames.append((ev["metadata"]["timestamp"], base64.b64decode(ev["data"])))
            cdp.send("Page.screencastFrameAck", {"sessionId": ev["sessionId"]})

        cdp.on("Page.screencastFrame", on_frame)
        cdp.send("Page.startScreencast", {"format": "jpeg", "quality": 92, "maxWidth": W, "maxHeight": H})

        mouse = {"x": W / 2, "y": H / 2}
        wait = pg.wait_for_timeout

        def move(x, y, ms=550):
            pg.mouse.move(x, y, steps=max(8, int(ms / 16)))
            mouse["x"], mouse["y"] = x, y

        def point(loc):
            bb = loc.bounding_box()
            return bb["x"] + bb["width"] / 2, bb["y"] + bb["height"] / 2

        def click(loc, ms=550, double=False):
            loc.wait_for(state="visible")
            loc.scroll_into_view_if_needed()
            move(*point(loc), ms)
            wait(120)
            # Dialogs re-centre between steps, so the target may have moved meanwhile.
            x, y = point(loc)
            if abs(x - mouse["x"]) > 3 or abs(y - mouse["y"]) > 3:
                move(x, y, 200)
                wait(80)
            (pg.mouse.dblclick if double else pg.mouse.click)(mouse["x"], mouse["y"])

        def caption(text=None, sub=None):
            pg.evaluate("([t, s]) => window.__caption(t, s)", [text, sub])

        move(W / 2, H / 2 + 120, 10)
        wait(2600)
        pg.evaluate("window.__card(null)")
        wait(700)

        # AI
        caption("An AI assistant beside every terminal")
        click(pg.get_by_text("prod-api-tokyo", exact=True).first, ms=800, double=True)
        wait(1800)
        click(pg.get_by_label("Show AI Panel (Ctrl+Shift+A)"))
        wait(700)
        click(pg.locator("textarea").last)
        caption("Ask it about the server you are on")
        pg.keyboard.type(PROMPT, delay=28)
        wait(400)
        pg.keyboard.press("Enter")
        approvals = 0
        deadline = time.time() + 90
        while time.time() < deadline and not pg.get_by_text("Fixed.", exact=True).count():
            run = pg.get_by_role("button", name=re.compile(r"^Run$"))
            if run.count() and run.first.is_visible():
                if approvals == 0:
                    caption("It runs commands only after you approve them")
                if approvals == 3:  # send_input, after two checks and the fix
                    caption("…and types into your shell to check its own fix")
                wait(900)
                click(run.first, ms=450)
                approvals += 1
                move(W - 300, H - 160, 400)
            wait(150)
        caption("Found, fixed, and verified")
        wait(3600)
        caption()
        click(pg.get_by_label("Close AI Panel"))
        wait(500)

        # Sync
        caption("Sync through your own Cloudflare account")
        click(pg.get_by_text("Cloud Sync", exact=True).first, ms=700)
        wait(1600)
        click(pg.get_by_role("button", name="Continue"))
        wait(900)
        caption("Hatoba deploys the Worker and D1 database for you")
        click(pg.locator("input[type=password]").first)
        pg.keyboard.type("demo-cloudflare-api-token-0000", delay=18)
        wait(400)
        click(pg.get_by_role("button", name="Continue"))
        wait(900)
        click(pg.get_by_role("button", name="Deploy", exact=True))
        pg.get_by_text("Sync Worker Deployed").wait_for(timeout=30000)
        wait(1200)
        click(pg.get_by_role("button", name="Continue"))
        wait(700)
        caption("End-to-end encrypted with your master password", "Your Worker and database only ever store ciphertext")
        click(pg.locator("input[type=password]").first)
        pg.keyboard.type("correct-horse-battery", delay=35)
        wait(500)
        click(pg.get_by_role("button", name="Finish Setup"))
        wait(2500)
        caption("Every device stays in sync. No Hatoba server in between.")
        move(W / 2 + 200, H / 2 + 200, 900)
        wait(3800)
        caption()
        wait(400)
        pg.evaluate("window.__card(`<h1>Hatoba</h1><p>Open source · MIT</p><code>github.com/scarletkc/Hatoba</code>`)")
        wait(1000)
        cdp.send("Page.stopScreencast")
        # The screencast sends no frame once the page stops changing, so the settled card is a screenshot.
        frames.append((max(f[0] for f in frames) + 0.05, pg.screenshot(type="jpeg", quality=92)))
        browser.close()
    return sorted(frames, key=lambda f: f[0])


def ffmpeg(*args):
    subprocess.run(["ffmpeg", "-loglevel", "error", "-y", *map(str, args)], check=True)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", type=Path, default=ROOT / "docs" / "media")
    ap.add_argument("--url", default="http://localhost:1420")
    args = ap.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)

    frames = record(args.url)
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        # Frames arrive only when the page changes; the concat list keeps each one on screen until the next.
        lines = []
        for i, (ts, data) in enumerate(frames):
            (tmp / f"f{i:05d}.jpg").write_bytes(data)
            dur = frames[i + 1][0] - ts if i + 1 < len(frames) else 3.0
            lines.append(f"file 'f{i:05d}.jpg'\nduration {max(dur, 0.001):.4f}")
        lines.append(f"file 'f{len(frames) - 1:05d}.jpg'")
        (tmp / "list.txt").write_text("\n".join(lines) + "\n")

        silent = tmp / "silent.mp4"
        ffmpeg("-f", "concat", "-safe", "0", "-i", tmp / "list.txt", "-vf", f"setpts=PTS/{SPEED},fps=30,format=yuv420p",
               "-c:v", "libx264", "-crf", "20", "-preset", "slow", "-an", silent)
        duration = float(subprocess.run(["ffprobe", "-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0", str(silent)],
                                        check=True, capture_output=True, text=True).stdout)
        music.render(duration, tmp / "music.wav")
        ffmpeg("-i", silent, "-i", tmp / "music.wav", "-c:v", "copy", "-c:a", "aac", "-b:a", "160k", "-shortest",
               "-movflags", "+faststart", args.out / "hatoba-demo.mp4")
        ffmpeg("-i", silent, "-vf", "fps=12,scale=1200:-1:flags=lanczos", "-c:v", "libwebp_anim", "-lossless", "0", "-q:v", "75",
               "-compression_level", "6", "-loop", "0", args.out / "hatoba-demo.webp")
    print(f"saved {args.out / 'hatoba-demo.mp4'} and hatoba-demo.webp ({duration:.1f} s)")


if __name__ == "__main__":
    main()
