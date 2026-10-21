"""Synthesize the promo's sound effects from the page's cues: original, so they carry no license.

render.py calls mix() with the cues index.html exports (PROMO.cues). Every sound is built here from
sines and filtered noise. The pitched ones sit on D major so they agree with each other.

usage (standalone, from a cues file render.py wrote):
  python scripts/media/promo/sfx.py out/cues-en.json out/audio-en.wav --duration 70 [--music track.mp3 --music-offset 0.37]
"""
import argparse
import json
import subprocess
import sys
from pathlib import Path

import numpy as np
from scipy.io import wavfile
from scipy.signal import butter, fftconvolve, sosfilt

SR = 48000
D5 = 587.33


def semis(f, p):
    return f * 2 ** (p / 12)


def env(n, a, d):
    """Linear attack over a seconds, then exponential decay with time constant d."""
    t = np.arange(n) / SR
    e = np.exp(-np.maximum(t - a, 0) / d)
    k = max(1, int(a * SR))
    e[:k] *= np.linspace(0, 1, k)[: len(e[:k])]
    return e


def osc(freq):
    """A sine whose frequency follows the array freq."""
    return np.sin(2 * np.pi * np.cumsum(freq) / SR)


def glide(n, f0, f1, curve=4.0):
    t = np.linspace(0, 1, n)
    return f1 + (f0 - f1) * np.exp(-curve * t)


def bandpass(x, lo, hi):
    return sosfilt(butter(2, [lo, hi], "band", fs=SR, output="sos"), x)


def lowpass(x, fc):
    return sosfilt(butter(2, fc, "low", fs=SR, output="sos"), x)


def highpass(x, fc):
    return sosfilt(butter(2, fc, "high", fs=SR, output="sos"), x)


def swept(x, f0, f1, width=0.9, bands=14):
    """Band-passes x around a center that glides from f0 to f1, by crossfading a bank of bands."""
    t = np.linspace(0, 1, len(x))
    target = np.log(f0) + (np.log(f1) - np.log(f0)) * t
    out = np.zeros(len(x))
    total = np.zeros(len(x))
    for c in np.geomspace(min(f0, f1) / 1.3, max(f0, f1) * 1.3, bands):
        w = np.exp(-((np.log(c) - target) ** 2) / (2 * 0.35**2))
        out += bandpass(x, c / (1 + width), min(c * (1 + width), SR / 2 - 100)) * w
        total += w
    return out / np.maximum(total, 1e-6)


rng = np.random.default_rng(11)


def noise(n):
    return rng.standard_normal(n)


# ---------- the sounds ----------


def pop(p=0, **_):
    n = int(0.16 * SR)
    f = glide(n, semis(D5, p) * 1.9, semis(D5, p), 60)
    x = osc(f) * env(n, 0.002, 0.045)
    x += highpass(noise(n), 3000) * env(n, 0.0005, 0.004) * 0.15
    return x * 0.8


def bubble(p=0, **_):
    out = np.zeros(int(0.3 * SR))
    for k, (dt, g) in enumerate(((0, 1.0), (0.055, 0.55), (0.11, 0.3))):
        n = int(0.09 * SR)
        f0 = semis(320, p + k * 5)
        f = np.linspace(f0, f0 * 2.6, n) ** 1.0
        x = osc(f) * env(n, 0.003, 0.03) * g
        i = int(dt * SR)
        out[i : i + n] += x
    return out * 0.8


def splash(**_):
    n = int(1.1 * SR)
    x = swept(noise(n), 5000, 600, 1.2) * env(n, 0.006, 0.22)
    x += lowpass(noise(n), 900) * env(n, 0.01, 0.35) * 0.6
    for k in range(7):
        b = bubble(p=rng.uniform(4, 14)) * rng.uniform(0.15, 0.35)
        i = int(rng.uniform(0.05, 0.6) * SR)
        x[i : i + len(b)] += b[: n - i]
    return x * 0.9


