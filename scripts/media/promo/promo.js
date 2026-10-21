'use strict';
/*
 * Hatoba's promo film. Every frame is a pure function of time: seek(t) lays out the whole stage for
 * second t, so render.py can step through it frame by frame and a browser can play it live.
 *
 * Scenes are authored in seconds on a 120 BPM grid, a beat every half second. ?bpm= stretches the
 * film to the tempo of another track, and ?lang=zh switches the copy to Chinese.
 */

const Q = new URLSearchParams(location.search);
const LANG = Q.get('lang') === 'zh' ? 'zh' : 'en';
const RATE = (Number(Q.get('bpm')) || 120) / 120;
const RENDER = Q.has('render');
const W = 1920;
const H = 1080;
const END = 70;
const ICON_SRC = '../../../apps/desktop/src-tauri/app-icon.svg';

const COPY = {
  en: {
    sea1: 'Your servers are everywhere.',
    sea2: 'Give them a harbor.',
    tagline: 'Open-source SSH client with AI and encrypted sync',
    hosts: 'All your hosts in one place.',
    hostsSub: 'Import from ~/.ssh/config, PuTTY, Xshell, MobaXterm, and more.',
    term: 'Terminal and files, side by side.',
    termSub: 'Tabs, truecolor, CJK input, SFTP, and port forwarding.',
    ai: 'An AI assistant beside every terminal.',
    aiSub1: 'It reads the screen, runs commands, and edits files.',
    aiSub2: 'Found, fixed, and verified.',
    prompt: 'nginx keeps returning 502. Find out why and fix it.',
    dash: 'Every server at a glance.',
    dashSub: 'Read over SSH, with nothing to install on the servers.',
    sync: 'Sync through your own Cloudflare account.',
    syncSub1: 'End-to-end encrypted: your account only ever holds ciphertext.',
    syncSub2: 'Local first. Everything works offline.',
    cloud: 'Your Cloudflare account',
    chat: 'Fix nginx 502',
    openSource: 'Open source · MIT',
  },
  zh: {
    sea1: '服务器，散落在各处。',
    sea2: '给它们一个港湾。',
    tagline: '内置 AI 与加密同步的开源 SSH 客户端',
    hosts: '所有主机，都在这里。',
    hostsSub: '可以从 ~/.ssh/config、PuTTY、Xshell、MobaXterm 等导入。',
    term: '终端和文件，并排在手边。',
    termSub: '多标签、真彩色、中日韩输入法、SFTP 和端口转发。',
    ai: '每个终端旁，都有 AI 助手。',
    aiSub1: '它会读取屏幕、运行命令、编辑文件。',
    aiSub2: '找到原因，修好，再验证一遍。',
    prompt: 'nginx 一直返回 502，找出原因并修好它。',
    dash: '每台服务器的状态，一眼看清。',
    dashSub: '通过 SSH 读取，服务器上什么都不用装。',
    sync: '通过你自己的 Cloudflare 账户同步。',
    syncSub1: '端到端加密：你的账户里只有密文。',
    syncSub2: '本地优先，离线也能用。',
    cloud: '你的 Cloudflare 账户',
    chat: '修复 nginx 502',
    openSource: '开源 · MIT 许可',
  },
};
const C = COPY[LANG];
document.documentElement.lang = LANG === 'zh' ? 'zh-CN' : 'en';

/* ---------- math ---------- */

const clamp = (x, a = 0, b = 1) => Math.min(b, Math.max(a, x));
const lerp = (a, b, k) => a + (b - a) * k;
const prog = (t, t0, d) => clamp((t - t0) / d);
const ease = {
  out: (x) => 1 - (1 - x) ** 3,
  in: (x) => x ** 3,
  inOut: (x) => (x < 0.5 ? 4 * x ** 3 : 1 - (-2 * x + 2) ** 3 / 2),
  sine: (x) => (1 - Math.cos(Math.PI * x)) / 2,
};

// The step response of a damped spring released at dt = 0: starts still, overshoots by an amount
// set by the damping ratio z, and settles at 1. f is the natural frequency in Hz.
function spring(dt, f = 2.2, z = 0.6) {
  if (dt <= 0) return 0;
  const w = 2 * Math.PI * f;
  if (z >= 1) return 1 - Math.exp(-w * dt) * (1 + w * dt);
  const wd = w * Math.sqrt(1 - z * z);
  return 1 - Math.exp(-z * w * dt) * (Math.cos(wd * dt) + ((z * w) / wd) * Math.sin(wd * dt));
}

// Rises from 0 to 1 and back over d seconds.
function bump(t, t0, d) {
  const p = (t - t0) / d;
  return p > 0 && p < 1 ? Math.sin(Math.PI * p) : 0;
}

function rnd(i) {
  const x = Math.sin(i * 127.1 + 311.7) * 43758.5453;
  return x - Math.floor(x);
}

const lerpRect = (a, b, k) => ({
  x: lerp(a.x, b.x, k),
  y: lerp(a.y, b.y, k),
  w: lerp(a.w, b.w, k),
  h: lerp(a.h, b.h, k),
  r: lerp(a.r, b.r, k),
});
const bez = (a, c, b, u) => ({
  x: (1 - u) ** 2 * a.x + 2 * (1 - u) * u * c.x + u * u * b.x,
  y: (1 - u) ** 2 * a.y + 2 * (1 - u) * u * c.y + u * u * b.y,
});
const n2 = (n) => n.toFixed(2);

/* ---------- dom ---------- */

const stage = document.getElementById('stage');

function el(parent, cls, html, tag = 'div') {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (html) e.innerHTML = html;
  parent.appendChild(e);
  return e;
}

const NS = 'http://www.w3.org/2000/svg';
function sv(parent, tag, attrs = {}) {
  const e = document.createElementNS(NS, tag);
  for (const k in attrs) e.setAttribute(k, attrs[k]);
  parent.appendChild(e);
  return e;
}

function shown(e, on) {
  const d = on ? '' : 'none';
  if (e.style.display !== d) e.style.display = d;
  return on;
}

function put(e, x, y, s = 1, o = 1, r = 0) {
  e.style.transform = `translate(${n2(x)}px,${n2(y)}px) rotate(${n2(r)}deg) scale(${s.toFixed(4)})`;
  e.style.opacity = clamp(o).toFixed(3);
}

function text(e, s) {
  if (e.textContent !== s) e.textContent = s;
}

const frameFns = [];
const onFrame = (fn) => frameFns.push(fn);
const measures = [];
const onMeasure = (fn) => measures.push(fn);

// Sound effects for sfx.py: the name of a sound, when it starts, and optional gain (dB), pan
// (-1 to 1) and pitch (semitones).
const CUES = [];
const cue = (t, name, opt = {}) => CUES.push({ t, name, ...opt });

const layer = (cls = '') => el(stage, `layer ${cls}`);
const L = {
  bg: layer('bg'),
  shadow: layer(),
  sea: layer(),
  lock: layer(),
  app: layer(),
  sync: layer(),
  end: layer(),
  wipe: layer(),
  caps: layer(),
  fx: layer(),
  cursor: layer(),
};

/* ---------- words that spring into place ---------- */

const CJK = '　-〿぀-ヿ㐀-鿿＀-￯';
const UNIT = new RegExp(`[${CJK}]|[^\\s${CJK}]+|\\s+`, 'g');

class Words {
  // Latin words move as one, CJK characters one by one; `chars` splits every character.
  constructor(parent, str, cls, chars = false) {
    this.root = el(parent, cls);
    this.units = [];
    for (const u of chars ? [...str] : str.match(UNIT)) {
      if (/^\s+$/.test(u)) {
        this.root.append(' ');
        continue;
      }
      const s = el(this.root, 'u', '', 'span');
      s.textContent = u;
      this.units.push(s);
    }
  }

  update(t, t0, t1, o = {}) {
    const { gap = 0.045, f = 2.3, z = 0.62, rise = 44, blur = 10, spin = 0 } = o;
    const n = this.units.length;
    const on = t >= t0 && (t1 == null || t < t1 + 0.45 + n * gap * 0.5);
    if (!shown(this.root, on)) return;
    this.units.forEach((s, i) => {
      const a = spring(t - t0 - i * gap, f, z);
      const b = t1 == null ? 0 : ease.in(prog(t, t1 + i * gap * 0.5, 0.32));
      s.style.opacity = (clamp(a * 1.8) * (1 - b)).toFixed(3);
      s.style.transform = `translateY(${n2((1 - a) * rise - b * 26)}px) rotate(${n2((1 - a) * spin)}deg) scale(${(0.86 + 0.14 * a).toFixed(4)})`;
      const bl = (1 - clamp(a)) * blur + b * 8;
      s.style.filter = bl > 0.2 ? `blur(${n2(bl)}px)` : 'none';
    });
  }
}

function caption(str, t0, t1, cls, top, o) {
  const w = new Words(L.caps, str, `words ${cls}`);
  w.root.style.top = `${top}px`;
  onFrame((t) => w.update(t, t0, t1, o));
}

/* ---------- light background ---------- */

{
  const blobs = [
    ['rgba(34,166,232,.32)', 560, 260, 180],
    ['rgba(150,125,255,.22)', 640, 1700, 240],
    ['rgba(255,143,177,.20)', 520, 1340, 1000],
    ['rgba(64,200,214,.22)', 600, 220, 980],
  ].map(([c, r, x, y], i) => {
    const e = el(L.bg, 'blob');
    e.style.width = e.style.height = `${2 * r}px`;
    e.style.background = `radial-gradient(circle closest-side, ${c}, transparent)`;
    return { e, r, x, y, i };
  });
  const waves = el(L.bg, 'bg-waves');
  onFrame((t) => {
    if (!shown(L.bg, t >= 7.9 && t < 48.3)) return;
    for (const b of blobs) {
      const x = b.x - b.r + Math.sin(t * 0.23 + b.i * 1.7) * 80;
      const y = b.y - b.r + Math.cos(t * 0.19 + b.i * 2.3) * 60;
      b.e.style.transform = `translate(${n2(x)}px,${n2(y)}px)`;
    }
    waves.style.transform = `translateX(${n2((t * 14) % 112)}px)`;
  });
}

/* ---------- cursor ---------- */

