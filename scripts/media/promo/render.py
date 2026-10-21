"""Render the promo film from index.html, frame by frame.

usage:
  python scripts/media/promo/capture.py              # once, with `pnpm dev` running
  python scripts/media/promo/render.py [--lang en|zh] [--bpm 120] [--music track.mp3 --music-offset 0.37] [--small] [--out DIR]
  python scripts/media/promo/render.py --stills 0,9.5,37.4 [--lang zh]

Needs Playwright's Chromium, ffmpeg with libx264 on PATH, and `pnpm install` (the page loads the
fonts and icons from node_modules). Writes docs/media/hatoba-promo-<lang>.mp4: the frames, the sound
effects that sfx.py synthesizes from the page's cues, and the music if --music is given. --small
writes hatoba-promo-<lang>-small.mp4 instead, under the 10 MB that GitHub takes for a video in a
comment on a free plan. --bpm stretches the film to the music's tempo. --stills writes single frames
as PNGs instead, for checking a scene. The small film, the stills, and the sound effects and cues go
to scripts/media/promo/out, and everything to --out when it's given.
"""
import argparse
import base64
import functools
import http.server
import json
import mimetypes
import os
import subprocess
import sys
import tempfile
import threading
from concurrent.futures import ProcessPoolExecutor
from pathlib import Path

from playwright.sync_api import sync_playwright

ROOT = Path(__file__).resolve().parents[3]
WORK = Path(__file__).resolve().parent / "out"
PAGE = "scripts/media/promo/index.html"
W, H = 1920, 1080

for ext, kind in ((".js", "text/javascript"), (".css", "text/css"), (".svg", "image/svg+xml"), (".woff2", "font/woff2")):
    mimetypes.add_type(kind, ext)


class Quiet(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *args):
        pass


def serve():
    handler = functools.partial(Quiet, directory=str(ROOT))
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


def open_page(p, url):
    browser = p.chromium.launch(args=["--force-device-scale-factor=1", "--hide-scrollbars", "--font-render-hinting=none"])
    page = browser.new_page(viewport={"width": W, "height": H}, device_scale_factor=1)
    page.on("pageerror", lambda e: print("page error:", e, flush=True))
    page.on("console", lambda m: m.type == "error" and print("console:", m.text, flush=True))
    page.goto(url)
    page.wait_for_function("window.PROMO && (window.PROMO.ready || window.PROMO.error)", timeout=60000)
    error = page.evaluate("window.PROMO.error")
    if error:
        raise SystemExit(f"The film didn't load: {error}")
    cdp = page.context.new_cdp_session(page)
    return browser, page, cdp


def grab(page, cdp, t):
    page.evaluate("t => PROMO.seek(t)", t)
    shot = cdp.send("Page.captureScreenshot", {"format": "png", "optimizeForSpeed": True})
    return base64.b64decode(shot["data"])