def thud(**_):
    n = int(0.5 * SR)
    x = osc(glide(n, 140, 52, 9)) * env(n, 0.003, 0.12)
    x += lowpass(noise(n), 300) * env(n, 0.001, 0.05) * 0.5
    return x * 1.1


def clack(p=0, **_):
    n = int(0.25 * SR)
    f = semis(880, p)
    x = (np.sin(2 * np.pi * f * np.arange(n) / SR) + 0.6 * np.sin(2 * np.pi * f * 2.76 * np.arange(n) / SR)) * env(n, 0.001, 0.035)
    x += bandpass(noise(n), 1500, 5000) * env(n, 0.0005, 0.006) * 0.5
    return x * 0.7


def whoosh(dur=1.0, **_):
    n = int(dur * SR)
    t = np.linspace(0, 1, n)
    shape = np.sin(np.pi * np.clip(t / 0.62, 0, 1) * 0.5) ** 2 * np.where(t < 0.62, 1, np.exp(-(t - 0.62) * 9))
    x = swept(noise(n), 250, 2600, 1.1) * shape
    return np.stack([x * np.sqrt(1 - t * 0.7), x * np.sqrt(0.3 + t * 0.7)]) * 0.9


def swish(**_):
    n = int(0.36 * SR)
    t = np.linspace(0, 1, n)
    x = swept(noise(n), 900, 4200, 0.8) * np.sin(np.pi * t) ** 2
    return np.stack([x * (1 - t * 0.5), x * (0.5 + t * 0.5)]) * 0.75


def click(**_):
    n = int(0.08 * SR)
    x = highpass(noise(n), 2500) * env(n, 0.0003, 0.003) * 0.6
    x += np.sin(2 * np.pi * 3200 * np.arange(n) / SR) * env(n, 0.0005, 0.008) * 0.35
    i = int(0.045 * SR)
    x[i:] += (highpass(noise(n - i), 3000) * env(n - i, 0.0003, 0.002)) * 0.3
    return x


def key(p=0, **_):
    n = int(0.06 * SR)
    c = semis(3200, p)
    x = bandpass(noise(n), c / 1.6, min(c * 1.6, 20000)) * env(n, 0.0004, 0.006)
    x += np.sin(2 * np.pi * semis(190, p) * np.arange(n) / SR) * env(n, 0.001, 0.012) * 0.5
    return x * 0.8


def bell(f, n, decay=1.2, index=2.2, ratio=3.5):
    t = np.arange(n) / SR
    mod = np.sin(2 * np.pi * f * ratio * t) * index * np.exp(-t / (decay * 0.3))
    return np.sin(2 * np.pi * f * t + mod) * env(n, 0.002, decay)


def chime(p=0, **_):
    n = int(3.2 * SR)
    out = np.zeros((2, n))
    for k, s in enumerate((0, 4, 7, 14, 19)):
        i = int(k * 0.045 * SR)
        x = bell(semis(D5, p + s), n - i, 1.4 - k * 0.12, 1.6) * (0.5 if k else 0.7)
        pan = (k - 2) / 3
        out[0, i:] += x * np.sqrt((1 - pan) / 2)
        out[1, i:] += x * np.sqrt((1 + pan) / 2)
    sp = sparkle(p + 12)
    out[:, : sp.shape[1]] += sp * 0.35
    return out


def sparkle(p=0, **_):
    n = int(0.9 * SR)
    out = np.zeros((2, n))
    for k, s in enumerate((12, 16, 19, 24, 28, 31, 36)):
        i = int(k * 0.035 * SR)
        m = int(0.4 * SR)
        x = np.sin(2 * np.pi * semis(D5, p + s) * np.arange(m) / SR) * env(m, 0.001, 0.07)
        pan = 0.6 if k % 2 else -0.6
        out[0, i : i + m] += x * np.sqrt((1 - pan) / 2) * 0.45
        out[1, i : i + m] += x * np.sqrt((1 + pan) / 2) * 0.45
    return out


def marimba(f, n):
    t = np.arange(n) / SR
    return (np.sin(2 * np.pi * f * t) * env(n, 0.002, 0.28) + 0.35 * np.sin(2 * np.pi * f * 4 * t) * env(n, 0.001, 0.04)) * 0.9