const cursor = (() => {
  const e = el(
    L.cursor,
    'cursor',
    '<svg width="44" height="44" viewBox="0 0 44 44"><path d="M9 5V35.5L16.6 28.4L21.6 39.6L27.4 37L22.4 26.2H32.6Z" fill="#fff" stroke="#0f1d3d" stroke-width="2.6" stroke-linejoin="round"/></svg>',
  );
  const keys = [];
  const clicks = [];
  const spans = [];
  const ripples = [];

  function pos(t) {
    let x = keys[0].x;
    let y = keys[0].y;
    for (const k of keys) {
      if (t < k.t) break;
      const p = k.d ? ease.inOut(prog(t, k.t, k.d)) : 1;
      if (p < 1) {
        // A slight arc, as a hand moves.
        const dx = k.x - x;
        const dy = k.y - y;
        const len = Math.hypot(dx, dy) || 1;
        const arc = Math.sin(Math.PI * p) * Math.min(70, len * 0.14);
        return { x: lerp(x, k.x, p) + (dy / len) * arc, y: lerp(y, k.y, p) - (dx / len) * arc };
      }
      x = k.x;
      y = k.y;
    }
    return { x, y };
  }

  onFrame((t) => {
    for (const r of ripples) {
      const p = (t - r.t) / 0.55;
      if (!shown(r.e, p >= 0 && p < 1)) continue;
      const c = pos(r.t);
      put(r.e, c.x, c.y, 0.3 + ease.out(p), (1 - p) * 0.9);
    }
    const span = spans.find(([a, b]) => t >= a && t < b + 0.3);
    if (!shown(e, !!span)) return;
    const a = spring(t - span[0], 2.4, 0.6);
    const b = ease.in(prog(t, span[1], 0.3));
    let press = 0;
    for (const c of clicks) press = Math.max(press, bump(t, c - 0.05, 0.22));
    const p = pos(t);
    e.style.transform = `translate(${n2(p.x - 9)}px,${n2(p.y - 5)}px) scale(${((0.6 + 0.4 * a) * (1 - 0.18 * press) * (1 - 0.3 * b)).toFixed(4)})`;
    e.style.opacity = (clamp(a * 2) * (1 - b)).toFixed(3);
  });

  return {
    pos,
    show: (t0, t1) => spans.push([t0, t1]),
    at: (t, x, y) => keys.push({ t, x, y, d: 0 }),
    to: (t, x, y, d = 0.6) => keys.push({ t, x, y, d }),
    click(t, sound = true) {
      clicks.push(t);
      ripples.push({ t, e: el(L.fx, 'ripple') });
      if (sound) cue(t, 'click', { gain: -4 });
    },
  };
})();

/* ---------- effects ---------- */

const PARTY = ['#22a6e8', '#5ee08a', '#ffd166', '#ff8fb1', '#a78bfa', '#e3eeff', '#0a84ff'];
const STAR_PATH = 'M0 -16C2 -4 4 -2 16 0C4 2 2 4 0 16C-2 4 -4 2 -16 0C-4 -2 -2 -4 0 -16Z';

function burst(t0, x, y, n, seed, { up = 1, speed = 1 } = {}) {
  const parts = [];
  for (let i = 0; i < n; i++) {
    const k = seed * 100 + i;
    const kind = i % 3;
    const e = el(L.fx, 'confetti');
    const size = 10 + rnd(k + 1) * 10;
    const color = PARTY[i % PARTY.length];
    if (kind === 2) {
      e.innerHTML = `<svg width="${size * 2}" height="${size * 2}" viewBox="-16 -16 32 32"><path d="${STAR_PATH}" fill="${color}"/></svg>`;
      e.style.margin = `${-size}px 0 0 ${-size}px`;
    } else {
      e.style.cssText = `width:${size}px;height:${kind ? size * 1.6 : size}px;margin:${-size / 2}px 0 0 ${-size / 2}px;border-radius:${kind ? 4 : size}px;background:${color}`;
    }
    const ang = -Math.PI / 2 + (rnd(k + 2) - 0.5) * 2.7 * up;
    const v = (520 + rnd(k + 3) * 520) * speed;
    parts.push({ e, vx: Math.cos(ang) * v, vy: Math.sin(ang) * v, spin: (rnd(k + 4) - 0.5) * 900, r0: rnd(k + 5) * 360 });
  }
  onFrame((t) => {
    const dt = t - t0;
    const on = dt >= 0 && dt < 1.5;
    for (const p of parts) {
      if (!shown(p.e, on)) continue;
      const drag = (1 - Math.exp(-2.4 * dt)) / 2.4;
      put(p.e, x + p.vx * drag, y + p.vy * drag + 700 * dt * dt, 1, 1 - prog(dt, 0.75, 0.7), p.r0 + p.spin * dt);
    }
  });
}

function twinkle(t0, x, y, size = 1, color = '#fff') {
  const e = el(L.fx, 'twinkle', `<svg width="34" height="34" viewBox="-17 -17 34 34"><path d="${STAR_PATH}" fill="${color}"/></svg>`);
  onFrame((t) => {
    const a = bump(t, t0, 0.8);
    if (!shown(e, a > 0)) return;
    put(e, x, y, a * size * 1.3, a, (t - t0) * 120);
  });
}

/* ---------- the sea: intro, logo, and the night of the last scenes ---------- */

let LOCK_DX = -330; // how far the icon moves left to make room for the wordmark; set once fonts load
const ICON_BOX = { x: 64, y: 64, w: 896, h: 896, r: 208 };
const SEA_START = { x: -292, y: 58, w: 1608, h: 908, r: 0 };
const SEA_ALL = { x: -6000, y: -6000, w: 13000, h: 13000, r: 0 };

