"""Synthesize the demo video's background track: original, so it carries no license.

usage: python scripts/media/music.py <seconds> <out.wav>

100 BPM in D major (I–vi–IV–V). A pad opens, then arpeggio and bass, drums from the third bar,
a melody over the second half, and the drums drop out for the last two bars as it fades.
"""
import sys

import numpy as np
from scipy.io import wavfile
from scipy.signal import butter, sosfilt

SR = 44100
BPM = 100
BEAT = 60 / BPM
BAR = 4 * BEAT

CHORDS = [
    [50, 54, 57, 61],  # Dmaj7
    [47, 50, 54, 57],  # Bm7
    [43, 47, 50, 54],  # Gmaj7
    [45, 49, 52, 54],  # A6
]
ROOTS = [38, 35, 31, 33]
ARP = [0, 2, 1, 3, 2, 1, 3, 2]
MELODY = [  # (bar offset, beat, midi note, length in beats)
    (0, 0, 73, 1.5), (0, 1.5, 71, 0.5), (0, 2, 69, 2),
    (1, 0, 66, 1.5), (1, 1.5, 69, 0.5), (1, 2, 71, 2),
    (2, 0, 74, 1.5), (2, 1.5, 73, 0.5), (2, 2, 71, 1), (2, 3, 69, 1),
    (3, 0, 73, 3),
]


def hz(midi):
    return 440.0 * 2 ** ((midi - 69) / 12)


def adsr(n, a, d, s, r):
    a, d, r = int(a * SR), int(d * SR), int(r * SR)
    e = np.full(n, s)
    e[: min(a, n)] = np.linspace(0, 1, a)[: min(a, n)]
    if a < n:
        k = min(d, n - a)
        e[a : a + k] = np.linspace(1, s, d)[:k]
    if r and n > r:
        e[-r:] *= np.linspace(1, 0, r)
    return e


def lowpass(x, fc):
    return sosfilt(butter(2, fc, "low", fs=SR, output="sos"), x)


def highpass(x, fc):
    return sosfilt(butter(2, fc, "high", fs=SR, output="sos"), x)