def render_segment(url, fps, crf, first, last, path):
    """Renders frames [first, last) into one H.264 file."""
    enc = subprocess.Popen(
        ["ffmpeg", "-loglevel", "error", "-y", "-f", "image2pipe", "-framerate", str(fps), "-i", "-",
         "-c:v", "libx264", "-preset", "veryslow", "-tune", "animation", "-crf", str(crf), "-x264-params", "aq-mode=3",
         "-pix_fmt", "yuv420p", str(path)],
        stdin=subprocess.PIPE,
    )
    with sync_playwright() as p:
        browser, page, cdp = open_page(p, url)
        for i in range(first, last):
            enc.stdin.write(grab(page, cdp, i / fps))
            if (i - first) % 120 == 0:
                print(f"  frames {i}/{last}", flush=True)
        browser.close()
    enc.stdin.close()
    if enc.wait():
        raise SystemExit(f"ffmpeg failed on {path}")
    return path


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--lang", choices=["en", "zh"], default="en")
    ap.add_argument("--bpm", type=float, default=120)
    ap.add_argument("--fps", type=int, default=60)
    ap.add_argument("--music", type=Path)
    ap.add_argument("--music-gain", type=float, default=0.0, help="dB")
    ap.add_argument("--music-offset", type=float, default=0.0, help="seconds to delay the music, or to skip into it when negative")
    ap.add_argument("--small", action="store_true", help="a film under 10 MB, for a video in a GitHub comment")
    ap.add_argument("--out", type=Path, help="where to write instead of docs/media (the film) or scripts/media/promo/out")
    ap.add_argument("--stills", help="comma-separated times in seconds")
    ap.add_argument("--audio-only", action="store_true", help="write only the sound effects (and music) as WAV")
    ap.add_argument("--workers", type=int, default=max(1, min(6, (os.cpu_count() or 2) // 2)))
    args = ap.parse_args()
    work = args.out or WORK
    work.mkdir(parents=True, exist_ok=True)

    server = serve()
    url = f"http://127.0.0.1:{server.server_port}/{PAGE}?render&lang={args.lang}&bpm={args.bpm}"

    if args.stills:
        with sync_playwright() as p:
            browser, page, cdp = open_page(p, url)
            for s in args.stills.split(","):
                t = float(s)
                out = work / f"still-{args.lang}-{t:06.2f}.png"
                out.write_bytes(grab(page, cdp, t))
                print(out)
            browser.close()
        return

    with sync_playwright() as p:
        browser, page, _ = open_page(p, url)
        info = page.evaluate("({ duration: PROMO.duration, cues: PROMO.cues })")
        browser.close()
    sys.path.insert(0, str(Path(__file__).parent))
    import sfx

    (work / f"cues-{args.lang}.json").write_text(json.dumps(info["cues"], indent=1))
    stem = work / f"sfx-{args.lang}.wav"
    if args.audio_only:
        mixed = work / f"audio-{args.lang}.wav"
        sfx.mix(info["cues"], info["duration"], mixed, music=args.music, music_gain=args.music_gain, music_offset=args.music_offset, stem=stem)
        print(stem)
        print(mixed)
        return

    frames = round(info["duration"] * args.fps)
    print(f"{frames} frames at {args.fps} fps, {len(info['cues'])} sound cues, {args.workers} workers")
    crf, kbps = (23, 128) if args.small else (14, 256)

    with tempfile.TemporaryDirectory(dir=work) as tmp:
        tmp = Path(tmp)
        bounds = [round(frames * k / args.workers) for k in range(args.workers + 1)]
        with ProcessPoolExecutor(args.workers) as pool:
            jobs = [pool.submit(render_segment, url, args.fps, crf, a, b, tmp / f"seg{k}.mp4") for k, (a, b) in enumerate(zip(bounds, bounds[1:]))]
            segments = [j.result() for j in jobs]
        (tmp / "list.txt").write_text("".join(f"file '{s.name}'\n" for s in segments))
        video = tmp / "video.mp4"
        subprocess.run(["ffmpeg", "-loglevel", "error", "-y", "-f", "concat", "-safe", "0", "-i", str(tmp / "list.txt"), "-c", "copy", str(video)], check=True)

        audio = tmp / "audio.wav"
        sfx.mix(info["cues"], info["duration"], audio, music=args.music, music_gain=args.music_gain, music_offset=args.music_offset, stem=stem)
        if args.small:
            out = work / f"hatoba-promo-{args.lang}-small.mp4"
        else:
            out = (args.out or ROOT / "docs" / "media") / f"hatoba-promo-{args.lang}.mp4"
        subprocess.run(
            ["ffmpeg", "-loglevel", "error", "-y", "-i", str(video), "-i", str(audio), "-c:v", "copy",
             "-af", "loudnorm=I=-14:TP=-1.5:LRA=11", "-ar", "48000", "-c:a", "aac", "-b:a", f"{kbps}k",
             "-movflags", "+faststart", "-shortest", str(out)],
            check=True,
        )
    print(out)


if __name__ == "__main__":
    main()