const sea = (() => {
  const shadow = el(L.shadow, 'icon-shadow');
  const svg = sv(L.sea, 'svg', { width: W, height: H, viewBox: `0 0 ${W} ${H}` });
  svg.innerHTML = `<defs>
    <linearGradient id="g-sky" x1="0" y1="64" x2="0" y2="698" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="#0d1735"/><stop offset="1" stop-color="#24408a"/></linearGradient>
    <linearGradient id="g-pier" x1="0" y1="272" x2="0" y2="698" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="#ffffff"/><stop offset="1" stop-color="#e3eeff"/></linearGradient>
    <linearGradient id="g-water" x1="0" y1="662" x2="0" y2="960" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="#22a6e8"/><stop offset="1" stop-color="#0d4590"/></linearGradient>
    <linearGradient id="g-depth" x1="0" y1="662" x2="0" y2="960" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="#ffffff" stop-opacity=".42"/><stop offset="1" stop-color="#ffffff" stop-opacity=".04"/></linearGradient>
    <linearGradient id="g-hull" x1="0" y1="-16" x2="0" y2="40" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="#ffffff"/><stop offset="1" stop-color="#d2e2ff"/></linearGradient>
    <radialGradient id="g-glow"><stop offset="0" stop-color="#fff4cf" stop-opacity=".35"/><stop offset="1" stop-color="#fff4cf" stop-opacity="0"/></radialGradient>
    <mask id="m-moon"><circle r="46" fill="#fff"/><circle cx="22" cy="-16" r="40" fill="#000"/></mask>
    <g id="rings" stroke="#ffffff" stroke-width="5"><circle r="56" fill="#000000"/><circle r="40.3" fill="none"/><circle r="24.6" fill="none"/><circle r="9" fill="none"/></g>
    <pattern id="p-seig" x="64" y="662" width="112" height="56" patternUnits="userSpaceOnUse">
      <use href="#rings" x="56" y="-28"/><use href="#rings" x="0" y="0"/><use href="#rings" x="112" y="0"/><use href="#rings" x="56" y="28"/><use href="#rings" x="0" y="56"/><use href="#rings" x="112" y="56"/><use href="#rings" x="56" y="84"/>
    </pattern>
    <mask id="m-waves" maskUnits="userSpaceOnUse" x="-3000" y="560" width="8000" height="4000"><rect x="-3000" y="560" width="8000" height="4000" fill="url(#p-seig)"/></mask>
    <clipPath id="c-frame"><rect id="frame-rect"/></clipPath>
    <clipPath id="c-below"><path id="below-path"/></clipPath>
    <clipPath id="c-above"><path id="above-path"/></clipPath>
  </defs>`;
  const frameRect = svg.querySelector('#frame-rect');
  const belowPath = svg.querySelector('#below-path');
  const abovePath = svg.querySelector('#above-path');
  const pattern = svg.querySelector('#p-seig');

  const cam = sv(svg, 'g');
  const framed = sv(cam, 'g', { 'clip-path': 'url(#c-frame)' });
  sv(framed, 'rect', { x: -6000, y: -6000, width: 13000, height: 13000, fill: 'url(#g-sky)' });

  const moon = sv(framed, 'g');
  moon.innerHTML = '<circle r="150" fill="url(#g-glow)"/><circle r="46" fill="#fff4cf" mask="url(#m-moon)"/>';

  const starsG = sv(framed, 'g');
  const stars = [];
  for (let i = 0; i < 90; i++) {
    const big = i % 9 === 0;
    const e = big
      ? sv(starsG, 'path', { d: STAR_PATH, fill: '#ffffff' })
      : sv(starsG, 'circle', { r: (1.5 + rnd(i + 7) * 2.3).toFixed(2), fill: '#ffffff' });
    stars.push({ e, big, x: -1000 + rnd(i) * 3000, y: -760 + rnd(i + 99) * 1330, w: 1.1 + rnd(i + 3) * 2.4, p: rnd(i + 5) * 6.28 });
  }

  const pierWrap = sv(framed, 'g');
  const pier = sv(pierWrap, 'g', { fill: 'url(#g-pier)' });
  sv(pier, 'rect', { x: 288, y: 272, width: 104, height: 688, rx: 52 });
  sv(pier, 'rect', { x: 632, y: 272, width: 104, height: 688, rx: 52 });
  const bar = sv(pier, 'rect', { x: 340, y: 500, width: 344, height: 88 });

  const water = sv(framed, 'path', { fill: 'url(#g-water)' });
  const below = sv(framed, 'g', { 'clip-path': 'url(#c-below)' });
  sv(below, 'rect', { x: -3000, y: 560, width: 8000, height: 4000, fill: '#ffffff', 'fill-opacity': 0.07, mask: 'url(#m-waves)' });
  const surf = sv(below, 'path', { fill: 'none', stroke: '#b5ecff', 'stroke-opacity': 0.7, 'stroke-width': 14 });
  const depth = sv(below, 'g', { fill: 'url(#g-depth)' });
  sv(depth, 'rect', { x: 288, y: 640, width: 104, height: 320 });
  sv(depth, 'rect', { x: 632, y: 640, width: 104, height: 320 });

  const boatsG = sv(framed, 'g', { 'clip-path': 'url(#c-above)' });
  const splashG = sv(framed, 'g');
  const rim = sv(cam, 'rect', { x: 64.5, y: 64.5, width: 895, height: 895, rx: 207.5, fill: 'none', stroke: '#ffffff', 'stroke-opacity': 0.1 });

  const surfY = (x, phi, A) => 680 - A * Math.cos((2 * Math.PI * (x - 64 - phi)) / 448);
  const slope = (x, phi, A) => A * ((2 * Math.PI) / 448) * Math.sin((2 * Math.PI * (x - 64 - phi)) / 448);

  // The intro's waves slow down and stop exactly on the icon's surface as the frame closes.
  function phase(t) {
    if (t < 20) {
      const u = 10.4 - t;
      const D = 2.4;
      return u <= 0 ? 0 : 36 * (u >= D ? u - D / 2 : (u * u) / (2 * D));
    }
    return -34 * (t - 48);
  }
  const amp = (t) => (t < 20 ? 18 + 8 * ease.sine(clamp((10.4 - t) / 5)) : 21);

  function camera(t) {
    if (t < 8) {
      const p = ease.sine(prog(t, 0, 8));
      return { s: lerp(1.27, 1.2, p), cx: lerp(566, 512, p), cy: 512, dx: 0, frame: SEA_ALL, e: 0 };
    }
    if (t < 20) {
      const e = spring(t - 8, 0.62, 0.8);
      return { s: lerp(1.2, 0.4, e), cx: 512, cy: 512, dx: LOCK_DX * spring(t - 10.0, 0.95, 0.75), frame: lerpRect(SEA_START, ICON_BOX, e), e };
    }
    return { s: 1.1, cx: 512, cy: lerp(-40, 404, spring(t - 58.25, 0.55, 0.9)), dx: 0, frame: SEA_ALL, e: 0 };
  }

  // Boats are servers: they bob on the waves, blink, and dock at the pier as it rises.
  const BOATS = [
    { name: 'prod-api-tokyo', tag: '#e0654f', dir: -1, seed: 1, t0: 1.0, keys: [[1.0, 430], [4.5, 176, 1.2]], end: [[59.0, -430], [59.1, -60, 2.8]] },
    { name: 'db-osaka-01', tag: '#8c66c9', dir: -1, seed: 2, t0: 1.45, keys: [[1.45, 1100], [4.7, 1230, 1.3]], end: [[59.0, 1460], [59.25, 1170, 2.8]] },
    { name: 'homelab-pi', tag: '#9a9ca6', dir: 1, seed: 3, t0: 1.9, keys: [[1.9, -110], [4.4, -640, 2.0]] },
    { name: 'staging-web', tag: '#d7a43c', dir: 1, seed: 4, t0: 2.35, keys: [[2.35, 700], [4.6, 848, 1.2]] },
  ];
  const boats = BOATS.map((b) => {
    const g = sv(boatsG, 'g');
    g.innerHTML = `<ellipse cx="0" cy="12" rx="108" ry="10" fill="#ffffff" fill-opacity=".22"/>
      <line x1="0" y1="-60" x2="0" y2="-124" stroke="#e3eeff" stroke-width="5" stroke-linecap="round"/>
      <g class="flag"><rect y="-142" height="36" rx="18" fill="#ffffff"/><circle cy="-124" r="6.5" fill="${b.tag}"/><text y="-117.5" font-family="Cascadia Mono, Consolas, monospace" font-size="19" font-weight="600" fill="#0f1d3d">${b.name}</text></g>
      <rect x="-40" y="-64" width="80" height="54" rx="20" fill="#22a6e8"/>
      <rect x="-40" y="-64" width="80" height="54" rx="20" fill="none" stroke="#ffffff" stroke-opacity=".55" stroke-width="4"/>
      <circle class="glow" cx="0" cy="-37" r="17" fill="#5ee08a" fill-opacity=".3"/>
      <circle cx="0" cy="-37" r="9" fill="#5ee08a"/>
      <path d="M-98 -16H98C94 18 68 38 34 40H-34C-68 38 -94 18 -98 -16Z" fill="url(#g-hull)"/>
      <path d="M-100 -16H100" stroke="#ffffff" stroke-width="8" stroke-linecap="round"/>
      <ellipse cx="-47" cy="13" rx="11" ry="6" fill="#ff8fb1" fill-opacity=".6"/>
      <ellipse cx="47" cy="13" rx="11" ry="6" fill="#ff8fb1" fill-opacity=".6"/>
      <g class="eyes"><ellipse cx="-25" cy="4" rx="6.5" ry="8" fill="#0d1735"/><ellipse cx="25" cy="4" rx="6.5" ry="8" fill="#0d1735"/><circle cx="-22.5" cy="0.5" r="2.4" fill="#fff"/><circle cx="27.5" cy="0.5" r="2.4" fill="#fff"/></g>
      <path d="M-8 14Q0 21 8 14" fill="none" stroke="#0d1735" stroke-width="3.6" stroke-linecap="round"/>`;
    return { ...b, g, eyes: g.querySelector('.eyes'), glow: g.querySelector('.glow'), flag: g.querySelector('.flag') };
  });
  onMeasure(() => {
    for (const b of boats) {
      const label = b.flag.querySelector('text');
      const w = label.getComputedTextLength() + 54;
      const x0 = b.dir > 0 ? 8 : -8 - w;
      b.flag.querySelector('rect').setAttribute('x', x0);
      b.flag.querySelector('rect').setAttribute('width', w);
      b.flag.querySelector('circle').setAttribute('cx', x0 + 21);
      label.setAttribute('x', x0 + 36);
    }
  });

  function keyX(keys, t) {
    let x = keys[0][1];
    for (const [kt, kx, d] of keys) {
      if (t < kt) break;
      const p = d ? ease.inOut(prog(t, kt, d)) : 1;
      x = lerp(x, kx, p);
      if (p < 1) break;
    }
    return x;
  }

  function blink(t, seed) {
    const period = 2.6 + seed * 0.45;
    const m = (t + seed * 0.9) % period;
    return m < 0.16 ? Math.max(0.1, Math.abs(1 - m / 0.08)) : 1;
  }

  const splashes = [];
  function splash(t0, x, size, n = 8) {
    const g = sv(splashG, 'g');
    const drops = [];
    for (let i = 0; i < n; i++) drops.push(sv(g, 'circle', { r: ((3 + rnd(t0 * 10 + i) * 4.5) * size).toFixed(2), fill: '#e8f7ff' }));
    const rings = [0, 1].map(() => sv(g, 'ellipse', { fill: 'none', stroke: '#ffffff', 'stroke-width': 4 }));
    splashes.push({ t0, x, size, g, drops, rings, seed: t0 * 10 });
  }
  for (const b of BOATS) {
    splash(b.t0 + 0.05, b.keys[0][1], 0.8, 7);
    cue(b.t0, 'bubble', { pitch: [0, 3, 5, 7][b.seed - 1], gain: -6, pan: (b.keys[0][1] - 512) / 900 });
  }
  splash(5.55, 340, 1.4, 10);
  splash(5.57, 684, 1.4, 10);
  cue(5.5, 'splash', { gain: -3 });
  cue(5.55, 'thud', { gain: -4 });
  cue(6.15, 'clack', { gain: -6 });

  onFrame((t) => {
    if (!shown(L.sea, t < 13.62 || t >= 48.2)) {
      shown(shadow, false);
      return;
    }
    const c = camera(t);
    cam.setAttribute('transform', `translate(${n2(960 + c.dx)} 540) scale(${c.s.toFixed(5)}) translate(${n2(-c.cx)} ${n2(-c.cy)})`);
    const f = c.frame;
    frameRect.setAttribute('x', n2(f.x));
    frameRect.setAttribute('y', n2(f.y));
    frameRect.setAttribute('width', n2(f.w));
    frameRect.setAttribute('height', n2(f.h));
    frameRect.setAttribute('rx', n2(Math.max(0, f.r)));
    rim.setAttribute('stroke-opacity', (0.1 * clamp((c.e - 0.85) / 0.15)).toFixed(3));

    if (shown(shadow, t >= 8 && t < 13.62)) {
      const sx = 960 + c.dx + (f.x - c.cx) * c.s;
      const sy = 540 + (f.y - c.cy) * c.s;
      shadow.style.cssText = `left:${n2(sx)}px;top:${n2(sy)}px;width:${n2(f.w * c.s)}px;height:${n2(f.h * c.s)}px;border-radius:${n2(Math.max(0, f.r) * c.s)}px;opacity:${clamp(c.e * 3).toFixed(3)}`;
    }

    const phi = phase(t);
    const A = amp(t);
    pattern.setAttribute('patternTransform', `translate(${n2(phi % 448)} 0)`);
    let top = '';
    let above = '';
    const back = [];
    for (let x = -1000; x <= 2100; x += 8) {
      const y = surfY(x, phi, A);
      top += `${x === -1000 ? 'M' : 'L'}${x} ${n2(y)}`;
      back.push(`L${x} ${n2(y + 26)}`);
    }
    const seaD = `${top}L2100 6000L-1000 6000Z`;
    water.setAttribute('d', seaD);
    belowPath.setAttribute('d', seaD);
    surf.setAttribute('d', top);
    above = `M-1000 -6000L2100 -6000${back.reverse().join('')}Z`;
    abovePath.setAttribute('d', above);

    shown(depth, t < 20);
    if (shown(pierWrap, t < 20)) {
      const rise = (1 - spring(t - 5.45, 1.2, 0.66)) * 480;
      pier.setAttribute('transform', `translate(0 ${n2(rise)})`);
      depth.setAttribute('transform', `translate(0 ${n2(rise)})`);
      const k = spring(t - 6.15, 1.7, 0.5);
      bar.setAttribute('transform', `translate(512 0) scale(${k.toFixed(4)} 1) translate(-512 0)`);
    }

    const starFade = t < 20 ? clamp(1 - c.e * 1.6) : clamp((t - 48.2) / 0.6);
    starsG.setAttribute('opacity', starFade.toFixed(3));
    for (const s of stars) {
      const tw = 0.3 + 0.7 * (0.5 + 0.5 * Math.sin(t * s.w + s.p));
      if (s.big) s.e.setAttribute('transform', `translate(${n2(s.x)} ${n2(s.y)}) scale(${(0.35 + 0.45 * tw).toFixed(3)})`);
      else {
        s.e.setAttribute('cx', n2(s.x));
        s.e.setAttribute('cy', n2(s.y));
      }
      s.e.setAttribute('opacity', tw.toFixed(3));
    }
    const moonO = t < 20 ? clamp(1 - c.e * 2.5) : prog(t, 59.2, 1);
    if (shown(moon, moonO > 0)) {
      moon.setAttribute('transform', `translate(1150 ${n2(170 + Math.sin(t * 0.8) * 6)}) rotate(-18)`);
      moon.setAttribute('opacity', moonO.toFixed(3));
    }

    for (const b of boats) {
      const intro = t >= b.t0 && t < 10.3;
      const outro = !!b.end && t >= 59.0;
      if (!shown(b.g, intro || outro)) continue;
      const keys = intro ? b.keys : b.end;
      const x = keyX(keys, t);
      const v = (keyX(keys, t + 0.02) - keyX(keys, t - 0.02)) / 0.04;
      let y = surfY(x, phi, A) + 8 + Math.sin(t * 2.1 + b.seed) * 5;
      let open = blink(t, b.seed);
      if (intro) {
        y += (1 - spring(t - b.t0, 1.5, 0.42)) * 130;
        y += ease.in(prog(t, 8.8 + b.seed * 0.08, 0.9)) * 220;
        open = Math.min(open, lerp(1, 0.12, prog(t, 8.3, 0.25)));
      }
      const tilt = (Math.atan(slope(x, phi, A)) * 180) / Math.PI * 0.45 + Math.sin(t * 1.6 + b.seed * 2) * 2.5 - v * 0.012;
      b.g.setAttribute('transform', `translate(${n2(x)} ${n2(y)}) rotate(${n2(tilt)})`);
      b.eyes.setAttribute('transform', `translate(0 4) scale(1 ${open.toFixed(3)}) translate(0 -4)`);
      b.glow.setAttribute('fill-opacity', (0.18 + 0.2 * (0.5 + 0.5 * Math.sin(t * 3 + b.seed))).toFixed(3));
    }

    for (const s of splashes) {
      const dt = t - s.t0;
      if (!shown(s.g, dt >= 0 && dt < 1.4 && t < 20)) continue;
      const y0 = surfY(s.x, phi, A);
      s.drops.forEach((d, i) => {
        const ang = -Math.PI / 2 + (rnd(s.seed + i * 3) - 0.5) * 1.9;
        const sp = (230 + rnd(s.seed + i * 5) * 280) * s.size;
        const py = y0 - 6 + Math.sin(ang) * sp * dt + 550 * dt * dt;
        d.setAttribute('cx', n2(s.x + Math.cos(ang) * sp * dt));
        d.setAttribute('cy', n2(py));
        d.setAttribute('opacity', (py < y0 + 4 ? clamp(1 - dt / 1.1) : 0).toFixed(3));
      });
      s.rings.forEach((r, k) => {
        const dk = dt - k * 0.18;
        const rx = (30 + 150 * ease.out(clamp(dk / 1.1))) * s.size;
        r.setAttribute('cx', n2(s.x));
        r.setAttribute('cy', n2(y0 + 4));
        r.setAttribute('rx', n2(rx));
        r.setAttribute('ry', n2(rx * 0.17));
        r.setAttribute('stroke-opacity', (dk > 0 ? (1 - clamp(dk / 1.1)) * 0.55 : 0).toFixed(3));
      });
    }
  });

  return { camera };
})();