def success(p=0, **_):
    n = int(1.6 * SR)
    out = np.zeros((2, n))
    for k, s in enumerate((7, 11, 14, 19)):
        i = int(k * 0.085 * SR)
        x = marimba(semis(D5, p + s), n - i)
        out[:, i:] += x * 0.55
    sp = sparkle(p + 5)
    i = int(0.3 * SR)
    out[:, i : i + sp.shape[1]] += sp * 0.6
    return out


def ding(p=0, **_):
    n = int(1.0 * SR)
    f = semis(D5 * 2, p)
    t = np.arange(n) / SR
    return (np.sin(2 * np.pi * f * t) * env(n, 0.002, 0.35) + 0.25 * np.sin(2 * np.pi * f * 2.76 * t) * env(n, 0.001, 0.08)) * 0.55


def tick(p=0, **_):
    n = int(0.12 * SR)
    a = marimba(semis(D5 * 2, p + 7), n) * 0.45
    return a * env(n, 0.001, 0.05)


def boop(p=0, **_):
    out = np.zeros(int(0.42 * SR))
    for k, (s0, s1) in enumerate(((-2, -1), (-6, -7))):
        n = int(0.16 * SR)
        f = np.linspace(semis(D5 * 0.5, p + s0), semis(D5 * 0.5, p + s1), n)
        ph = np.cumsum(f) / SR
        x = lowpass(np.sign(np.sin(2 * np.pi * ph)) * 0.5 + np.sin(2 * np.pi * ph), 1800) * env(n, 0.005, 0.07)
        i = int(k * 0.17 * SR)
        out[i : i + n] += x * 0.55
    return out


def send(**_):
    n = int(0.3 * SR)
    t = np.linspace(0, 1, n)
    x = swept(noise(n), 600, 3800, 0.7) * np.sin(np.pi * np.clip(t * 1.25, 0, 1)) ** 2 * 0.6
    b = pop(9)
    i = int(0.2 * SR)
    x[i:] += b[: n - i]
    return x


def lock(p=0, **_):
    n = int(0.3 * SR)
    out = np.zeros(n)
    for k, g in ((0, 0.6), (0.075, 1.0)):
        m = int(0.06 * SR)
        tt = np.arange(m) / SR
        x = (np.sin(2 * np.pi * semis(2400, p) * tt) + 0.7 * np.sin(2 * np.pi * semis(3900, p) * tt)) * env(m, 0.0003, 0.012)
        x += highpass(noise(m), 2500) * env(m, 0.0002, 0.003) * 0.6
        i = int(k * SR)
        out[i : i + m] += x * g * 0.5
    out += osc(glide(n, 220, 110, 10)) * env(n, 0.07, 0.05) * 0.35 * (np.arange(n) > int(0.075 * SR))
    return out


def unlock(p=0, **_):
    n = int(0.35 * SR)
    out = lock(p)[:n] * 0.6
    m = int(0.16 * SR)
    i = int(0.09 * SR)
    out[i : i + m] += osc(np.linspace(semis(700, p), semis(1500, p), m)) * env(m, 0.005, 0.05) * 0.5
    return out


def wipe(**_):
    n = int(1.8 * SR)
    t = np.linspace(0, 1, n)
    shape = np.sin(np.pi * np.clip(t / 0.75, 0, 1)) ** 1.5
    x = swept(noise(n), 300, 1800, 1.2) * shape * 0.8
    x += lowpass(noise(n), 400) * shape * 0.5
    for _ in range(14):
        b = bubble(p=rng.uniform(-2, 16)) * rng.uniform(0.15, 0.4)
        i = int(rng.uniform(0.1, 1.3) * SR)
        x[i : i + len(b)] += b[: n - i]
    return np.stack([x, np.roll(x, 240)])


