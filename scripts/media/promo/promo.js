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
const END = 64;
const ICON_SRC = '../../../apps/desktop/src-tauri/app-icon.svg';

const COPY = {
  en: {
    sea1: 'Your servers are everywhere.',
    sea2: 'Give them a harbor.',
    tagline: 'Open-source SSH client with AI and encrypted sync',
    hosts: 'All your hosts in one place.',
    hostsSub: 'Import from ~/.ssh/config, PuTTY, Xshell, MobaXterm, and more.',
    search: 'Search hosts, or type ssh user@host',
    dashboard: 'Dashboard',
    allHosts: 'All Hosts',
    favorites: 'Favorites',
    recent: 'Recent',
    groups: 'Groups',
    tags: 'Tags',
    synced: 'Synced',
    syncedVia: '2 min ago · Cloudflare D1',
    term: 'Terminal and files, side by side.',
    termSub: 'Tabs, truecolor, CJK input, SFTP, and port forwarding.',
    connected: 'Connected',
    memory: 'Memory',
    ai: 'An AI assistant beside every terminal.',
    aiSub1: 'It reads the screen, runs commands, and edits files.',
    aiSub2: 'Found, fixed, and verified.',
    prompt: 'nginx keeps returning 502. Find out why and fix it.',
    placeholder: 'Ask about this terminal, or type / for commands',
    newChat: 'New chat',
    chatTitle: 'nginx 502 on prod-api-tokyo',
    reasoning: 'Reasoning',
    runCommand: 'Run command',
    editFile: 'Edit file',
    typeInTerminal: 'Type in terminal',
    run: 'Run',
    exit0: 'Exit 0',
    done: 'Done',
    found: ['Found it: nginx forwards to ', '127.0.0.1:8081', ', but the API listens on ', '8080', '.'],
    fixed: 'Fixed.',
    fixedRest: ' Requests get through again.',
    dash: 'Every server at a glance.',
    dashSub: 'Read over SSH, with nothing to install on the servers.',
    hostsCount: '11 hosts',
    cards: 'Cards',
    table: 'Table',
    disk: 'Disk',
    up: 'Up',
    days: 'days',
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
    search: '搜索主机，或输入 ssh user@host',
    dashboard: '仪表盘',
    allHosts: '所有主机',
    favorites: '收藏',
    recent: '最近',
    groups: '分组',
    tags: '标签',
    synced: '已同步',
    syncedVia: '2 分钟前 · Cloudflare D1',
    term: '终端和文件，并排在手边。',
    termSub: '多标签、真彩色、中日韩输入法、SFTP 和端口转发。',
    connected: '已连接',
    memory: '内存',
    ai: '每个终端旁，都有 AI 助手。',
    aiSub1: '它会读取屏幕、运行命令、编辑文件。',
    aiSub2: '找到原因，修好，再验证一遍。',
    prompt: 'nginx 一直返回 502，找出原因并修好它。',
    placeholder: '问问这个终端，或输入 / 使用命令',
    newChat: '新对话',
    chatTitle: 'prod-api-tokyo 上的 nginx 502',
    reasoning: '思考过程',
    runCommand: '运行命令',
    editFile: '编辑文件',
    typeInTerminal: '向终端输入',
    run: '运行',
    exit0: '退出码 0',
    done: '完成',
    found: ['找到了：nginx 转发到 ', '127.0.0.1:8081', '，但 API 监听的是 ', '8080', '。'],
    fixed: '修好了。',
    fixedRest: '请求又能正常通过了。',
    dash: '每台服务器的状态，一眼看清。',
    dashSub: '通过 SSH 读取，服务器上什么都不用装。',
    hostsCount: '11 台主机',
    cards: '卡片',
    table: '表格',
    disk: '磁盘',
    up: '运行',
    days: '天',
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
      return { s: lerp(1.2, 0.4, e), cx: 512, cy: 512, dx: LOCK_DX * spring(t - 10.4, 0.95, 0.75), frame: lerpRect(SEA_START, ICON_BOX, e), e };
    }
    return { s: 1.1, cx: 512, cy: lerp(-40, 404, spring(t - 56.5, 0.5, 0.92)), dx: 0, frame: SEA_ALL, e: 0 };
  }

  // Boats are servers: they bob on the waves, blink, and dock at the pier as it rises.
  const BOATS = [
    { name: 'prod-api-tokyo', tag: '#e0654f', dir: -1, seed: 1, t0: 1.0, keys: [[1.0, 430], [4.5, 176, 1.2]], end: [[57.2, -620], [57.3, -60, 2.1]] },
    { name: 'db-osaka-01', tag: '#8c66c9', dir: -1, seed: 2, t0: 1.45, keys: [[1.45, 1100], [4.7, 1230, 1.3]], end: [[57.2, 1700], [57.45, 1170, 2.1]] },
    { name: 'homelab-pi', tag: '#9a9ca6', dir: 1, seed: 3, t0: 1.9, keys: [[1.9, -110], [4.6, -215, 1.4]] },
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
      const rise = (1 - spring(t - 5.45, 1.25, 0.5)) * 480;
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
    const moonO = t < 20 ? clamp(1 - c.e * 2.5) : prog(t, 57.2, 1);
    if (shown(moon, moonO > 0)) {
      moon.setAttribute('transform', `translate(1150 ${n2(170 + Math.sin(t * 0.8) * 6)}) rotate(-18)`);
      moon.setAttribute('opacity', moonO.toFixed(3));
    }

    for (const b of boats) {
      const intro = t >= b.t0 && t < 10.6;
      const outro = !!b.end && t >= 57.2;
      if (!shown(b.g, intro || outro)) continue;
      const keys = intro ? b.keys : b.end;
      const x = keyX(keys, t);
      const v = (keyX(keys, t + 0.02) - keyX(keys, t - 0.02)) / 0.04;
      let y = surfY(x, phi, A) + 8 + Math.sin(t * 2.1 + b.seed) * 5;
      let open = blink(t, b.seed);
      if (intro) {
        y += (1 - spring(t - b.t0, 1.5, 0.42)) * 130;
        y += ease.in(prog(t, 8.9 + b.seed * 0.08, 0.9)) * 110;
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
caption(C.sea1, 1.2, 4.0, 'headline big night', 112);
caption(C.sea2, 4.5, 7.7, 'headline big night', 112);
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
    mark.root.style.left = `${n2(960 + LOCK_DX + 179.2 + 66)}px`;
    mark.root.style.top = '540px';
    mark.root.style.transform = 'translateY(-54%)';
    mark.update(t, 10.55, 13.4, { gap: 0.055, f: 2.0, z: 0.5, rise: 70, spin: -14, blur: 6 });
    tag.update(t, 11.3, 13.35, { gap: 0.03 });
  });
  cue(10.35, 'chime', { gain: -2 });
  [0, 2, 4, 7, 9, 12].forEach((p, i) => cue(10.6 + i * 0.055, 'pop', { pitch: p + 5, gain: -14 }));
  cue(13.35, 'swish', { gain: -10 });
}

