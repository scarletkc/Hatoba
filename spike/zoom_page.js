// Temporary spike (#268): what WebKit reports under page zoom. Returns a JSON string.
(() => {
  const box = document.getElementById("box");
  const native = window.devicePixelRatio;
  const own = Object.getOwnPropertyDescriptor(window, "devicePixelRatio");
  let override;
  try {
    Object.defineProperty(window, "devicePixelRatio", { configurable: true, get: () => native * 7 });
    const seen = window.devicePixelRatio;
    if (own) Object.defineProperty(window, "devicePixelRatio", own);
    else delete window.devicePixelRatio;
    override = { seen, restored: window.devicePixelRatio, own: own ? Object.keys(own) : null };
  } catch (e) {
    override = String(e);
  }
  return JSON.stringify({
    dpr: native,
    innerWidth: window.innerWidth,
    innerHeight: window.innerHeight,
    box100: box.getBoundingClientRect().width,
    resolution: [1, 1.5, 2, 3, 4].filter((r) => matchMedia(`(resolution: ${r}dppx)`).matches),
    override,
  });
})();