def waves(dur=10.0, **_):
    n = int(dur * SR)
    t = np.arange(n) / SR
    fade = np.minimum(1, np.minimum(t / 1.5, (dur - t) / 1.5))
    out = []
    for ch in range(2):
        x = lowpass(np.cumsum(noise(n)) * 0.02, 600)
        x = highpass(x, 60)
        x /= np.abs(x).max() + 1e-9
        swell = 0.55 + 0.45 * np.sin(2 * np.pi * t / 3.6 + ch * 1.3)
        out.append(x * swell * fade * 0.6)
    return np.stack(out)


SOUNDS = {f.__name__: f for f in (pop, bubble, splash, thud, clack, whoosh, swish, click, key, chime, sparkle, success, ding, tick, boop, send, lock, unlock, wipe, waves)}


# ---------- mixing ----------


def reverb(x, seconds=1.4, wet=0.16):
    n = int(seconds * SR)
    t = np.arange(n) / SR
    ir = np.stack([lowpass(noise(n), 5000) * np.exp(-t / (seconds / 5)) for _ in range(2)])
    ir /= np.sqrt((ir**2).sum(axis=1, keepdims=True))
    w = np.stack([fftconvolve(x[c], ir[c])[: x.shape[1]] for c in range(2)])
    return x + w * wet


def load_music(path, n, offset=0.0):
    """The track as a (2, n) array: delayed by offset seconds (or started that far in when negative),
    and faded out over the film's last seconds."""
    seek = ["-ss", f"{-offset:.3f}"] if offset < 0 else []
    raw = subprocess.run(["ffmpeg", "-loglevel", "error", *seek, "-i", str(path), "-f", "f32le", "-ac", "2", "-ar", str(SR), "-"], capture_output=True, check=True).stdout
    m = np.frombuffer(raw, dtype=np.float32).reshape(-1, 2).T.astype(np.float64)
    out = np.zeros((2, n))
    i = int(max(0.0, offset) * SR)
    k = min(n - i, m.shape[1])
    out[:, i : i + k] = m[:, :k]
    return out


def mix(cues, duration, path, music=None, music_gain=0.0, music_offset=0.0, sfx_gain=-4.0, stem=None):
    n = int(duration * SR)
    bus = np.zeros((2, n))
    for c in cues:
        f = SOUNDS[c["name"]]
        x = f(p=c.get("pitch", 0), dur=c.get("dur", 1.0))
        if x.ndim == 1:
            pan = c.get("pan", 0.0)
            x = np.stack([x * np.sqrt((1 - pan) / 2), x * np.sqrt((1 + pan) / 2)]) * np.sqrt(2)
        i = int(c["t"] * SR)
        if i >= n:
            continue
        k = min(x.shape[1], n - i)
        bus[:, i : i + k] += x[:, :k] * 10 ** (c.get("gain", 0) / 20)
    bus = reverb(bus)
    bus = np.tanh(bus * 1.2) / 1.2
    bus *= 0.5 / max(1e-9, np.abs(bus).max())
    if stem:
        wavfile.write(stem, SR, (bus.T * 32767).astype(np.int16))
    if music:
        bus = bus * 10 ** (sfx_gain / 20) + load_music(music, n, music_offset) * 10 ** (music_gain / 20)
    # Everything fades out over the last 3.5 seconds.
    fade = min(n, int(3.5 * SR))
    bus[:, n - fade :] *= np.linspace(1, 0, fade) ** 1.5
    bus *= 0.89 / max(1e-9, np.abs(bus).max(), 0.89)
    wavfile.write(path, SR, (bus.T * 32767).astype(np.int16))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("cues", type=Path)
    ap.add_argument("out", type=Path)
    ap.add_argument("--music", type=Path)
    ap.add_argument("--music-gain", type=float, default=0.0)
    ap.add_argument("--music-offset", type=float, default=0.0)
    ap.add_argument("--duration", type=float, required=True)
    args = ap.parse_args()
    cues = json.loads(args.cues.read_text())
    mix(cues, args.duration, args.out, music=args.music, music_gain=args.music_gain, music_offset=args.music_offset)
    print(args.out)


if __name__ == "__main__":
    sys.exit(main())