def render(duration, path):
    n_total = int(SR * (duration + 1))
    rng = np.random.default_rng(7)
    left, right = np.zeros(n_total), np.zeros(n_total)

    def add(bl, br, x, t, pan=0.0, gain=1.0):
        i = int(t * SR)
        if i >= n_total:
            return
        x = x[: n_total - i] * gain
        bl[i : i + len(x)] += x * np.sqrt((1 - pan) / 2)
        br[i : i + len(x)] += x * np.sqrt((1 + pan) / 2)

    bars = int(np.ceil(duration / BAR))
    intro, drums_in = 1, 2
    outro = max(drums_in + 1, bars - 2)

    # pad: detuned saws, low-passed, slow swell
    for b in range(bars):
        n = int((BAR + 0.6) * SR)
        tt = np.arange(n) / SR
        sig = np.zeros(n)
        for m in CHORDS[b % 4]:
            for det in (-0.08, 0.0, 0.08):
                sig += 2 * ((tt * hz(m + 12) * 2 ** (det / 12) + rng.random()) % 1) - 1
        add(left, right, lowpass(sig, 1400) * adsr(n, 0.5, 0.4, 0.8, 0.6), b * BAR, 0, 0.018)

    # bass
    for b in range(intro, bars):
        for beat in (0, 2.5):
            n = int(BEAT * 1.4 * SR)
            tt = np.arange(n) / SR
            f = hz(ROOTS[b % 4])
            x = (np.sin(2 * np.pi * f * tt) + 0.25 * np.sin(4 * np.pi * f * tt)) * adsr(n, 0.01, 0.2, 0.6, 0.15)
            add(left, right, x, b * BAR + beat * BEAT, 0, 0.22)

    # pluck arpeggio in eighths, with a ping-pong echo
    arp_l, arp_r = np.zeros(n_total), np.zeros(n_total)
    for b in range(intro, bars):
        for k in range(8):
            n = int(0.6 * SR)
            tt = np.arange(n) / SR
            f = hz(CHORDS[b % 4][ARP[k]] + 12)
            x = (np.sin(2 * np.pi * f * tt) + 0.3 * np.sin(4 * np.pi * f * tt) + 0.1 * np.sin(6 * np.pi * f * tt)) * np.exp(-tt * 9)
            add(arp_l, arp_r, x, b * BAR + k * BEAT / 2, -0.25 + 0.5 * (k % 2), 0.07)
    d = int(BEAT * 0.75 * SR)
    echo_l, echo_r = np.zeros(n_total), np.zeros(n_total)
    echo_l[d:] += arp_r[:-d] * 0.35
    echo_r[d:] += arp_l[:-d] * 0.35
    echo_l[2 * d :] += arp_l[: -2 * d] * 0.15
    echo_r[2 * d :] += arp_r[: -2 * d] * 0.15
    left += arp_l + lowpass(echo_l, 3000)
    right += arp_r + lowpass(echo_r, 3000)

    # melody over the second half
    start = (bars // 2) // 4 * 4 if bars >= 8 else bars
    for rep in range(start, outro, 4):
        for bo, bt, m, ln in MELODY:
            t = (rep + bo) * BAR + bt * BEAT
            if t >= outro * BAR:
                continue
            n = int(ln * BEAT * SR)
            tt = np.arange(n) / SR
            f = hz(m)
            vib = 1 + 0.003 * np.sin(2 * np.pi * 5 * tt)
            x = np.sin(2 * np.pi * f * np.cumsum(vib) / SR) + 0.2 * np.sin(4 * np.pi * f * tt)
            add(left, right, x * adsr(n, 0.04, 0.2, 0.7, min(0.2, ln * BEAT * 0.5)), t, 0.1, 0.05)

    # drums: kick, soft clap on 2 and 4, hats in eighths
    kick_env = np.zeros(n_total)
    for b in range(drums_in, outro):
        for beat in (0, 2, 2.5):
            if beat == 2.5 and b % 2 == 0:
                continue
            t = b * BAR + beat * BEAT
            n = int(0.35 * SR)
            tt = np.arange(n) / SR
            x = np.sin(2 * np.pi * np.cumsum(45 + 90 * np.exp(-tt * 30)) / SR) * np.exp(-tt * 9)
            add(left, right, x, t, 0, 0.36)
            i = int(t * SR)
            seg = kick_env[i : i + n]
            kick_env[i : i + n] = np.maximum(seg, np.exp(-tt * 7)[: len(seg)])
        for beat in (1, 3):
            n = int(0.25 * SR)
            tt = np.arange(n) / SR
            add(left, right, highpass(lowpass(rng.standard_normal(n), 4000), 900) * np.exp(-tt * 18), b * BAR + beat * BEAT, 0, 0.09)
        for k in range(8):
            n = int(0.06 * SR)
            tt = np.arange(n) / SR
            x = highpass(rng.standard_normal(n), 7000) * np.exp(-tt * 70)
            add(left, right, x, b * BAR + k * BEAT / 2, 0.3 if k % 2 else -0.3, 0.05 if k % 2 else 0.025)

    # a cymbal swell as the drums come in
    n = int(2.0 * SR)
    add(left, right, highpass(rng.standard_normal(n), 5000) * np.exp(-np.arange(n) / SR * 2.5), drums_in * BAR, 0, 0.06)

    # the kick ducks the rest
    duck = 1 - 0.35 * kick_env
    left *= duck
    right *= duck

    mix = np.stack([highpass(left, 30), highpass(right, 30)], axis=1)[: int(duration * SR)]
    mix = np.tanh(mix * 1.2)
    fade_in, fade_out = int(1.0 * SR), int(3.0 * SR)
    mix[:fade_in] *= np.linspace(0, 1, fade_in)[:, None]
    mix[-fade_out:] *= np.linspace(1, 0, fade_out)[:, None] ** 1.5
    mix *= 0.7 / np.max(np.abs(mix))
    wavfile.write(path, SR, (mix * 32767).astype(np.int16))


if __name__ == "__main__":
    render(float(sys.argv[1]), sys.argv[2])