cue(0, 'waves', { gain: -10, dur: 11 });
caption(C.sea1, 1.2, 4.0, 'headline big night', 96);
caption(C.sea2, 4.5, 7.7, 'headline big night', 96);
cue(8.0, 'whoosh', { gain: -4, dur: 1.6 });
cue(9.3, 'thud', { gain: -8 });

/* ---------- logo lockup ---------- */

{
  const mark = new Words(L.lock, 'Hatoba', 'wordmark', true);
  const tag = new Words(L.lock, C.tagline, 'words tagline');
  tag.root.style.top = '790px';
  onMeasure(() => {
    const total = 358.4 + 66 + mark.root.offsetWidth;
    LOCK_DX = 179.2 - total / 2;
  });
  onFrame((t) => {
    if (!shown(L.lock, t >= 10 && t < 14.2)) return;
    // The wordmark rides along as the icon makes room for it.
    mark.root.style.left = `${n2(960 + LOCK_DX * spring(t - 10.0, 0.95, 0.75) + 179.2 + 66)}px`;
    mark.root.style.top = '540px';
    mark.root.style.transform = 'translateY(-54%)';
    mark.update(t, 10.15, 13.4, { gap: 0.055, f: 2.0, z: 0.5, rise: 70, spin: -14, blur: 6 });
    tag.update(t, 10.9, 13.35, { gap: 0.03 });
  });
  cue(10.0, 'chime', { gain: -2 });
  [0, 2, 4, 7, 9, 12].forEach((p, i) => cue(10.2 + i * 0.055, 'pop', { pitch: p + 5, gain: -14 }));
  cue(13.35, 'swish', { gain: -10 });
}

/* ---------- the app window: snapshots of the real app, animated in place ---------- */

// capture.py saves each state the film shows from the app's mock frontend. Every snapshot loads into
// its own iframe at the app's own size, zoomed by Z, and the film moves its real elements.
const Z = 1.2;
const W1 = { x: 216, y: 252, w: 1488, h: 792, r: 12 };
const W2 = { x: 120, y: 252, w: 1680, h: 792, r: 12 };
const T_MORPH = 13.6;
const T_FILTER = 17.95;
const T_PICK = 19.75;
const T_TERM = 20.3;
const T_SFTP = 23.6;
const T_SFTP_OUT = 25.65;
const T_AI = 28.7;
const T_SEND = 30.8;
const RUNS = [32.65, 33.7, 35.9, 37.45];
const T_OK = RUNS[3] + 0.75; // the assistant's check in the terminal comes back 200
const T_DONE = T_OK + 0.15;
const T_AI_OUT = 41.2;
const T_DASH = 41.75;
const T_WIPE = 47.5;
const STAGES = [
  ['ai-0', T_SEND],
  ['ai-1', RUNS[0] + 0.12],
  ['ai-2', RUNS[1] + 0.12],
  ['ai-3', RUNS[2] + 0.12],
  ['ai-done', T_OK],
];
const SNAPS = ['hosts', 'term', 'sftp', 'ai-empty', ...STAGES.map((s) => s[0]), 'dash'];
const RUN_LABEL = LANG === 'zh' ? '运行' : 'Run';
const FIXED = LANG === 'zh' ? '修好了' : 'Fixed';
const WORD_RATE = 40; // streamed words per second, or faster when a stage needs it
const LEAD = 0.75; // how long an approval card is up before its Run is clicked
const SNAP_CSS = `html{zoom:${Z}}
*,*::before,*::after{transition:none!important;animation:none!important;caret-color:transparent!important}
::-webkit-scrollbar{display:none}
.promo-term{position:absolute;left:18px;top:14px;right:0;bottom:0;z-index:5;pointer-events:none;font:13px/19px "Cascadia Mono",Consolas,monospace;color:#d5d8de;white-space:pre}
.promo-term .l{position:absolute;left:0;height:19px;border-radius:3px}
.promo-term .k{position:absolute;left:0;top:0;height:19px;background:#d5d8de}
.promo-term .pu,.promo-term .g{color:#8bd49c}.promo-term .pp,.promo-term .b{color:#7ab7ff}.promo-term .y{color:#e5c07b}.promo-term .r{color:#f07178}.promo-term .dim{color:#6b717c}
.pw{transition:none}`;

// The terminal is xterm's canvas in the app, empty in a snapshot, so the film types its own lines
// over it in the same font, size, and colors (features/terminal/theme.ts and schemes.ts).
const PROMPT = '<span class="pu">deploy@prod-api-tokyo</span>:<span class="pp">/srv/api</span>$ ';
const PROMPT_LEN = 'deploy@prod-api-tokyo:/srv/api$ '.length;
const CURL = 'curl -sI http://localhost/v1/hosts';
const TERM_LINES = [
  { at: 20.9, html: 'Last login: Thu Oct  8 09:41:12 2026 from 10.0.0.4' },
  { at: 21.2, html: '<span class="pu">deploy@prod-api-tokyo</span>:<span class="pp">~</span>$ ', type: 'cd /srv/api && git log --oneline -3', t0: 21.45, cps: 34, len: 25 },
  { at: 22.6, html: '<span class="y">a3f91c2</span> (<span class="b">HEAD -&gt; main</span>, <span class="r">origin/main</span>) fix: retry D1 writes on 429' },
  { at: 22.66, html: '<span class="y">7be0d14</span> chore: bump tokio to 1.41' },
  { at: 22.72, html: '<span class="y">e02c5aa</span> feat: add /healthz latency histogram' },
  { at: 22.8, html: PROMPT, type: CURL, t0: 26.2, cps: 32 },
  { at: 27.35, html: 'HTTP/1.1 502 Bad Gateway', flash: 'bad' },
  { at: 27.41, html: 'Server: nginx/1.24.0 (Ubuntu)' },
  { at: 27.47, html: 'Content-Type: text/html' },
  { at: 27.55, html: PROMPT, type: CURL, t0: RUNS[3] + 0.12, cps: 60, ai: true },
  { at: T_OK, html: 'HTTP/1.1 200 OK', flash: 'good' },
  { at: T_OK + 0.06, html: 'Server: nginx/1.24.0 (Ubuntu)' },
  { at: T_OK + 0.12, html: 'Content-Type: application/json' },
  { at: T_OK + 0.2, html: PROMPT, type: '', t0: 99, cps: 1 },
];
TERM_LINES.forEach((l, i) => {
  if (!l.type) return;
  for (let k = 0; k < l.type.length; k++) cue(l.t0 + k / l.cps, 'key', { gain: -15 + rnd(i * 50 + k) * 4, pitch: (rnd(i * 70 + k) - 0.5) * 4 + (l.ai ? 3 : 0) });
});