/* ---------- the app window ---------- */

const W1 = { x: 230, y: 262, w: 1460, h: 780, r: 30 };
const W2 = { x: 120, y: 262, w: 1680, h: 780, r: 30 };
const SIDE = 300;
const AIW = 620;
const T_MORPH = 13.6;
const T_TERM = 20.3;
const T_AI = 28.7;
const T_AI_OUT = 41.2;
const T_DASH = 41.4;
const T_WIPE = 47.5;

const OS = {
  ubuntu: ['#e95420', 'ph-linux-logo'],
  debian: ['#a81d33', 'ph-linux-logo'],
  raspbian: ['#a22846', 'ph-linux-logo'],
  windows: ['#0078d4', 'ph-windows-logo'],
};
const TAGC = {
  production: 'var(--tag-1)',
  api: 'var(--tag-5)',
  database: 'var(--tag-3)',
  edge: 'var(--tag-4)',
  staging: 'var(--tag-2)',
  personal: 'var(--tag-7)',
  ci: 'var(--tag-6)',
};
const osBadge = (os, cls = 'os') => `<div class="${cls}" style="background:${OS[os][0]}"><i class="ph-fill ${OS[os][1]}"></i><span class="dot"></span></div>`;
const tagPills = (tags) => `<div class="tags">${tags.map((g) => `<span class="tag"><b style="background:${TAGC[g]}"></b>${g}</span>`).join('')}</div>`;