const app = (() => {
  const iconRect = () => ({ x: 960 + LOCK_DX - 179.2, y: 540 - 179.2, w: 358.4, h: 358.4, r: 83.2 });
  const aiOpen = (t) => spring(t - T_AI, 1.0, 0.82) - spring(t - T_AI_OUT, 1.0, 0.9);
  function winRect(t) {
    const r = lerpRect(iconRect(), W1, spring(t - T_MORPH, 0.95, 0.8));
    const a = aiOpen(t);
    r.x += (W2.x - W1.x) * a;
    r.w += (W2.w - W1.w) * a;
    return r;
  }

  const win = el(L.app, 'win');
  const S = {};
  // A CSS-module class by its local name: `_row_nubwf_57` is part "row".
  const one = (root, part) => root.querySelector(`[class*="_${part}_"]`);
  const all = (root, part) => [...root.querySelectorAll(`[class*="_${part}_"]`)];
  const icon = el(win, 'win-icon', `<img src="${ICON_SRC}" alt="">`);
  const expander = el(win, 'expander');

  /* the terminal overlay, one per snapshot that shows the terminal */
  function termOverlay(doc) {
    const term = one(doc, 'term');
    if (!term) return null;
    if (getComputedStyle(term).position === 'static') term.style.position = 'relative';
    const box = doc.createElement('div');
    box.className = 'promo-term';
    term.appendChild(box);
    const lines = TERM_LINES.map((l, i) => {
      const e = doc.createElement('div');
      e.className = 'l';
      e.style.top = `${i * 19}px`;
      e.innerHTML = `${l.html}<span class="t"></span>`;
      box.appendChild(e);
      return { e, typed: e.querySelector('.t') };
    });
    const caret = doc.createElement('div');
    caret.className = 'k';
    box.appendChild(caret);
    return { box, lines, caret };
  }
  let charW = 7.8;

  function drawTerm(o, t) {
    let active = null;
    TERM_LINES.forEach((l, i) => {
      const { e, typed } = o.lines[i];
      const a = prog(t, l.at, 0.1);
      if (!shown(e, a > 0)) return;
      e.style.opacity = a.toFixed(3);
      let dx = 0;
      let bg = 'transparent';
      if (l.flash === 'bad' && t >= l.at) {
        const dt = t - l.at;
        dx = Math.sin(dt * 42) * 6 * Math.exp(-dt * 6);
        bg = `rgba(240,113,120,${(0.26 * bump(t, l.at, 1.3)).toFixed(3)})`;
      }
      if (l.flash === 'good') bg = `rgba(139,212,156,${(0.24 * bump(t, l.at, 1.3)).toFixed(3)})`;
      e.style.background = bg;
      e.style.padding = l.flash ? '0 6px' : '';
      e.style.marginLeft = l.flash ? '-6px' : '';
      e.style.transform = `translateX(${n2(dx)}px)`;
      if (l.type != null) {
        const n = t < l.t0 ? 0 : Math.min(l.type.length, Math.floor((t - l.t0) * l.cps) + 1);
        text(typed, l.type.slice(0, n));
        active = { l, i, n };
      }
    });
    if (shown(o.caret, !!active)) {
      const len = active.l.len ?? PROMPT_LEN;
      o.caret.style.width = `${n2(charW)}px`;
      o.caret.style.transform = `translate(${n2((len + active.n) * charW)}px,${active.i * 19}px)`;
    }
  }

  /* loading and measuring */
  async function load() {
    for (const name of SNAPS) {
      const res = await fetch(`out/snapshots/${LANG}/${name}.html`);
      if (!res.ok) throw new Error(`no snapshot out/snapshots/${LANG}/${name}.html: run capture.py with \`pnpm dev\` running first`);
      const html = await res.text();
      const f = el(win, 'snap', '', 'iframe');
      const loaded = new Promise((r) => (f.onload = r));
      f.srcdoc = html.replace('</head>', `<style>${SNAP_CSS}</style></head>`);
      await loaded;
      const doc = f.contentDocument;
      f.style.width = `${(name.startsWith('ai') ? W2 : W1).w}px`;
      f.style.height = `${W1.h}px`;
      void doc.body.offsetHeight;
      await doc.fonts.ready;
      for (const e of doc.querySelectorAll('[data-value]')) e.value = e.getAttribute('data-value');
      for (const e of doc.querySelectorAll('[data-scroll]')) {
        const [x, y] = e.getAttribute('data-scroll').split(',').map(Number);
        e.scrollLeft = x;
        e.scrollTop = y;
      }
      S[name] = { f, doc, term: termOverlay(doc) };
    }
    const probe = S.term.doc.createElement('span');
    probe.style.cssText = 'font:13px "Cascadia Mono",Consolas,monospace;position:absolute;white-space:pre';
    probe.textContent = 'M'.repeat(40);
    S.term.doc.body.appendChild(probe);
    charW = probe.getBoundingClientRect().width / 40 / Z;
    probe.remove();
    setup();
  }

  // Stage coordinates of an element's center inside a snapshot shown at window rect w.
  function at(snap, e, w, dx = 0.5, dy = 0.5) {
    const r = e.getBoundingClientRect();
    return { x: w.x + r.x + r.width * dx, y: w.y + r.y + r.height * dy };
  }
  function rectOf(snap, e, w) {
    const r = e.getBoundingClientRect();
    return { x: w.x + r.x, y: w.y + r.y, w: r.width, h: r.height };
  }
  const byText = (root, sel, txt) => [...root.querySelectorAll(sel)].find((e) => e.textContent.trim() === txt);
  // Where a stage's conversation is scrolled to at time t.
  const scrollAt = (snap, t) => snap.moves.reduce((y, m) => y + m.d * spring(t - m.t + 0.05, 1.5, 0.92), snap.scroll0);

  const H = {}; // handles found in the snapshots
  function setup() {
    /* hosts */
    {
      const doc = S.hosts.doc;
      const rows = all(one(doc, 'rows'), 'row').filter((r) => r.parentElement === one(doc, 'rows'));
      const selClass = [...rows[0].classList].find((c) => /^_rowSel_/.test(c));
      const names = rows.map((r) => one(r, 'name').textContent.trim());
      const count = one(doc, 'pageCount');
      H.hosts = {
        rows: rows.map((e, i) => ({ e, name: names[i], h: e.offsetHeight, match: names[i].includes('tokyo') })),
        selClass,
        input: doc.querySelector('input'),
        count,
        countText: count.textContent,
        prod: rows[names.indexOf('prod-api-tokyo')],
        first: rows[0],
      };
    }
    /* terminal, SFTP */
    H.tab = one(S.term.doc, 'tabActive');
    H.termRect = rectOf(S.term, one(S.term.doc, 'term'), W1);
    {
      const doc = S.sftp.doc;
      const panel = all(doc, 'panel').find((e) => one(e, 'pathBar'));
      H.sftp = { panel, w: panel.offsetWidth, rows: [...one(panel, 'list').children] };
      panel.style.flex = 'none';
      panel.style.overflow = 'hidden';
    }
    /* AI */
    const aiPanel = (doc) => all(doc, 'panel').find((e) => one(e, 'composer'));
    for (const [name] of [['ai-empty'], ...STAGES]) {
      const p = aiPanel(S[name].doc);
      Object.assign(p.style, { flex: 'none', overflow: 'hidden', minWidth: '0', maxWidth: 'none' });
      S[name].panel = p;
      S[name].list = one(p, 'list');
    }
    const blocksOf = (doc) => {
      const list = S[doc].list;
      const out = [];
      for (const item of list.children) {
        const a = one(item, 'assistant');
        if (a) out.push(...a.children);
        else out.push(item);
      }
      return out;
    };
    // When each block of the conversation first shows: blocks keep their position from one stage to
    // the next, and only the new ones at the end of a stage appear, each once the one before it has
    // finished streaming. A stage streams faster when it must, so that its approval card is up LEAD
    // before the click, and the summary has finished a second before the panel closes.
    H.appear = [];
    let seen = 0;
    let next = 0;
    STAGES.forEach(([name, t0], s) => {
      const snap = S[name];
      const blocks = blocksOf(name);
      snap.blocks = blocks;
      snap.first = seen;
      for (const md of blocks.filter((b) => /_md_/.test(b.className))) {
        if (md.words) continue;
        const walker = snap.doc.createTreeWalker(md, NodeFilter.SHOW_TEXT);
        const nodes = [];
        while (walker.nextNode()) nodes.push(walker.currentNode);
        md.words = [];
        for (const node of nodes) {
          const frag = snap.doc.createDocumentFragment();
          for (const w of node.textContent.match(UNIT) ?? []) {
            const span = snap.doc.createElement('span');
            span.className = 'pw';
            span.textContent = w;
            frag.appendChild(span);
            md.words.push(span);
          }
          node.replaceWith(frag);
        }
        // Code spans and tables show with their first word rather than as empty boxes ahead of it.
        md.boxes = [...md.querySelectorAll('code, pre, table, tr, th, td')].map((e) => {
          const k = md.words.findIndex((w) => e.compareDocumentPosition(w) & Node.DOCUMENT_POSITION_FOLLOWING);
          return { e, k: k < 0 ? md.words.length : k };
        });
      }
      const fresh = blocks.slice(seen);
      const timed = s < 4 ? fresh.slice(0, -1) : fresh;
      const deadline = s < 4 ? RUNS[s] - LEAD : T_AI_OUT - 1.0;
      const start = Math.max(t0 + (s ? 0.15 : 0), next);
      const gap = (i) => (i === 0 ? 0.3 : 0.25);
      let fixed = 0;
      let units = 0;
      timed.forEach((b, j) => {
        fixed += b.words ? 0.15 : gap(seen + j);
        units += b.words ? b.words.length : 0;
      });
      const room = deadline - start - fixed;
      if (room <= 0.05 && units) console.error(`promo: ${name} has no time to stream its text`);
      const rate = units ? Math.max(WORD_RATE, units / Math.max(room, 0.05)) : WORD_RATE;
      let when = start;
      fresh.forEach((b, j) => {
        const i = seen + j;
        H.appear[i] = when;
        b.rate = rate;
        when += b.words ? 0.15 + b.words.length / rate : gap(i);
      });
      next = when;
      seen = blocks.length;
    });
    // Each stage's list follows the conversation as the app's does: when a block appears, the list
    // scrolls until the whole block shows, or the top of a tall one. A stage starts where the last
    // one left off, and the card that was waiting for approval folds into its one-line summary.
    let from = 0;
    STAGES.forEach(([name], s) => {
      const snap = S[name];
      const list = snap.list;
      list.scrollTop = 0;
      const box = list.getBoundingClientRect();
      const k = box.height / list.offsetHeight; // the snapshot's zoom
      const pad = parseFloat(getComputedStyle(list).paddingBottom);
      const max = list.scrollHeight - list.clientHeight;
      let y = Math.min(from, max);
      snap.scroll0 = y;
      snap.moves = [];
      snap.blocks.slice(snap.first).forEach((b, j) => {
        const r = b.getBoundingClientRect();
        const want = clamp(Math.min((r.bottom - box.top) / k + pad - list.clientHeight, (r.top - box.top) / k - 14), 0, max);
        if (want > y + 0.5) snap.moves.push({ t: H.appear[snap.first + j], d: want - y });
        y = Math.max(y, want);
      });
      from = y;
      if (s) {
        const card = snap.blocks[snap.first - 1];
        snap.fold = { card, from: S[STAGES[s - 1][0]].blocks[snap.first - 1].offsetHeight, to: card.offsetHeight };
        Object.assign(card.style, { boxSizing: 'border-box', overflow: 'hidden', flexShrink: '0' });
      }
    });
    H.composer = one(S['ai-empty'].panel, 'composer');
    H.textarea = H.composer.querySelector('textarea');
    H.send = [...H.composer.querySelectorAll('button')].pop();
    H.runs = STAGES.slice(0, 4).map(([name]) => byText(S[name].panel, 'button', RUN_LABEL));
    const done = S['ai-done'];
    const fixed = [...done.list.querySelectorAll('strong, b')].find((e) => e.textContent.trim().startsWith(FIXED));
    done.list.scrollTop = scrollAt(done, T_DONE + 0.3);
    H.fixedAt = fixed ? at(done, fixed, W2) : { x: 1500, y: 700 };

    /* dashboard */
    {
      const doc = S.dash.doc;
      H.dash = all(doc, 'card').map((card) => ({
        card,
        meters: all(card, 'meterRow').map((row) => {
          const fill = one(row, 'meterFill');
          const value = one(row, 'meterValue');
          return { fill, value, w: fill ? parseFloat(fill.style.width) || 0 : 0, text: value ? value.textContent : '' };
        }),
      }));
    }

    /* cursor */
    const c = cursor;
    const search = at(S.hosts, H.hosts.input, W1, 0.3);
    const prod = at(S.hosts, one(H.hosts.prod, 'name'), W1, 0.6);
    c.show(16.2, T_TERM - 0.05);
    c.at(16.2, 1330, 930);
    c.to(16.3, search.x, search.y, 0.6);
    c.click(16.95);
    c.to(19.15, prod.x, prod.y, 0.6);
    c.click(19.95);
    c.click(20.1);

    const sftpBtn = at(S.term, one(S.term.doc, 'sftp'), W1);
    const aiBtn = at(S.term, one(S.term.doc, 'aiButton'), W1);
    const ta = at(S['ai-empty'], H.textarea, W2, 0.3);
    const send = at(S['ai-empty'], H.send, W2);
    c.show(22.9, T_SEND + 0.2);
    c.at(22.9, 1100, 900);
    c.to(23.0, sftpBtn.x, sftpBtn.y, 0.45);
    c.click(T_SFTP - 0.05);
    c.to(24.2, sftpBtn.x - 40, sftpBtn.y + 260, 0.6);
    c.to(25.0, sftpBtn.x, sftpBtn.y, 0.5);
    c.click(T_SFTP_OUT - 0.05);
    c.to(27.75, aiBtn.x, aiBtn.y, 0.6);
    c.click(28.5);
    c.to(28.9, ta.x, ta.y, 0.5);
    c.click(29.4);
    c.to(T_SEND - 0.6, send.x, send.y, 0.4);
    c.click(T_SEND - 0.05);
    c.show(RUNS[0] - 0.6, RUNS[3] + 0.5);
    H.runs.forEach((b, k) => {
      if (!b) return;
      // Measured where the button sits when it's clicked, with the list scrolled as it is then.
      const snap = S[STAGES[k][0]];
      snap.list.scrollTop = scrollAt(snap, RUNS[k]);
      const p = at(snap, b, W2);
      if (k === 0) c.at(RUNS[0] - 0.6, p.x - 120, p.y + 160);
      c.to(RUNS[k] - 0.5, p.x, p.y, 0.4);
      c.click(RUNS[k]);
    });
    burst(T_DONE + 0.15, H.fixedAt.x, H.fixedAt.y, 26, 3);
  }

  // A docked panel at width w of its full width: its contents keep their size and slide in from the edge.
  // (The panels are flex columns, so the children's heights stay as the app lays them out.)
  function slide(panel, w, full) {
    for (const c of panel.children) {
      c.style.width = `${full}px`;
      c.style.transform = `translateX(${n2(w - full)}px)`;
    }
  }

  /* timing of each snapshot */
  function opacityOf(name, t) {
    const fadeIn = (t0, d = 0.25) => prog(t, t0, d);
    switch (name) {
      case 'hosts':
        return t >= T_MORPH && t < T_TERM + 0.6 ? 1 : 0;
      case 'term':
        return (t >= T_TERM + 0.15 && t < T_SFTP) || (t >= T_SFTP_OUT + 0.5 && t < T_AI) ? fadeIn(T_TERM + 0.15, 0.18) : 0;
      case 'sftp':
        return t >= T_SFTP && t < T_SFTP_OUT + 0.5 ? 1 : 0;
      case 'ai-empty':
        return t >= T_AI && t < T_SEND ? 1 : 0;
      case 'dash':
        return t >= T_DASH ? fadeIn(T_DASH, 0.15) : 0;
      default: {
        const k = STAGES.findIndex((s) => s[0] === name);
        const end = k + 1 < STAGES.length ? STAGES[k + 1][1] : T_DASH + 0.3;
        return t >= STAGES[k][1] && t < end ? 1 : 0;
      }
    }
  }

  cue(T_MORPH, 'whoosh', { gain: -6, dur: 1.0 });
  for (let i = 0; i < 11; i++) cue(14.3 + i * 0.045, 'pop', { gain: -18, pitch: [0, 2, 4, 7, 9, 12, 14, 16, 19, 21, 24][i] });
  for (let k = 0; k < 5; k++) cue(17.15 + k * 0.13, 'key', { gain: -13, pitch: (rnd(300 + k) - 0.5) * 4 });
  cue(T_FILTER, 'swish', { gain: -11 });
  cue(T_TERM, 'whoosh', { gain: -7, dur: 0.9 });
  cue(T_SFTP, 'swish', { gain: -9 });
  cue(T_SFTP_OUT, 'swish', { gain: -11 });
  cue(27.35, 'boop', { gain: -5 });
  cue(T_AI, 'whoosh', { gain: -7, dur: 0.8 });
  cue(T_SEND, 'send', { gain: -5 });
  RUNS.forEach((r, k) => cue(r + 0.35, 'tick', { gain: -8, pitch: k * 2 }));
  cue(T_OK, 'ding', { gain: -8, pitch: 7 });
  cue(T_DONE + 0.15, 'success', { gain: -3 });
  cue(T_DONE + 0.18, 'sparkle', { gain: -7 });
  cue(T_AI_OUT, 'swish', { gain: -9 });
  cue(T_DASH, 'whoosh', { gain: -9, dur: 0.8 });
  for (let i = 0; i < 6; i++) cue(T_DASH + 0.3 + i * 0.06, 'pop', { gain: -17, pitch: [0, 2, 4, 7, 9, 12][i] });
  const promptChars = [...C.prompt];
  const T_TYPE = 29.5;
  const promptCps = promptChars.length / 0.8;
  promptChars.forEach((_, k) => cue(T_TYPE + k / promptCps, 'key', { gain: -16 + rnd(900 + k) * 4, pitch: (rnd(950 + k) - 0.5) * 4 }));

  onFrame((t) => {
    // A gentle push-in while the assistant works, held at the window's bottom edge so the whole window
    // stays in frame; the cursor and effects ride along.
    const zoom = 1 + 0.04 * (spring(t - T_SEND, 0.7, 0.9) - spring(t - 40.4, 0.7, 0.9));
    const oy = W2.y + W2.h;
    const cam = Math.abs(zoom - 1) < 1e-4 ? '' : `translate(960px,${oy}px) scale(${zoom.toFixed(5)}) translate(-960px,-${oy}px)`;
    for (const l of [L.app, L.fx, L.cursor]) {
      l.style.transformOrigin = '0 0';
      l.style.transform = cam;
    }
    if (!shown(L.app, t >= T_MORPH && t < T_WIPE + 0.72) || !S.dash) return;

    const r = winRect(t);
    win.style.cssText = `left:${n2(r.x)}px;top:${n2(r.y)}px;width:${n2(r.w)}px;height:${n2(r.h)}px;border-radius:${n2(r.r)}px`;
    const morphing = t < T_MORPH + 1.6;
    const iconO = 1 - prog(t, T_MORPH + 0.02, 0.24);
    if (shown(icon, iconO > 0)) {
      const s = (Math.max(r.w, r.h) * 1024) / 896;
      icon.style.cssText = `width:${n2(s)}px;height:${n2(s)}px;margin:${n2(-s / 2)}px 0 0 ${n2(-s / 2)}px;opacity:${iconO.toFixed(3)}`;
    }

    for (const name of SNAPS) {
      const o = opacityOf(name, t);
      const snap = S[name];
      if (!shown(snap.f, o > 0)) continue;
      snap.f.style.opacity = o.toFixed(3);
      // While the window grows out of the icon, the app sits where it will end up and the window reveals it.
      const w = morphing ? W1.w : r.w;
      snap.f.style.width = `${n2(w)}px`;
      snap.f.style.height = `${W1.h}px`;
      snap.f.style.transform = morphing ? `translate(${n2(W1.x - r.x)}px,${n2(W1.y - r.y)}px)` : '';
      if (snap.term) drawTerm(snap.term, t);
    }

    // hosts: rows fall into place, the search filters them, and a double-click opens one
    if (S.hosts.f.style.display !== 'none') {
      const h = H.hosts;
      const n = t < 17.15 ? 0 : Math.min(5, Math.floor((t - 17.15) / 0.13) + 1);
      h.input.value = 'tokyo'.slice(0, n);
      const focus = t >= 16.95;
      if (focus && S.hosts.doc.activeElement !== h.input) h.input.focus();
      if (!focus && S.hosts.doc.activeElement === h.input) h.input.blur();
      text(h.count, t < T_FILTER ? h.countText : h.countText.replace(/\d+/, '3'));
      const fOut = ease.inOut(prog(t, T_FILTER, 0.4));
      h.rows.forEach((row, i) => {
        const a = spring(t - (14.3 + i * 0.045), 2.0, 0.65);
        let o = clamp(a * 1.6);
        let height = row.h;
        if (!row.match) {
          o *= 1 - fOut;
          height = row.h * (1 - fOut);
        }
        row.e.style.opacity = o.toFixed(3);
        row.e.style.transform = `translateY(${n2((1 - a) * 18)}px)`;
        row.e.style.height = `${n2(height)}px`;
        row.e.style.minHeight = '0';
        row.e.style.overflow = 'hidden';
      });
      const picked = t >= T_PICK;
      h.first.classList.toggle(h.selClass, !picked);
      h.prod.classList.toggle(h.selClass, picked);
    }
    if (shown(expander, t >= T_TERM && t < T_TERM + 0.6)) {
      const e = spring(t - T_TERM, 1.3, 0.85);
      const from = rectOf(S.hosts, H.hosts.prod, W1);
      const q = lerpRect({ ...from, r: 6 }, { ...H.termRect, r: 0 }, e);
      expander.style.cssText = `left:${n2(q.x - r.x)}px;top:${n2(q.y - r.y)}px;width:${n2(q.w)}px;height:${n2(q.h)}px;border-radius:${n2(q.r)}px;opacity:${(1 - prog(t, T_TERM + 0.3, 0.3)).toFixed(3)}`;
    }
    if (H.tab) {
      const a = spring(t - (T_TERM + 0.15), 2.2, 0.55);
      H.tab.style.transform = `scale(${(0.6 + 0.4 * a).toFixed(4)})`;
      H.tab.style.opacity = clamp(a * 2).toFixed(3);
    }

    // SFTP: the panel slides open beside the terminal and closes again
    if (S.sftp.f.style.display !== 'none') {
      const o = clamp(spring(t - T_SFTP, 1.4, 0.85) - spring(t - T_SFTP_OUT, 1.4, 0.95));
      H.sftp.panel.style.width = `${n2(H.sftp.w * o)}px`;
      slide(H.sftp.panel, H.sftp.w * o, H.sftp.w);
      H.sftp.rows.forEach((row, i) => {
        const a = spring(t - (T_SFTP + 0.15 + i * 0.03), 2.2, 0.7);
        row.style.opacity = clamp(a * 1.6).toFixed(3);
        row.style.transform = `translateX(${n2((1 - a) * 16)}px)`;
      });
    }

    // AI: the panel opens, the prompt is typed and sent, and the conversation plays out stage by stage
    const aw = 440 * clamp(aiOpen(t), 0, 1.04);
    for (const [name] of [['ai-empty'], ...STAGES]) {
      const snap = S[name];
      if (snap.f.style.display === 'none') continue;
      snap.panel.style.width = `${n2(aw)}px`;
      slide(snap.panel, aw, 440);
    }
    if (S['ai-empty'].f.style.display !== 'none') {
      const n = t < T_TYPE ? 0 : Math.min(promptChars.length, Math.floor((t - T_TYPE) * promptCps) + 1);
      H.textarea.value = promptChars.slice(0, n).join('');
      H.send.style.transform = `scale(${(1 - 0.15 * bump(t, T_SEND - 0.1, 0.2)).toFixed(4)})`;
    }
    STAGES.forEach(([name, t0], s) => {
      const snap = S[name];
      if (snap.f.style.display === 'none') return;
      if (snap.fold) snap.fold.card.style.height = `${n2(lerp(snap.fold.from, snap.fold.to, spring(t - t0, 2.4, 0.9)))}px`;
      snap.list.scrollTop = scrollAt(snap, t);
      snap.blocks.forEach((b, i) => {
        const t1 = H.appear[i];
        const a = spring(t - t1, 2.0, 0.72);
        b.style.opacity = clamp(a * 1.8).toFixed(3);
        b.style.transform = i === 0 && s === 0 ? `translateY(${n2((1 - spring(t - t1, 1.4, 0.8)) * 260)}px)` : `translateY(${n2((1 - a) * 14)}px)`;
        if (b.words) {
          const shows = (k) => prog(t, t1 + 0.1 + k / b.rate, 0.08).toFixed(3);
          b.words.forEach((w, k) => (w.style.opacity = shows(k)));
          b.boxes.forEach((x) => (x.e.style.opacity = shows(x.k)));
        }
      });
      const run = H.runs[s];
      if (run && s < 4) {
        run.style.transform = `scale(${(1 - 0.12 * bump(t, RUNS[s] - 0.04, 0.2)).toFixed(4)})`;
        // Approved: the card's buttons settle back while the command runs.
        run.parentElement.style.opacity = (1 - 0.6 * prog(t, RUNS[s] + 0.12, 0.25)).toFixed(3);
      }
    });

    // dashboard
    if (S.dash.f.style.display !== 'none') {
      H.dash.forEach((d, i) => {
        const a = spring(t - (T_DASH + 0.25 + i * 0.05), 1.9, 0.65);
        d.card.style.opacity = clamp(a * 1.6).toFixed(3);
        d.card.style.transform = `translateY(${n2((1 - a) * 26)}px) scale(${(0.94 + 0.06 * a).toFixed(4)})`;
        d.meters.forEach((m, k) => {
          if (!m.fill) return;
          const f = spring(t - (T_DASH + 0.5 + i * 0.05 + k * 0.05), 1.3, 0.75);
          m.fill.style.width = `${(m.w * f).toFixed(3)}%`;
          if (m.value && /\d/.test(m.text)) text(m.value, m.text.replace(/\d+/, String(Math.round(parseInt(m.text.match(/\d+/)[0], 10) * clamp(f)))));
        });
      });
    }
  });

  return { load };
})();

caption(C.hosts, 14.1, 20.2, 'headline', 82);
caption(C.hostsSub, 15.0, 20.15, 'sub', 168);
caption(C.term, 21.0, 27.6, 'headline', 82);
caption(C.termSub, 21.8, 27.55, 'sub', 168);
caption(C.ai, 29.0, 41.0, 'headline', 82);
caption(C.aiSub1, 33.2, T_DONE - 0.45, 'sub', 168);
caption(C.aiSub2, T_DONE - 0.15, 40.95, 'sub', 168);
caption(C.dash, 42.0, 47.1, 'headline', 82);
caption(C.dashSub, 42.8, 47.05, 'sub', 168);

/* ---------- wave wipe into the night ---------- */

{
  const BH = 1500;
  const svg = sv(L.wipe, 'svg', { width: W, height: H, viewBox: `0 0 ${W} ${H}` });
  svg.innerHTML = `<defs>
    <linearGradient id="g-band" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#22a6e8"/><stop offset=".45" stop-color="#1670c4"/><stop offset="1" stop-color="#0d1735"/></linearGradient>
    <g id="rings2" stroke="#ffffff" stroke-width="5"><circle r="56" fill="#000000"/><circle r="40.3" fill="none"/><circle r="24.6" fill="none"/><circle r="9" fill="none"/></g>
    <pattern id="p-band" width="112" height="56" patternUnits="userSpaceOnUse">
      <use href="#rings2" x="56" y="-28"/><use href="#rings2" x="0" y="0"/><use href="#rings2" x="112" y="0"/><use href="#rings2" x="56" y="28"/><use href="#rings2" x="0" y="56"/><use href="#rings2" x="112" y="56"/><use href="#rings2" x="56" y="84"/>
    </pattern>
    <mask id="m-band" maskUnits="userSpaceOnUse" x="0" y="-3000" width="1920" height="7000"><rect x="0" y="-3000" width="1920" height="7000" fill="url(#p-band)"/></mask>
    <clipPath id="c-band"><path id="band-clip"/></clipPath>
  </defs>`;
  const pat = svg.querySelector('#p-band');
  const clip = svg.querySelector('#band-clip');
  const band = sv(svg, 'path', { fill: 'url(#g-band)' });
  const inner = sv(svg, 'g', { 'clip-path': 'url(#c-band)' });
  sv(inner, 'rect', { x: 0, y: -3000, width: 1920, height: 7000, fill: '#ffffff', 'fill-opacity': 0.08, mask: 'url(#m-band)' });
  const crest = sv(inner, 'path', { fill: 'none', stroke: '#b5ecff', 'stroke-opacity': 0.75, 'stroke-width': 18 });
  const bubbles = [];
  for (let i = 0; i < 18; i++) {
    bubbles.push({ e: sv(inner, 'circle', { r: (6 + rnd(500 + i) * 12).toFixed(1), fill: 'none', stroke: '#ffffff', 'stroke-opacity': 0.45, 'stroke-width': 3 }), x: rnd(600 + i) * 1920, y: 120 + rnd(700 + i) * 1100, v: 120 + rnd(800 + i) * 260 });
  }
  cue(T_WIPE - 0.1, 'wipe', { gain: -3 });
  onFrame((t) => {
    const p = prog(t, T_WIPE, 1.4);
    if (!shown(L.wipe, p > 0 && p < 1)) return;
    const yT = lerp(1110, -BH - 90, ease.inOut(p));
    const ph = t * 260;
    let topD = '';
    const bottom = [];
    for (let x = -20; x <= 1940; x += 12) {
      topD += `${x === -20 ? 'M' : 'L'}${x} ${n2(yT - 24 * Math.cos((2 * Math.PI * (x + ph)) / 448))}`;
      bottom.push(`L${x} ${n2(yT + BH + 24 * Math.cos((2 * Math.PI * (x - ph)) / 448))}`);
    }
    const d = `${topD}${bottom.reverse().join('')}Z`;
    band.setAttribute('d', d);
    clip.setAttribute('d', d);
    crest.setAttribute('d', topD);
    pat.setAttribute('patternTransform', `translate(${n2(ph * 0.2)} ${n2(yT)})`);
    for (const b of bubbles) {
      b.e.setAttribute('cx', n2(b.x + Math.sin(t * 3 + b.y) * 10));
      b.e.setAttribute('cy', n2(yT + b.y - (t - T_WIPE) * b.v));
    }
  });
}