const app = (() => {
  const iconRect = () => ({ x: 960 + LOCK_DX - 179.2, y: 540 - 179.2, w: 358.4, h: 358.4, r: 83.2 });
  const aiOpen = (t) => spring(t - T_AI, 1.0, 0.82) - spring(t - T_AI_OUT, 1.0, 0.9);
  const sideOpen = (t) => 1 - spring(t - (T_TERM + 0.25), 1.1, 0.9) + spring(t - T_DASH, 1.1, 0.78);
  function winRect(t) {
    const r = lerpRect(iconRect(), W1, spring(t - T_MORPH, 0.95, 0.8));
    const a = aiOpen(t);
    r.x += (W2.x - W1.x) * a;
    r.w += (W2.w - W1.w) * a;
    return r;
  }

  const win = el(L.app, 'win');
  const icon = el(win, 'win-icon', `<img src="${ICON_SRC}" alt="">`);

  // sidebar
  const side = el(win, 'side');
  const sideIn = el(side, 'side-in');
  sideIn.innerHTML = `<div class="side-top"><i class="ph ph-gear-six"></i><i class="ph ph-sidebar-simple"></i></div>
    <div class="nav"><div class="sel"></div>
      <div class="row"><i class="ph ph-gauge"></i>${C.dashboard}</div>
      <div class="row"><i class="ph ph-hard-drives"></i>${C.allHosts}<span class="n">11</span></div>
      <div class="row"><i class="ph ph-star"></i>${C.favorites}<span class="n">4</span></div>
      <div class="row"><i class="ph ph-clock-counter-clockwise"></i>${C.recent}</div>
      <div class="sec">${C.groups}</div>
      <div class="row"><i class="ph ph-folder-simple"></i>Tokyo Prod<span class="n">3</span></div>
      <div class="row"><i class="ph ph-folder-simple"></i>Osaka DB<span class="n">2</span></div>
      <div class="row"><i class="ph ph-folder-simple"></i>Staging<span class="n">2</span></div>
      <div class="row"><i class="ph ph-folder-simple"></i>Homelab<span class="n">2</span></div>
      <div class="sec">${C.tags}</div>
      <div class="row"><b style="background:var(--tag-1)"></b>production<span class="n">5</span></div>
      <div class="row"><b style="background:var(--tag-5)"></b>api<span class="n">2</span></div>
      <div class="row"><b style="background:var(--tag-3)"></b>database<span class="n">2</span></div>
    </div>
    <div class="side-foot"><i class="ph ph-cloud-check"></i><div><div>${C.synced}</div><small>${C.syncedVia}</small></div><i class="ph ph-lock-simple"></i></div>`;
  const sel = sideIn.querySelector('.sel');
  const sideItems = [...sideIn.querySelectorAll('.side-top,.row,.sec,.side-foot')];

  // main area
  const main = el(win, 'main');
  const tb = el(main, 'tb');
  const TABS = [
    { html: `<i class="ph ph-hard-drives"></i>${C.allHosts}`, t0: 14.05, t1: T_DASH + 0.1, on: (t) => t < T_TERM + 0.3 },
    { html: `<span class="os-dot" style="background:${OS.ubuntu[0]}"><i class="ph-fill ph-linux-logo"></i></span>prod-api-tokyo`, t0: T_TERM + 0.3, t1: T_DASH + 0.1, on: () => true },
    { html: `<i class="ph ph-gauge"></i>${C.dashboard}`, t0: T_DASH + 0.15, t1: 99, on: () => true },
  ].map((d) => ({ ...d, e: el(tb, 'tab', d.html), w: 0 }));
  const plus = el(tb, 'tab-plus', '<i class="ph ph-plus"></i>');
  const tbRight = el(tb, 'tb-right', '<span class="ai-btn"><i class="ph ph-sparkle"></i></span><span class="cap"><i class="ph ph-minus"></i></span><span class="cap"><i class="ph ph-square"></i></span><span class="cap"><i class="ph ph-x"></i></span>');
  const aiBtn = tbRight.querySelector('.ai-btn');
  onMeasure(() => {
    for (const tab of TABS) tab.w = tab.e.offsetWidth;
  });
  const content = el(main, 'content');

  /* hosts */
  const hostsV = el(content, 'view');
  const search = el(hostsV, 'search', `<i class="ph ph-magnifying-glass"></i><span class="q"></span><span class="caret"></span><span class="ph-text">${C.search}</span>`);
  const searchQ = search.querySelector('.q');
  const searchCaret = search.querySelector('.caret');
  const searchPh = search.querySelector('.ph-text');
  const HOSTS = [
    ['db-osaka-01', 'postgres@10.24.3.11', 'debian', ['production', 'database'], true],
    ['prod-api-tokyo', 'deploy@43.206.118.27', 'ubuntu', ['production', 'api'], true],
    ['homelab-pi', 'pi@192.168.1.20', 'raspbian', ['personal'], false],
    ['staging-web-02', 'ubuntu@172.31.40.8', 'ubuntu', ['staging'], false],
    ['bastion-tokyo', 'ops@bastion.tky.example.net', 'debian', ['production', 'edge'], true],
    ['ci-runner-01', 'runner@192.168.50.21', 'windows', ['ci'], false],
    ['edge-sg-cache', 'root@159.89.204.73', 'ubuntu', ['edge'], false],
    ['prod-api-tokyo-02', 'deploy@43.206.118.31', 'ubuntu', ['production', 'api'], false],
    ['db-osaka-02', 'postgres@10.24.3.12', 'debian', ['production', 'database'], false],
  ];
  const cards = HOSTS.map(([name, addr, os, tags, fav], i) => {
    const nm = name.replace('tokyo', '<span class="hl">tokyo</span>');
    const e = el(hostsV, 'card', `${osBadge(os)}<div class="name">${nm}${fav ? '<i class="ph-fill ph-star fav"></i>' : ''}</div><div class="addr">${addr}</div>${tagPills(tags)}`);
    return { e, i, rank: ['prod-api-tokyo', 'bastion-tokyo', 'prod-api-tokyo-02'].indexOf(name), hl: e.querySelector('.hl') };
  });
  const slot = (i) => ({ x: 32 + (i % 3) * 373, y: 108 + Math.floor(i / 3) * 192 });
  const expander = el(content, 'expander');

  /* terminal */
  const termV = el(content, 'view term');
  el(
    termV,
    'status',
    `<span class="ok"><b></b>${C.connected}</span><span class="mono">deploy@43.206.118.27:22</span><span class="dim">38 ms</span><span class="grow"></span><span class="dim"><i class="ph ph-gauge"></i> CPU <em>10%</em></span><span class="dim">${C.memory} <em>46%</em></span><i class="ph ph-magnifying-glass"></i><i class="ph ph-arrows-left-right"></i><span class="dim"><i class="ph ph-folder-simple"></i> SFTP</span><i class="ph ph-dots-three"></i>`,
  );
  const linesBox = el(termV, 'lines');
  const PROMPT = '<span class="pu">deploy@prod-api-tokyo</span>:<span class="pp">/srv/api</span>$ ';
  const PROMPT_LEN = 'deploy@prod-api-tokyo:/srv/api$ '.length;
  const LINES = [
    { at: 21.2, html: '<span class="dim">Last login: Thu Oct  8 09:41:12 2026 from 10.0.0.4</span>' },
    { at: 21.45, type: 'cd /srv/api && git log --oneline -3', t0: 21.65, cps: 34 },
    { at: 22.8, html: '<span class="y">a3f91c2</span> (<span class="c">HEAD -&gt; </span><span class="g">main</span>) fix: retry D1 writes on 429' },
    { at: 22.86, html: '<span class="y">7be0d14</span> chore: bump tokio to 1.41' },
    { at: 22.92, html: '<span class="y">e02c5aa</span> feat: add /healthz latency histogram' },
    { at: 23.0, type: 'curl -sI localhost/v1/hosts', t0: 26.25, cps: 32 },
    { at: 27.2, html: '<span class="r bold">HTTP/1.1 502 Bad Gateway</span>', flash: 'bad' },
    { at: 27.26, html: 'Server: nginx/1.24.0 (Ubuntu)' },
    { at: 27.32, html: 'Content-Type: text/html' },
    { at: 27.4, type: 'curl -sI localhost/v1/hosts', t0: 35.95, cps: 40, ai: true },
    { at: 36.75, html: '<span class="g bold">HTTP/1.1 200 OK</span>', flash: 'good' },
    { at: 36.81, html: 'Server: nginx/1.24.0 (Ubuntu)' },
    { at: 36.87, html: 'Content-Type: application/json' },
    { at: 36.95, type: '', t0: 99, cps: 1 },
  ].map((l, i) => {
    const e = el(linesBox, 'line', l.type != null ? `${PROMPT}<span class="typed"></span>` : l.html);
    e.style.top = `${i * 34}px`;
    if (l.type) for (let k = 0; k < l.type.length; k++) cue(l.t0 + k / l.cps, 'key', { gain: -15 + rnd(i * 50 + k) * 4, pitch: (rnd(i * 70 + k) - 0.5) * 4 + (l.ai ? 3 : 0) });
    return { ...l, i, e, typed: e.querySelector('.typed') };
  });
  const tcaret = el(linesBox, 'tcaret');
  let charW = 12;
  onMeasure(() => {
    const probe = el(linesBox, 'line', 'MMMMMMMMMMMMMMMMMMMM');
    charW = probe.offsetWidth / 20;
    probe.remove();
  });
  cue(27.2, 'boop', { gain: -5 });
  cue(36.75, 'ding', { gain: -8, pitch: 7 });

  // SFTP: a file dragged in from the desktop uploads beside the terminal.
  const sftp = el(
    termV,
    'sftp',
    '<div class="sftp-head"><i class="ph ph-folder-simple"></i><span>/srv/api</span><span class="grow"></span><i class="ph ph-arrow-up"></i><i class="ph ph-arrow-clockwise"></i></div>',
  );
  const upRow = el(sftp, 'frow up', '<i class="ph-fill ph-file-archive" style="color:var(--orange)"></i>release-0.5.0.tar.gz<span class="grow"></span><span class="sz pct"></span><i class="ph-fill ph-check-circle okc"></i><div class="bar"><b></b></div>');
  const upPct = upRow.querySelector('.pct');
  const upOk = upRow.querySelector('.okc');
  const upBar = upRow.querySelector('.bar b');
  const upBarBox = upRow.querySelector('.bar');
  const fileRows = [
    ['ph-fill ph-folder-simple fd', 'logs', '—'],
    ['ph-fill ph-folder-simple fd', 'releases', '—'],
    ['ph ph-file fl', 'Cargo.toml', '3.1 KB'],
    ['ph ph-file fl', 'deploy.sh', '1.2 KB'],
    ['ph ph-file fl', 'api.conf', '812 B'],
    ['ph ph-file fl', 'README.md', '4.6 KB'],
  ].map(([ic, name, size]) => el(sftp, 'frow', `<i class="${ic}"></i>${name}<span class="grow"></span><span class="sz">${size}</span>`));
  const drag = el(L.fx, 'dragchip', '<i class="ph-fill ph-file-archive"></i>release-0.5.0.tar.gz');
  cue(23.3, 'swish', { gain: -9 });
  cue(24.45, 'bubble', { gain: -6, pitch: 5 });
  cue(25.42, 'ding', { gain: -8 });
  cue(25.95, 'swish', { gain: -11 });

  /* ai panel */
  const ai = el(win, 'ai');
  const aiIn = el(ai, 'ai-in');
  aiIn.innerHTML = `<div class="ai-head"><i class="ph-fill ph-sparkle"></i><span class="ai-title"></span><span class="grow"></span><i class="ph ph-clock-counter-clockwise"></i><i class="ph ph-note-pencil"></i><i class="ph ph-x"></i></div>
    <div class="ai-msgs"></div>
    <div class="ai-input"><div class="ai-text"><span class="ai-q"></span><span class="caret"></span><span class="ai-ph">${C.placeholder}</span></div><div class="ai-tools"><i class="ph ph-plus"></i><i class="ph ph-shield-check"></i><span class="grow"></span><span class="send"><i class="ph-bold ph-arrow-up"></i></span></div></div>`;
  const aiTitle = aiIn.querySelector('.ai-title');
  const msgs = aiIn.querySelector('.ai-msgs');
  const aiQ = aiIn.querySelector('.ai-q');
  const aiPh = aiIn.querySelector('.ai-ph');
  const aiCaret = aiIn.querySelector('.ai-text .caret');
  const send = aiIn.querySelector('.send');

  const item = (cls, html) => el(msgs, `ai-item ${cls}`, html);
  const toolCard = (icon, label, cmd, res, extra = '') =>
    item('tool', `<div class="tool-row"><i class="ph ${icon} ti"></i><span class="tl">${label}</span><span class="tc">${cmd}</span><span class="run">${C.run}</span><span class="spin"></span><span class="res"><i class="ph-bold ph-check"></i>${res}</span></div>${extra}`);
  const bubble = item('msg-user', C.prompt);
  const reason = item('msg-reason', `<span class="dots"><b></b><b></b><b></b></span><span class="rl"><i class="ph ph-brain"></i>${C.reasoning}<i class="ph ph-caret-right"></i></span>`);
  const cardA = toolCard('ph-terminal-window', C.runCommand, 'sudo tail -n 3 /var/log/nginx/error.log', C.exit0);
  const found = item('msg-text', C.found.map((s, i) => (i % 2 ? `<code class="u">${s}</code>` : s.match(UNIT).map((u) => (/^\s+$/.test(u) ? u : `<span class="u">${u}</span>`)).join(''))).join(''));
  const cardB = toolCard('ph-pencil-simple', C.editFile, '/etc/nginx/conf.d/api.conf', C.done, '<div class="diff"><div class="del">-  proxy_pass http://127.0.0.1:8081;</div><div class="add">+  proxy_pass http://127.0.0.1:8080;</div></div>');
  const cardC = toolCard('ph-keyboard', C.typeInTerminal, 'curl -sI localhost/v1/hosts', C.done);
  const final = item('msg-final', `<div><b>${C.fixed}</b>${C.fixedRest}</div><div class="flip"><span class="pill bad">502 Bad Gateway</span><i class="ph-bold ph-arrow-right"></i><span class="pill good">200 OK</span></div>`);
  const foundUnits = [...found.querySelectorAll('.u')];
  const diffDel = cardB.querySelector('.del');
  const diffAdd = cardB.querySelector('.add');
  const badPill = final.querySelector('.bad');
  const goodPill = final.querySelector('.good');
  const flipArrow = final.querySelector('.flip i');
  const dots = [...reason.querySelectorAll('.dots b')];
  const reasonLabel = reason.querySelector('.rl');

  // y is in panel coordinates; the message list starts 60px down and scrolls as replies arrive.
  const ITEMS = [
    { e: bubble, y: 84, h: 92, t: 31.0 },
    { e: reason, y: 196, h: 32, t: 31.15 },
    { e: cardA, y: 240, h: 60, t: 32.0, click: 32.6, done: 33.0 },
    { e: found, y: 316, h: 62, t: 33.15 },
    { e: cardB, y: 394, h: 144, t: 34.0, click: 34.85, done: 35.1 },
    { e: cardC, y: 554, h: 60, t: 35.25, click: 35.65, done: 36.8 },
    { e: final, y: 630, h: 86, t: 37.0 },
  ];
  const LIMIT = 636;
  let prevNeed = 0;
  for (const it of ITEMS) {
    const need = Math.max(0, it.y + it.h - LIMIT);
    it.dScroll = need - prevNeed;
    prevNeed = need;
  }
  const scroll = (t) => ITEMS.reduce((s, it) => s + it.dScroll * spring(t - it.t, 1.3, 0.85), 0);
  const PANEL_X = W2.x + W2.w - AIW;
  let goodAt = { x: 1500, y: 900 };
  onMeasure(() => {
    const it = ITEMS[ITEMS.length - 1];
    goodAt = { x: PANEL_X + 24 + goodPill.offsetLeft + goodPill.offsetWidth / 2, y: W2.y + it.y - scroll(37.35) + goodPill.offsetTop + goodPill.offsetHeight / 2 };
  });

  for (const it of ITEMS.filter((x) => x.click)) {
    cue(it.t, 'pop', { gain: -9, pitch: 2 });
    cue(it.done, 'tick', { gain: -8 });
  }
  cue(31.0, 'send', { gain: -5 });
  cue(33.15, 'pop', { gain: -12, pitch: 7 });
  cue(37.0, 'pop', { gain: -10, pitch: 4 });
  cue(37.35, 'success', { gain: -3 });
  cue(37.38, 'sparkle', { gain: -7 });
  const promptChars = [...C.prompt];
  const promptCps = promptChars.length / 1.15;
  promptChars.forEach((_, k) => cue(29.6 + k / promptCps, 'key', { gain: -16 + rnd(900 + k) * 4, pitch: (rnd(950 + k) - 0.5) * 4 }));

  /* dashboard */
  const dashV = el(content, 'view');
  const dashHead = el(dashV, 'dash-head', `<h2>${C.dashboard}</h2><span>${C.hostsCount}</span><div class="seg"><b class="on"><i class="ph ph-squares-four"></i>${C.cards}</b><b><i class="ph ph-rows"></i>${C.table}</b></div>`);
  const DASH = [
    ['bastion-tokyo', 'ops@bastion.tky.example.net', 'debian', ['production', 'edge'], true, [3, 30, 32], 154, 41, 97],
    ['prod-api-tokyo', 'deploy@43.206.118.27', 'ubuntu', ['production', 'api'], true, [41, 58, 41], 155, 47, 41],
    ['prod-api-tokyo-02', 'deploy@43.206.118.31', 'ubuntu', ['production', 'api'], false, [62, 70, 41], 258, 48, 41],
    ['db-osaka-01', 'postgres@10.24.3.11', 'debian', ['production', 'database'], true, [19, 81, 38], 279, 48, 203],
    ['db-osaka-02', 'postgres@10.24.3.12', 'debian', ['production', 'database'], false, [12, 44, 41], 96, 22, 2],
    ['homelab-pi', 'pi@192.168.1.20', 'raspbian', ['personal'], false, [7, 36, 52], 31, 9, 64],
  ];
  const dcards = DASH.map(([name, addr, os, tags, fav, vals, rx, tx, up], i) => {
    const e = el(
      dashV,
      'dcard',
      `${osBadge(os)}<div class="nm">${name}${fav ? '<i class="ph-fill ph-star fav"></i>' : ''}</div><div class="addr">${addr}</div>${tagPills(tags)}${['CPU', C.memory, `${C.disk} /`]
        .map((m, k) => `<div class="met" style="top:${146 + k * 32}px"><span>${m}</span><div class="trk"><b></b></div><em></em></div>`)
        .join('')}<div class="foot"></div>`,
    );
    const mets = [...e.querySelectorAll('.met')].map((m) => ({ m, b: m.querySelector('b'), em: m.querySelector('em') }));
    return { e, i, vals, rx, tx, up, mets, foot: e.querySelector('.foot') };
  });
  cue(43.6, 'boop', { gain: -12, pitch: 5 });

  // Positions the cursor reaches, in stage coordinates.
  const contentAt = (w, side, x, y) => ({ x: w.x + side + x, y: w.y + 64 + y });
  const SEARCH_AT = contentAt(W1, SIDE, 220, 54);
  const CARD_AT = contentAt(W1, SIDE, 32 + 174, 108 + 92);
  const AI_BTN_AT = { x: W1.x + W1.w - 208, y: W1.y + 32 };
  const INPUT_AT = { x: PANEL_X + 200, y: W2.y + 690 };
  const SEND_AT = { x: PANEL_X + 24 + 572 - 14 - 22, y: W2.y + 652 + 104 - 12 - 22 };
  const runAt = (it) => ({ x: PANEL_X + AIW - 24 - 14 - 36, y: W2.y + it.y + 30 - scroll(it.click) });

  cursor.show(16.2, 20.25);
  cursor.at(16.2, 1330, 930);
  cursor.to(16.3, SEARCH_AT.x, SEARCH_AT.y, 0.6);
  cursor.click(16.95);
  cursor.to(19.15, CARD_AT.x, CARD_AT.y, 0.65);
  cursor.click(19.95);
  cursor.click(20.1);
  for (let k = 0; k < 5; k++) cue(17.15 + k * 0.13, 'key', { gain: -13, pitch: (rnd(300 + k) - 0.5) * 4 });
  cue(17.95, 'swish', { gain: -11 });
  cue(T_TERM, 'whoosh', { gain: -7, dur: 0.9 });

  cursor.show(23.55, 24.75);
  cursor.at(23.55, 1790, 1010);
  cursor.to(23.62, 1440, 524, 0.78);
  cursor.click(24.45, false);

  cursor.show(27.6, 36.0);
  cursor.at(27.6, 1240, 760);
  cursor.to(27.7, AI_BTN_AT.x, AI_BTN_AT.y, 0.6);
  cursor.click(28.5);
  cue(T_AI, 'whoosh', { gain: -7, dur: 0.8 });
  cursor.to(28.9, INPUT_AT.x, INPUT_AT.y, 0.5);
  cursor.click(29.45);
  cursor.to(30.45, SEND_AT.x, SEND_AT.y, 0.4);
  cursor.click(30.95);
  for (const it of ITEMS.filter((x) => x.click)) {
    const p = runAt(it);
    cursor.to(it.click - 0.55, p.x, p.y, 0.45);
    cursor.click(it.click);
  }
  cue(T_AI_OUT, 'swish', { gain: -9 });

  /* per frame */
  onFrame((t) => {
    // A gentle push-in while the assistant works; the cursor and effects ride along.
    const zoom = 1 + 0.1 * (spring(t - 30.9, 0.7, 0.9) - spring(t - 40.4, 0.7, 0.9));
    const cam = Math.abs(zoom - 1) < 1e-4 ? '' : `translate(960px,652px) scale(${zoom.toFixed(5)}) translate(-960px,-652px)`;
    for (const l of [L.app, L.fx, L.cursor]) {
      l.style.transformOrigin = '0 0';
      l.style.transform = cam;
    }
    if (!shown(L.app, t >= T_MORPH && t < T_WIPE + 0.72)) return;
    const r = winRect(t);
    win.style.cssText = `left:${n2(r.x)}px;top:${n2(r.y)}px;width:${n2(r.w)}px;height:${n2(r.h)}px;border-radius:${n2(r.r)}px`;
    const iconO = 1 - prog(t, T_MORPH + 0.02, 0.32);
    if (shown(icon, iconO > 0)) {
      const s = (Math.max(r.w, r.h) * 1024) / 896;
      icon.style.cssText = `width:${n2(s)}px;height:${n2(s)}px;margin:${n2(-s / 2)}px 0 0 ${n2(-s / 2)}px;opacity:${iconO.toFixed(3)}`;
    }

    const sw = SIDE * clamp(sideOpen(t));
    const aw = AIW * clamp(aiOpen(t), 0, 1.06);
    side.style.width = `${n2(sw)}px`;
    sideIn.style.transform = `translateX(${n2(sw - SIDE)}px)`;
    main.style.left = `${n2(sw)}px`;
    main.style.width = `${n2(r.w - sw - aw)}px`;
    if (shown(ai, aw > 0.5)) ai.style.width = `${n2(aw)}px`;
    const mainW = r.w - sw - aw;

    sideItems.forEach((e, i) => {
      const a = spring(t - (13.95 + i * 0.03), 2.2, 0.75);
      e.style.opacity = clamp(a * 1.5).toFixed(3);
      e.style.transform = `translateX(${n2((1 - a) * -36)}px)`;
    });
    sel.style.transform = `translateY(${n2(8 + 42 * (1 - spring(t - (T_DASH + 0.2), 1.6, 0.7)))}px)`;

    // tabs
    let x = 14;
    for (const tab of TABS) {
      const v = spring(t - tab.t0, 2.2, 0.62) * (1 - ease.in(prog(t, tab.t1, 0.18)));
      if (!shown(tab.e, v > 0.01)) continue;
      tab.e.classList.toggle('on', tab.on(t));
      put(tab.e, x, 0, 0.7 + 0.3 * v, clamp(v * 1.5));
      x += (tab.w + 6) * clamp(v);
    }
    put(plus, x, 0, 1, clamp((t - 14.1) * 3));
    const aiv = aiOpen(t);
    aiBtn.style.background = `rgba(10,132,255,${(0.22 * clamp(aiv) + 0.12 * bump(t, 27.95, 0.7)).toFixed(3)})`;
    aiBtn.style.transform = `scale(${(1 - 0.12 * bump(t, 28.45, 0.2)).toFixed(4)})`;
    tbRight.style.opacity = clamp((t - 14.0) * 3).toFixed(3);

    // hosts
    if (shown(hostsV, t < T_TERM + 0.9)) {
      const sa = spring(t - 14.2, 2.0, 0.75);
      const leave = prog(t, T_TERM, 0.25);
      search.style.opacity = (clamp(sa * 1.5) * (1 - leave)).toFixed(3);
      search.style.transform = `translateY(${n2((1 - sa) * 20)}px)`;
      const focused = t >= 16.95;
      search.style.boxShadow = focused ? `0 0 0 2px rgba(10,132,255,${(0.8 * prog(t, 16.95, 0.15)).toFixed(3)}) inset, 0 0 0 6px rgba(10,132,255,${(0.18 * prog(t, 16.95, 0.15)).toFixed(3)})` : '';
      const n = t < 17.15 ? 0 : Math.min(5, Math.floor((t - 17.15) / 0.13) + 1);
      text(searchQ, 'tokyo'.slice(0, n));
      shown(searchPh, n === 0);
      shown(searchCaret, focused && (n > 0 || (t * 1.9) % 1 < 0.55));
      const fOut = ease.out(prog(t, 17.95, 0.35));
      for (const c of cards) {
        const a = spring(t - (14.35 + c.i * 0.06), 2.0, 0.6);
        let p = slot(c.i);
        let s = 0.6 + 0.4 * a;
        let o = clamp(a * 1.5);
        if (c.rank >= 0) {
          const f = spring(t - (18.0 + c.rank * 0.05), 1.5, 0.72);
          const q = slot(c.rank);
          p = { x: lerp(p.x, q.x, f), y: lerp(p.y, q.y, f) };
          if (c.rank === 0) {
            s *= 1 - 0.035 * (bump(t, 19.95, 0.16) + bump(t, 20.1, 0.16));
            if (t >= T_TERM + 0.02) o = 0;
          } else o *= 1 - leave;
          c.hl.style.background = `rgba(10,132,255,${(0.38 * prog(t, 17.85, 0.2)).toFixed(3)})`;
        } else {
          s *= lerp(1, 0.82, fOut);
          o *= 1 - fOut;
        }
        shown(c.e, o > 0.005);
        put(c.e, p.x, p.y + (1 - a) * 40, s, o);
      }
      if (shown(expander, t >= T_TERM && t < T_TERM + 0.9)) {
        const e = spring(t - T_TERM, 1.2, 0.85);
        const from = { ...slot(0), w: 349, h: 170, r: 22 };
        const q = lerpRect(from, { x: 0, y: 0, w: mainW, h: 716, r: 0 }, e);
        const cg = Math.round(lerp(42, 23, clamp(e)));
        expander.style.cssText = `left:${n2(q.x)}px;top:${n2(q.y)}px;width:${n2(q.w)}px;height:${n2(q.h)}px;border-radius:${n2(q.r)}px;background:rgb(${cg},${cg},${cg + 3});opacity:${(1 - prog(t, T_TERM + 0.6, 0.3)).toFixed(3)}`;
      }
    }

    // terminal
    const termO = prog(t, T_TERM + 0.35, 0.3) * (1 - prog(t, T_DASH, 0.35));
    if (shown(termV, termO > 0)) {
      termV.style.opacity = termO.toFixed(3);
      termV.style.transform = `scale(${(1 - 0.03 * prog(t, T_DASH, 0.35)).toFixed(4)})`;
      let active = null;
      for (const l of LINES) {
        const a = prog(t, l.at, 0.12);
        if (!shown(l.e, a > 0)) continue;
        l.e.style.opacity = a.toFixed(3);
        let dx = 0;
        let bg = 'transparent';
        if (l.flash === 'bad' && t >= l.at) {
          const dt = t - l.at;
          dx = Math.sin(dt * 42) * 9 * Math.exp(-dt * 6);
          bg = `rgba(255,91,82,${(0.24 * bump(t, l.at, 1.2)).toFixed(3)})`;
        }
        if (l.flash === 'good') bg = `rgba(50,215,75,${(0.22 * bump(t, l.at, 1.2)).toFixed(3)})`;
        l.e.style.background = bg;
        l.e.style.padding = l.flash ? '0 8px' : '';
        l.e.style.marginLeft = l.flash ? '-8px' : '';
        l.e.style.transform = `translate(${n2(dx)}px,${n2((1 - a) * 6)}px)`;
        if (l.type != null) {
          active = l;
          const n = t < l.t0 ? 0 : Math.min(l.type.length, Math.floor((t - l.t0) * l.cps) + 1);
          text(l.typed, l.type.slice(0, n));
          l.n = n;
        }
      }
      if (shown(tcaret, !!active)) {
        const typing = t >= active.t0 && t < active.t0 + active.type.length / active.cps + 0.25;
        const aiTyping = active.ai && t >= active.t0 - 0.2 && t < 36.75;
        tcaret.classList.toggle('ai', !!aiTyping);
        tcaret.style.opacity = typing || aiTyping || (t * 1.9) % 1 < 0.55 ? '1' : '0';
        tcaret.style.transform = `translate(${n2((PROMPT_LEN + active.n) * charW)}px,${active.i * 34 + 3}px)`;
      }

      const so = spring(t - 23.3, 1.3, 0.78) - spring(t - 25.95, 1.2, 0.9);
      if (shown(sftp, t > 23.3 && t < 27)) {
        sftp.style.transform = `translateX(${n2((1 - so) * 500)}px)`;
        sftp.style.opacity = clamp(so * 2).toFixed(3);
        const ins = spring(t - 24.45, 1.6, 0.7);
        fileRows.forEach((e, i) => (e.style.transform = `translateY(${n2(70 + (i + ins) * 52)}px)`));
        const up = spring(t - 24.45, 2.0, 0.55);
        shown(upRow, t >= 24.45);
        upRow.style.transform = `translateY(70px) scale(${(0.6 + 0.4 * up).toFixed(4)})`;
        upRow.style.opacity = clamp(up * 2).toFixed(3);
        const p = ease.inOut(prog(t, 24.55, 0.85));
        upBar.style.transform = `scaleX(${p.toFixed(4)})`;
        upBarBox.style.opacity = (1 - prog(t, 25.45, 0.2)).toFixed(3);
        text(upPct, t < 25.42 ? `${Math.round(p * 100)}%` : '24.6 MB');
        const ok = spring(t - 25.42, 2.2, 0.45);
        shown(upOk, t >= 25.42);
        upOk.style.transform = `scale(${ok.toFixed(4)})`;
      }
    }
    if (shown(drag, t >= 23.6 && t < 24.6)) {
      const c = cursor.pos(t);
      const d = prog(t, 24.42, 0.15);
      put(drag, c.x + 20, c.y + 24, (0.9 + 0.1 * spring(t - 23.6, 2, 0.6)) * (1 - 0.5 * d), clamp((t - 23.6) * 6) * (1 - d), -4 + Math.sin(t * 5) * 2);
    }

    // ai panel
    if (aw > 0.5) {
      text(aiTitle, t < 31.0 ? C.newChat : C.chatTitle);
      const n = t < 29.6 ? 0 : Math.min(promptChars.length, Math.floor((t - 29.6) * promptCps) + 1);
      text(aiQ, t < 31.0 ? promptChars.slice(0, n).join('') : '');
      shown(aiPh, n === 0 || t >= 31.0);
      shown(aiCaret, t >= 29.45 && t < 31.0 && (n > 0 || (t * 1.9) % 1 < 0.55));
      const ready = n > 0 && t < 31.0;
      send.style.background = ready ? 'var(--accent)' : '';
      send.style.color = ready ? '#fff' : '';
      send.style.transform = `scale(${(1 - 0.15 * bump(t, 30.9, 0.2)).toFixed(4)})`;
      const sc = scroll(t);
      for (const it of ITEMS) {
        if (!shown(it.e, t >= it.t)) continue;
        const a = spring(t - it.t, 2.0, 0.7);
        let y = it.y - 60 - sc + (1 - a) * 24;
        if (it.e === bubble) y = lerp(500, it.y - 60, spring(t - it.t, 1.4, 0.8)) - sc;
        it.e.style.transform = `translateY(${n2(y)}px) scale(${(0.96 + 0.04 * a).toFixed(4)})`;
        it.e.style.opacity = clamp(a * 2).toFixed(3);
        if (it.click) {
          const run = it.e.querySelector('.run');
          const spin = it.e.querySelector('.spin');
          const res = it.e.querySelector('.res');
          shown(run, t < it.click + 0.08);
          run.style.transform = `scale(${(1 - 0.12 * bump(t, it.click - 0.04, 0.2)).toFixed(4)})`;
          if (shown(spin, t >= it.click + 0.08 && t < it.done)) spin.style.transform = `rotate(${((t * 720) % 360).toFixed(1)}deg)`;
          if (shown(res, t >= it.done)) res.style.transform = `scale(${spring(t - it.done, 2.2, 0.5).toFixed(4)})`;
        }
      }
      dots.forEach((d, j) => (d.style.transform = `translateY(${n2(-Math.abs(Math.sin((t - 31.15) * 7 + j * 0.7)) * 9)}px)`));
      shown(reason.querySelector('.dots'), t < 31.95);
      reasonLabel.style.opacity = prog(t, 31.95, 0.2).toFixed(3);
      foundUnits.forEach((u, i) => (u.style.opacity = prog(t, 33.15 + i * 0.045, 0.12).toFixed(3)));
      for (const [e, t0] of [
        [diffDel, 34.15],
        [diffAdd, 34.3],
      ]) {
        const a = spring(t - t0, 2.0, 0.7);
        e.style.opacity = clamp(a * 2).toFixed(3);
        e.style.transform = `translateX(${n2((1 - a) * 30)}px)`;
      }
      badPill.style.opacity = t < 37.35 ? '1' : '0.5';
      badPill.style.textDecoration = t < 37.35 ? '' : 'line-through';
      const g = spring(t - 37.35, 2.1, 0.45);
      goodPill.style.transform = `scale(${g.toFixed(4)})`;
      goodPill.style.opacity = clamp(g * 2).toFixed(3);
      flipArrow.style.opacity = clamp(g * 2).toFixed(3);
    }

    // dashboard
    const dashO = prog(t, T_DASH + 0.15, 0.3);
    if (shown(dashV, dashO > 0)) {
      dashV.style.opacity = dashO.toFixed(3);
      const ha = spring(t - 41.7, 2, 0.75);
      dashHead.style.opacity = clamp(ha * 1.5).toFixed(3);
      dashHead.style.transform = `translateY(${n2((1 - ha) * 20)}px)`;
      for (const d of dcards) {
        const a = spring(t - (41.85 + d.i * 0.07), 1.9, 0.62);
        put(d.e, 32 + (d.i % 3) * 373, 104 + Math.floor(d.i / 3) * 304 + (1 - a) * 40, 0.7 + 0.3 * a, clamp(a * 1.5));
        d.mets.forEach((m, k) => {
          const fill = spring(t - (42.2 + d.i * 0.07 + k * 0.05), 1.4, 0.7);
          let v = d.vals[k];
          if (d.i === 2 && k === 0) v = lerp(62, 95, spring(t - 43.5, 1.2, 0.8));
          v = v * fill + (fill > 0.9 ? Math.sin(t * 1.3 + d.i + k * 2) * (k ? 0.6 : 2) : 0);
          v = clamp(v, 0, 99);
          m.b.style.transform = `scaleX(${(v / 100).toFixed(4)})`;
          text(m.em, `${Math.round(v)}%`);
          m.m.classList.toggle('hot', v >= 90);
          if (d.i === 2 && k === 0) m.em.style.transform = `rotate(${n2(Math.sin(t * 30) * 6 * bump(t, 43.6, 0.6))}deg)`;
        });
        const jitter = (k) => Math.round(k * (1 + 0.08 * Math.sin(t * 2.3 + d.i)));
        text(d.foot, `↓ ${jitter(d.rx)} KB/s  ↑ ${jitter(d.tx)} KB/s · ${C.up} ${d.up} ${C.days}`);
      }
    }
  });

  return { goodAt: () => goodAt };
})();