/* ---------- sync through your own Cloudflare account ---------- */

{
  const T_SYNC = 48.2;
  const laptop = el(L.sync, 'device', '<div class="screen" style="left:-230px;top:-150px;width:460px;height:300px"><div class="screen-in"><div class="strip"><i></i><i></i><i></i></div></div></div><div class="base" style="left:-275px;top:150px;width:550px;height:28px"></div>');
  const monitor = el(L.sync, 'device', '<div class="stand" style="left:-26px;top:150px;width:52px;height:52px"></div><div class="stand" style="left:-92px;top:196px;width:184px;height:22px;border-radius:11px"></div><div class="screen" style="left:-230px;top:-150px;width:460px;height:300px"><div class="screen-in"><div class="strip"><i></i><i></i><i></i></div></div></div>');
  const cloud = el(
    L.sync,
    'device',
    `<svg width="440" height="300" viewBox="-220 -170 440 300" style="position:absolute;left:-220px;top:-170px;overflow:visible;filter:drop-shadow(0 0 46px rgba(120,190,255,.45))">
      <defs><linearGradient id="g-cloud" x1="0" y1="-150" x2="0" y2="110" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="#ffffff"/><stop offset="1" stop-color="#cddfff"/></linearGradient></defs>
      <g fill="url(#g-cloud)"><circle cx="-120" cy="30" r="72"/><circle cx="-50" cy="-30" r="100"/><circle cx="55" cy="-50" r="92"/><circle cx="130" cy="20" r="76"/><rect x="-190" y="10" width="380" height="92" rx="46"/></g>
      <ellipse cx="-74" cy="54" rx="15" ry="8" fill="#ff8fb1" fill-opacity=".6"/><ellipse cx="74" cy="54" rx="15" ry="8" fill="#ff8fb1" fill-opacity=".6"/>
      <g class="eyes"><ellipse cx="-38" cy="30" rx="9" ry="11.5" fill="#0d1735"/><ellipse cx="38" cy="30" rx="9" ry="11.5" fill="#0d1735"/><circle cx="-34.5" cy="25" r="3.2" fill="#fff"/><circle cx="41.5" cy="25" r="3.2" fill="#fff"/></g>
      <g class="happy" fill="none" stroke="#0d1735" stroke-width="6" stroke-linecap="round"><path d="M-50 34Q-38 18 -26 34"/><path d="M26 34Q38 18 50 34"/></g>
      <path d="M-13 56Q0 68 13 56" fill="none" stroke="#0d1735" stroke-width="5" stroke-linecap="round"/>
    </svg>`,
  );
  const cloudEyes = cloud.querySelector('.eyes');
  const cloudHappy = cloud.querySelector('.happy');
  const label = el(L.sync, 'cloud-label', `<i class="ph-fill ph-cloud"></i>${C.cloud}<b>Worker</b><b>D1</b>`);
  let labelW = 400;
  onMeasure(() => (labelW = label.offsetWidth));
  for (const d of [laptop, monitor, cloud]) d.style.transformOrigin = '0 0';

  const CHIPS = [
    ['ph-hard-drives', 'prod-api-tokyo', '9f2c·a71e·04bd'],
    ['ph-key', 'id_ed25519', 'e83a·5c10·7fd2'],
    ['ph-chat-circle-dots', C.chat, '1b6e·d94f·a0c7'],
  ].map(([icon, name, cipher], i) => {
    const home = el(L.sync, 'chip', `<span class="ico"><i class="ph ${icon}"></i></span><span class="txt">${name}</span>`);
    const e = el(L.sync, 'chip', `<div class="lockbg"></div><span class="ico"><i class="ph ${icon} it"></i><i class="ph-fill ph-lock-simple-open uk"></i><i class="ph-fill ph-lock-simple lk"></i></span><span class="txt">${name}</span><span class="cipher"></span>`);
    const T = {
      appear: 49.2 + i * 0.08,
      lift: 49.7 + i * 0.12,
      open: 50.1 + i * 0.15,
      lock: 50.3 + i * 0.15,
      go: 50.9 + i * 0.18,
      back: 53.6 + i * 0.18,
      unlock: 54.6 + i * 0.15,
      home: 54.95 + i * 0.15,
      drop: 55.1 + i * 0.12,
    };
    cue(T.lock, 'lock', { gain: -6, pitch: i * 2 });
    cue(T.go + 0.7, 'bubble', { gain: -10, pitch: 7 + i * 2 });
    cue(T.unlock, 'unlock', { gain: -6, pitch: i * 2 });
    cue(T.drop, 'pop', { gain: -10, pitch: 7 + i * 3 });
    twinkle(T.drop + 0.15, 1511 + 170, 590 + i * 68 - 20, 0.8, '#ffd166');
    return {
      e,
      home,
      i,
      T,
      name,
      cipher,
      bg: e.querySelector('.lockbg'),
      it: e.querySelector('.it'),
      uk: e.querySelector('.uk'),
      lk: e.querySelector('.lk'),
      txt: e.querySelector('.txt'),
      ci: e.querySelector('.cipher'),
    };
  });
  const CLOUD = { x: 960, y: 362 };
  const HEX = '0123456789abcdef';
  const scramble = (len, seed) => Array.from({ length: len }, (_, k) => (k % 5 === 4 ? '·' : HEX[Math.floor(rnd(seed + k) * 16)])).join('');

  cue(48.5, 'pop', { gain: -6 });
  cue(48.8, 'pop', { gain: -8, pitch: 4 });
  cue(49.0, 'pop', { gain: -8, pitch: 7 });
  cue(49.7, 'swish', { gain: -10 });
  cue(52.0, 'sparkle', { gain: -12 });
  cue(53.6, 'swish', { gain: -10 });
  cue(58.3, 'swish', { gain: -9 });

  onFrame((t) => {
    if (!shown(L.sync, t >= T_SYNC && t < 59.2)) return;
    const pop = (t0, out) => {
      const a = spring(t - t0, 1.6, 0.55);
      const b = ease.in(prog(t, out, 0.35));
      return { s: (0.3 + 0.7 * a) * (1 - 0.6 * b), o: clamp(a * 2) * (1 - b) };
    };
    let p = pop(48.5, 58.4);
    let sq = 0;
    for (const c of CHIPS) sq += bump(t, c.T.go + 0.72, 0.3) + 0.5 * bump(t, c.T.back, 0.25);
    const cy = CLOUD.y + Math.sin(t * 1.6) * 7;
    cloud.style.transform = `translate(${CLOUD.x}px,${n2(cy)}px) scale(${n2(p.s * (1 + 0.07 * sq))},${n2(p.s * (1 - 0.07 * sq))})`;
    cloud.style.opacity = p.o.toFixed(3);
    const happy = t >= 51.85 && t < 53.5;
    cloudHappy.style.display = happy ? '' : 'none';
    cloudEyes.style.display = happy ? 'none' : '';
    const open = (t + 0.4) % 3.1 < 0.15 ? 0.15 : 1;
    cloudEyes.setAttribute('transform', `translate(0 30) scale(1 ${open}) translate(0 -30)`);
    const lp = pop(48.9, 58.45);
    label.style.transform = `translate(${n2(960 - labelW / 2)}px,${n2(510 + (1 - lp.s) * 20)}px)`;
    label.style.opacity = lp.o.toFixed(3);
    p = pop(48.8, 58.3);
    put(laptop, 440, 690 + Math.sin(t * 1.3) * 4, p.s, p.o);
    p = pop(49.0, 58.35);
    put(monitor, 1480, 680 + Math.sin(t * 1.3 + 1) * 4, p.s, p.o);

    for (const c of CHIPS) {
      const T = c.T;
      const p0 = { x: 471, y: 600 + c.i * 68 };
      const ha = spring(t - T.appear, 2, 0.7);
      const hout = ease.in(prog(t, 58.3 + c.i * 0.05, 0.3));
      if (shown(c.home, t >= T.appear)) put(c.home, p0.x, p0.y, (0.7 + 0.3 * ha) * (1 - 0.5 * hout), clamp(ha * 2) * (1 - hout));
      if (!shown(c.e, t >= T.lift)) continue;
      const p1 = { x: 560, y: 290 + c.i * 76 };
      const p3 = { x: 1360, y: 290 + c.i * 76 };
      const p4 = { x: 1511, y: 590 + c.i * 68 };
      let s = 1;
      let o = 1;
      const l = spring(t - T.lift, 1.4, 0.75);
      let q = { x: lerp(p0.x, p1.x, l), y: lerp(p0.y, p1.y, l) };
      const u1 = ease.inOut(prog(t, T.go, 0.75));
      if (u1 > 0) {
        q = bez(p1, { x: 760, y: 140 }, CLOUD, u1);
        s = lerp(1, 0.4, u1);
        o = 1 - clamp((u1 - 0.75) / 0.25);
      }
      const u2 = ease.inOut(prog(t, T.back, 0.75));
      if (u2 > 0) {
        q = bez(CLOUD, { x: 1160, y: 140 }, p3, u2);
        s = lerp(0.4, 1, u2);
        o = clamp(u2 / 0.25);
      }
      if (t >= T.drop) {
        const d = spring(t - T.drop, 1.6, 0.7);
        q = { x: lerp(p3.x, p4.x, d), y: lerp(p3.y, p4.y, d) };
      }
      const out = ease.in(prog(t, 58.3 + c.i * 0.05, 0.3));
      put(c.e, q.x, q.y, s * (1 - 0.5 * out), o * (1 - out));

      const locked = prog(t, T.lock, 0.18) * (1 - prog(t, T.unlock, 0.18));
      c.bg.style.opacity = locked.toFixed(3);
      const ic = t < T.open || t >= T.home ? 'it' : t < T.lock || t >= T.unlock ? 'uk' : 'lk';
      const since = ic === 'it' ? Math.max(0, t - T.home) : ic === 'uk' ? (t < T.lock ? t - T.open : t - T.unlock) : t - T.lock;
      for (const k of ['it', 'uk', 'lk']) {
        c[k].style.opacity = k === ic ? '1' : '0';
        if (k === ic) c[k].style.transform = `scale(${(t < T.open ? 1 : spring(since, 2.4, 0.45)).toFixed(4)})`;
      }
      const cipherOn = t >= T.lock && t < T.unlock + 0.35;
      shown(c.ci, cipherOn);
      c.txt.style.opacity = cipherOn ? '0' : '1';
      if (cipherOn) {
        const settling = t < T.lock + 0.35 || t >= T.unlock;
        text(c.ci, settling ? scramble(14, Math.floor(t * 30) * 7 + c.i * 1000) : c.cipher);
      }
    }
  });
  caption(C.sync, 48.6, 58.2, 'headline night', 66);
  caption(C.syncSub1, 51.6, 55.15, 'sub night', 152);
  caption(C.syncSub2, 55.5, 58.15, 'sub night', 152);
}

/* ---------- end card ---------- */

{
  const icon = el(L.end, 'end-icon', `<img src="${ICON_SRC}" alt="">`);
  const mark = new Words(L.end, 'Hatoba', 'wordmark end-mark', true);
  mark.root.style.top = '452px';
  const tag = new Words(L.end, C.tagline, 'words tagline end-tag');
  tag.root.style.top = '598px';
  const rows = [
    [
      ['ph-fill ph-windows-logo', 'Windows'],
      ['ph-fill ph-linux-logo', 'Linux'],
      ['ph-fill ph-apple-logo', 'macOS'],
    ],
    [
      ['ph-fill ph-github-logo', C.openSource],
      ['ph-bold ph-globe-simple', 'hatoba.fog.moe', 'url'],
    ],
  ];
  const pills = [];
  rows.forEach((row, r) => {
    const box = el(L.end, 'pills');
    box.style.top = `${672 + r * 72}px`;
    for (const [ic, label, cls] of row) pills.push(el(box, `epill ${cls ?? ''}`, `<i class="${ic}"></i>${label}`));
  });
  cue(62.0, 'chime', { gain: -2, pitch: 5 });
  cue(59.2, 'waves', { gain: -12, dur: 11 });
  [0, 2, 4, 7, 9, 12].forEach((p, i) => cue(62.35 + i * 0.05, 'pop', { gain: -15, pitch: p + 5 }));
  pills.forEach((_, i) => cue(63.25 + i * 0.08, 'pop', { gain: -14, pitch: [7, 9, 12, 14, 16][i] }));
  for (const [t0, x, y, s] of [
    [62.7, 1110, 175, 1],
    [63.6, 800, 360, 0.7],
    [64.4, 1150, 400, 0.8],
    [65.5, 790, 190, 0.9],
    [66.3, 1140, 260, 0.6],
    [67.1, 820, 300, 0.8],
  ])
    twinkle(t0, x, y, s);
  onFrame((t) => {
    if (!shown(L.end, t >= 61.8)) return;
    const a = spring(t - 62.0, 1.5, 0.5);
    put(icon, 960, 290 + (1 - a) * 60 + Math.sin(t * 1.5) * 6 * prog(t, 62.7, 1), 0.2 + 0.8 * a, clamp(a * 2), (1 - a) * -25);
    mark.update(t, 62.35, null, { gap: 0.05, f: 2.0, z: 0.5, rise: 60, spin: -12, blur: 6 });
    tag.update(t, 62.85, null, { gap: 0.03 });
    pills.forEach((p, i) => {
      const b = spring(t - (63.25 + i * 0.08), 2, 0.55);
      p.style.opacity = clamp(b * 2).toFixed(3);
      p.style.transform = `translateY(${n2((1 - b) * 30)}px) scale(${(0.6 + 0.4 * b).toFixed(4)})`;
    });
  });
}

/* ---------- playback ---------- */

function seek(realT) {
  const t = realT * RATE;
  for (const fn of frameFns) fn(t);
}

async function init() {
  await Promise.all(
    [
      '700 100px "Archivo Variable"',
      '20px Phosphor',
      '20px "Phosphor-Fill"',
      '20px "Phosphor-Bold"',
      '20px "Cascadia Mono"',
      '20px "Segoe UI Variable Text"',
      '700 20px "Microsoft YaHei UI"',
    ].map((f) => document.fonts.load(f).catch(() => null)),
  );
  await document.fonts.ready;
  await Promise.all([...stage.querySelectorAll('img')].map((img) => (img.complete ? null : new Promise((r) => (img.onload = img.onerror = r)))));
  for (const fn of measures) fn();
  await app.load();
  seek(0);
  window.PROMO = {
    ready: true,
    duration: END / RATE,
    cues: CUES.map((c) => ({ ...c, t: c.t / RATE })).sort((a, b) => a.t - b.t),
    seek,
  };
  if (!RENDER) controls();
}

function controls() {
  const bar = document.getElementById('controls');
  const play = document.getElementById('play');
  const scrub = document.getElementById('scrub');
  const clock = document.getElementById('clock');
  const lang = document.getElementById('lang');
  bar.hidden = false;
  const fit = () => {
    const s = Math.min(innerWidth / W, (innerHeight - 52) / H);
    stage.style.transform = `scale(${s})`;
    stage.style.left = `${(innerWidth - W * s) / 2}px`;
    stage.style.top = `${(innerHeight - 52 - H * s) / 2}px`;
  };
  addEventListener('resize', fit);
  fit();
  const dur = window.PROMO.duration;
  scrub.max = dur;
  let t = Number(Q.get('t')) || 0;
  let playing = false;
  let last = 0;
  const show = () => {
    seek(t);
    scrub.value = t;
    clock.textContent = t.toFixed(2);
  };
  const loop = (now) => {
    if (!playing) return;
    t = Math.min(dur, t + (now - last) / 1000);
    last = now;
    if (t >= dur) toggle();
    show();
    requestAnimationFrame(loop);
  };
  const toggle = () => {
    playing = !playing;
    play.textContent = playing ? 'Pause' : 'Play';
    if (playing) {
      if (t >= dur) t = 0;
      last = performance.now();
      requestAnimationFrame(loop);
    }
  };
  play.onclick = toggle;
  scrub.oninput = () => {
    t = Number(scrub.value);
    show();
  };
  addEventListener('keydown', (e) => {
    if (e.key === ' ') toggle();
    else if (e.key === 'ArrowRight' || e.key === 'ArrowLeft') {
      t = clamp(t + (e.key === 'ArrowRight' ? 1 : -1) * (e.shiftKey ? 1 : 1 / 60), 0, dur);
      show();
    } else return;
    e.preventDefault();
  });
  lang.textContent = LANG === 'zh' ? 'English' : '中文';
  lang.onclick = (e) => {
    e.preventDefault();
    const q = new URLSearchParams(location.search);
    q.set('lang', LANG === 'zh' ? 'en' : 'zh');
    q.set('t', t.toFixed(2));
    location.search = q.toString();
  };
  show();
  if (Q.has('play')) toggle();
}

// render.py waits for PROMO.ready, or stops on PROMO.error.
init().catch((e) => {
  window.PROMO = { error: e.message };
  console.error(e);
});