caption(C.hosts, 14.1, 20.2, 'headline', 82);
caption(C.hostsSub, 15.0, 20.15, 'sub', 168);
caption(C.term, 21.0, 27.6, 'headline', 82);
caption(C.termSub, 21.8, 27.55, 'sub', 168);
caption(C.ai, 29.0, 41.0, 'headline', 82);
caption(C.aiSub1, 33.2, 37.2, 'sub', 168);
caption(C.aiSub2, 37.5, 40.95, 'sub', 168);
caption(C.dash, 42.0, 47.1, 'headline', 82);
caption(C.dashSub, 42.8, 47.05, 'sub', 168);
cue(T_DASH, 'whoosh', { gain: -9, dur: 0.8 });
for (let i = 0; i < 9; i++) cue(14.35 + i * 0.06, 'pop', { gain: -17, pitch: [0, 2, 4, 7, 9, 12, 14, 16, 19][i] });
for (let i = 0; i < 6; i++) cue(41.85 + i * 0.07, 'pop', { gain: -17, pitch: [0, 2, 4, 7, 9, 12][i] });
cue(13.62, 'whoosh', { gain: -6, dur: 1.0 });

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
      back: 52.6 + i * 0.18,
      unlock: 53.6 + i * 0.15,
      home: 53.95 + i * 0.15,
      drop: 54.1 + i * 0.12,
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
  cue(52.6, 'swish', { gain: -10 });
  cue(56.3, 'swish', { gain: -9 });

  onFrame((t) => {
    if (!shown(L.sync, t >= T_SYNC && t < 57.2)) return;
    const pop = (t0, out) => {
      const a = spring(t - t0, 1.6, 0.55);
      const b = ease.in(prog(t, out, 0.35));
      return { s: (0.3 + 0.7 * a) * (1 - 0.6 * b), o: clamp(a * 2) * (1 - b) };
    };
    let p = pop(48.5, 56.4);
    let sq = 0;
    for (const c of CHIPS) sq += bump(t, c.T.go + 0.72, 0.3) + 0.5 * bump(t, c.T.back, 0.25);
    const cy = CLOUD.y + Math.sin(t * 1.6) * 7;
    cloud.style.transform = `translate(${CLOUD.x}px,${n2(cy)}px) scale(${n2(p.s * (1 + 0.07 * sq))},${n2(p.s * (1 - 0.07 * sq))})`;
    cloud.style.opacity = p.o.toFixed(3);
    const happy = t >= 51.85 && t < 52.75;
    cloudHappy.style.display = happy ? '' : 'none';
    cloudEyes.style.display = happy ? 'none' : '';
    const open = (t + 0.4) % 3.1 < 0.15 ? 0.15 : 1;
    cloudEyes.setAttribute('transform', `translate(0 30) scale(1 ${open}) translate(0 -30)`);
    const lp = pop(48.9, 56.45);
    label.style.transform = `translate(${n2(960 - labelW / 2)}px,${n2(510 + (1 - lp.s) * 20)}px)`;
    label.style.opacity = lp.o.toFixed(3);
    p = pop(48.8, 56.3);
    put(laptop, 440, 690 + Math.sin(t * 1.3) * 4, p.s, p.o);
    p = pop(49.0, 56.35);
    put(monitor, 1480, 680 + Math.sin(t * 1.3 + 1) * 4, p.s, p.o);

    for (const c of CHIPS) {
      const T = c.T;
      const p0 = { x: 471, y: 600 + c.i * 68 };
      const ha = spring(t - T.appear, 2, 0.7);
      const hout = ease.in(prog(t, 56.3 + c.i * 0.05, 0.3));
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
      const out = ease.in(prog(t, 56.3 + c.i * 0.05, 0.3));
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
  caption(C.sync, 48.6, 56.2, 'headline night', 66);
  caption(C.syncSub1, 51.6, 54.15, 'sub night', 152);
  caption(C.syncSub2, 54.5, 56.15, 'sub night', 152);
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
  cue(57.3, 'chime', { gain: -2, pitch: 5 });
  cue(57.2, 'waves', { gain: -12, dur: 7 });
  [0, 2, 4, 7, 9, 12].forEach((p, i) => cue(57.65 + i * 0.05, 'pop', { gain: -15, pitch: p + 5 }));
  pills.forEach((_, i) => cue(58.55 + i * 0.08, 'pop', { gain: -14, pitch: [7, 9, 12, 14, 16][i] }));
  for (const [t0, x, y, s] of [
    [58.0, 1110, 175, 1],
    [58.9, 800, 360, 0.7],
    [59.7, 1150, 400, 0.8],
    [60.8, 790, 190, 0.9],
    [61.6, 1140, 260, 0.6],
    [62.4, 820, 300, 0.8],
  ])
    twinkle(t0, x, y, s);
  onFrame((t) => {
    if (!shown(L.end, t >= 57.1)) return;
    const a = spring(t - 57.3, 1.5, 0.5);
    put(icon, 960, 290 + (1 - a) * 60 + Math.sin(t * 1.5) * 6 * prog(t, 58, 1), 0.2 + 0.8 * a, clamp(a * 2), (1 - a) * -25);
    mark.update(t, 57.65, null, { gap: 0.05, f: 2.0, z: 0.5, rise: 60, spin: -12, blur: 6 });
    tag.update(t, 58.15, null, { gap: 0.03 });
    pills.forEach((p, i) => {
      const b = spring(t - (58.55 + i * 0.08), 2, 0.55);
      p.style.opacity = clamp(b * 2).toFixed(3);
      p.style.transform = `translateY(${n2((1 - b) * 30)}px) scale(${(0.6 + 0.4 * b).toFixed(4)})`;
    });
  });
}

/* ---------- the AI fix's confetti, placed once the layout is measured ---------- */

onMeasure(() => {
  const at = app.goodAt();
  burst(37.35, at.x, at.y, 26, 3);
});

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

init();
