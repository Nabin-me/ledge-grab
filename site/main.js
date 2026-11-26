/* Ledge Grab landing page. Motion curves are the app's own (src/anim.rs). */
(() => {
  "use strict";

  const $ = (s, r = document) => r.querySelector(s);
  const $$ = (s, r = document) => [...r.querySelectorAll(s)];
  const reduced = matchMedia("(prefers-reduced-motion: reduce)").matches;
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

  // ───────── Curves ─────────
  function bezier(x1, y1, x2, y2) {
    const f = (a, b, t) => 3 * (1 - t) ** 2 * t * a + 3 * (1 - t) * t * t * b + t * t * t;
    return (x) => {
      if (x <= 0) return 0;
      if (x >= 1) return 1;
      let lo = 0, hi = 1, t = x;
      for (let i = 0; i < 18; i++) { if (f(x1, x2, t) < x) lo = t; else hi = t; t = (lo + hi) / 2; }
      return f(y1, y2, t);
    };
  }
  const raw = (t) => 1 - Math.exp(-6.2 * t) * Math.cos(7.4 * t);
  const ease = {
    emph: bezier(0.32, 0.72, 0, 1),
    std: bezier(0.4, 0, 0.2, 1),
    spring: (x) => (x <= 0 ? 0 : x >= 1 ? 1 : raw(x) / raw(1)),
  };
  const SPRING = "linear(" + Array.from({ length: 41 }, (_, i) => (raw(i / 40) / raw(1)).toFixed(4)).join(",") + ")";
  const EMPH = "cubic-bezier(.32,.72,0,1)";
  const STD = "cubic-bezier(.4,0,.2,1)";

  const tween = (dur, fn, cb) =>
    new Promise((res) => {
      if (reduced) { cb(1); return res(); }
      const t0 = performance.now();
      const step = (now) => {
        const x = Math.min(1, (now - t0) / dur);
        cb(fn(x));
        x < 1 ? requestAnimationFrame(step) : res();
      };
      requestAnimationFrame(step);
    });
  const lerp = (a, b, t) => a + (b - a) * t;

  // ───────── Mini screenshot art ─────────
  const ART = {
    inv: `<div class="art art-inv"><b>March invoices</b>${'<div class="r"><i></i><i></i><i></i></div>'.repeat(5)}</div>`,
    pal: `<div class="art art-pal">${"<i></i>".repeat(8)}</div>`,
    clock: `<div class="art art-clock"><small>Focus</small><b>24:10</b><i></i></div>`,
    map: `<div class="art art-map"><svg viewBox="0 0 160 100" preserveAspectRatio="none" style="width:100%;height:100%"><path d="M-5 72C40 52 70 92 165 56" stroke="#8fb8d8" stroke-width="11" fill="none"/><path d="M22 0 62 100M0 32 160 22M112 0l18 100" stroke="#fff" stroke-width="5"/><path d="M30 66 70 50l25 2 35-22" stroke="#d64a4a" stroke-width="2" fill="none"/><circle cx="130" cy="30" r="3.4" fill="#d64a4a"/></svg></div>`,
    chat: `<div class="art art-chat"><i></i><i class="me"></i><i></i><i class="me"></i></div>`,
    poster: `<div class="art art-poster"><b>Long<br>Light</b></div>`,
    code: `<div class="art art-code"><i></i><i></i><i></i><i></i><i></i></div>`,
    chart: `<div class="art art-chart"><small>Quarter in review</small><div class="bars">${[22, 30, 34, 44, 40, 58, 62, 72, 92, 84]
      .map((h, i) => `<i style="height:${h}%"${i === 8 ? ' class="hi"' : ""}></i>`).join("")}</div></div>`,
  };
  const tileEl = (inner, cls = "") => {
    const t = document.createElement("div");
    t.className = "tile " + cls;
    t.innerHTML = inner;
    return t;
  };

  // ───────── Hero title: word-by-word blur rise ─────────
  {
    const h = $("#heroTitle");
    let i = 0;
    const words = (text, soft) =>
      text.split(/(\s+)/).map((w) => {
        if (!w.trim()) return w;
        return `<span class="w"><span class="${soft ? "soft" : ""}" style="--i:${i++}">${w}</span></span>`;
      }).join("");
    h.innerHTML = [...h.childNodes]
      .map((n) => (n.nodeName === "BR" ? "<br>" : n.nodeType === 3 ? words(n.textContent, false) : words(n.textContent, true)))
      .join("");
  }

  // ───────── Nav: plain bar at the top, docks into the notch on scroll ─────────
  const nav = $("#nav");
  const navRow = $("#navRow");
  const navNotch = $("#navNotch");
  const shelfBtn = $("#shelfBtn");
  const ledgeTiles = $("#ledgeTiles");
  const ledgeCount = $("#ledgeCount");
  const shots = [{ art: "chart" }, { art: "clock" }, { art: "pal" }];
  let total = 3, docked = false, isOpen = false;
  const ROW_H = 48;
  const flipEls = () => [$(".brand img"), ...$$(".links a"), $("#navDl")];

  function panelDims() {
    const rowW = navRow.offsetWidth;
    const ow = Math.max(rowW, Math.min(612, innerWidth - 36));
    const bw = ow - 32;
    const tw = (bw - 20) / 3;
    return { rowW, ow, bw, tw, th: tw / 1.6, oh: 54 + tw / 1.6 + 12 + 44 + 12 };
  }
  let dims;
  function sizeNotch() {
    if (!docked) {
      // Keep the panel at its docked size so it never stretches to the full bar.
      const ow = Math.min(612, innerWidth - 36), tw = (ow - 52) / 3;
      dims = { ow, bw: ow - 32, tw, th: tw / 1.6 };
      nav.style.setProperty("--bw", dims.bw + "px");
      return;
    }
    dims = panelDims();
    nav.style.setProperty("--bw", dims.bw + "px");
    nav.style.setProperty("--nw", (isOpen ? dims.ow : dims.rowW) + "px");
    nav.style.setProperty("--nh", (isOpen ? dims.oh : ROW_H) + "px");
  }

  function setDocked(v) {
    if (v === docked) return;
    const els = flipEls();
    const first = els.map((e) => e.getBoundingClientRect());
    docked = v;
    if (!v) setOpen(false);
    nav.classList.toggle("docked", v);
    sizeNotch();
    const cx0 = innerWidth / 2;
    els.forEach((e) => {
      const a = first.shift(), b = e.getBoundingClientRect();
      if (!b.width) return;
      const dx = a.left - b.left, dy = a.top - b.top;
      const sx = a.width / b.width || 1;
      if (Math.abs(dx) < 0.5 && Math.abs(dy) < 0.5) return;
      // Items nearest the centre move first when docking, last when leaving.
      const dist = Math.min(1, Math.abs(a.left + a.width / 2 - cx0) / cx0);
      e.animate(
        [{ transform: `translate(${dx}px,${dy}px) scale(${sx})`, transformOrigin: "0 0" }, { transform: "none", transformOrigin: "0 0" }],
        { duration: v ? 820 : 620, delay: v ? dist * 90 : (1 - dist) * 60, easing: v ? SPRING : EMPH, fill: "backwards" }
      );
    });
  }
  const onScroll = () => setDocked(scrollY > 90);
  addEventListener("scroll", onScroll, { passive: true });
  addEventListener("resize", () => { sizeNotch(); renderLedge(); });

  function shotInTile(shot, tw, th) {
    const t = tileEl("");
    if (shot.art) { t.innerHTML = ART[shot.art]; return t; }
    const clip = document.createElement("div");
    clip.className = "shotclip";
    const node = shot.node.cloneNode(true);
    const s = Math.max(tw / shot.w, th / shot.h);
    node.style.left = (tw - shot.w * s) / 2 + "px";
    node.style.top = (th - shot.h * s) / 2 + "px";
    node.style.transform = `scale(${s})`;
    clip.appendChild(node);
    t.appendChild(clip);
    return t;
  }
  function renderLedge() {
    ledgeTiles.innerHTML = "";
    shots.slice(0, 3).forEach((s, i) => {
      const t = shotInTile(s, dims.tw, dims.th);
      t.style.setProperty("--i", i);
      t.addEventListener("click", () => {
        const c = document.createElement("div");
        c.className = "copied";
        c.textContent = "Copied";
        t.appendChild(c);
        setTimeout(() => c.remove(), 1150);
      });
      ledgeTiles.appendChild(t);
    });
  }

  let openT, closeT;
  function setOpen(v) {
    if (v && !docked) return;
    if (v === isOpen) return;
    isOpen = v;
    nav.classList.toggle("open", v);
    if (v) nav.classList.remove("fresh");
    sizeNotch();
  }
  sizeNotch();
  renderLedge();
  onScroll();

  shelfBtn.addEventListener("pointerenter", (e) => {
    if (e.pointerType === "touch") return;
    clearTimeout(closeT);
    openT = setTimeout(() => setOpen(true), 110);
  });
  shelfBtn.addEventListener("click", () => setOpen(!isOpen));
  nav.addEventListener("pointerenter", () => clearTimeout(closeT));
  nav.addEventListener("pointerleave", (e) => {
    if (e.pointerType === "touch") return;
    clearTimeout(openT);
    closeT = setTimeout(() => setOpen(false), 380);
  });
  $("#ledgeClose").addEventListener("click", () => setOpen(false));
  document.addEventListener("pointerdown", (e) => { if (!nav.contains(e.target)) setOpen(false); });
  addEventListener("keydown", (e) => { if (e.key === "Escape") setOpen(false); });

  function landOnLedge(shot) {
    shots.unshift(shot);
    shots.length = Math.min(shots.length, 6);
    total++;
    ledgeCount.textContent = total;
    ledgeCount.classList.remove("tick");
    void ledgeCount.offsetWidth;
    ledgeCount.classList.add("tick");
    navNotch.animate(
      [{ transform: "translate(-50%,0) scale(1,1)" }, { transform: "translate(-50%,0) scale(1.04,1.12)", offset: 0.3 }, { transform: "translate(-50%,0) scale(1,1)" }],
      { duration: 620, easing: SPRING }
    );
    shelfBtn.animate([{ transform: "scale(1)" }, { transform: "scale(1.12)", offset: 0.3 }, { transform: "scale(1)" }], { duration: 600, easing: SPRING });
    if (!isOpen) nav.classList.add("fresh");
    renderLedge();
  }

  // ───────── Hero: a live capture, flying up to the ledge ─────────
  const stage = $("#stage");
  const win = $("#win");
  const sel = $("#sel");
  const selDim = $("#selDim");
  const cursor = $("#cursor");
  const toolbar = $("#toolbar");
  const flash = $("#flash");
  const flyLayer = $("#flyLayer");
  const heroKeys = $$("#heroKeys .kbd");

  let heroVisible = false;
  new IntersectionObserver(([e]) => (heroVisible = e.intersectionRatio >= 0.55), { threshold: [0, 0.55, 0.8] }).observe(stage);
  const until = async (fn) => { while (!fn()) await sleep(200); };

  let cx = 0, cy = 0;
  const placeCursor = (x, y) => { cx = x; cy = y; cursor.style.transform = `translate(${x - 11}px, ${y - 11}px)`; };
  const moveCursor = (x, y, dur = 700) => {
    const x0 = cx, y0 = cy;
    return tween(dur, ease.emph, (t) => placeCursor(lerp(x0, x, t), lerp(y0, y, t)));
  };

  function regionRect(key) {
    const el = win.querySelector(`[data-r="${key}"]`);
    const s = stage.getBoundingClientRect();
    const r = el.getBoundingClientRect();
    const pad = Math.max(6, s.width * 0.008);
    return { x: r.left - s.left - pad, y: r.top - s.top - pad, w: r.width + pad * 2, h: r.height + pad * 2 };
  }
  function makeShot(rect) {
    const s = stage.getBoundingClientRect();
    const w = win.getBoundingClientRect();
    const node = document.createElement("div");
    node.style.cssText = `width:${rect.w}px;height:${rect.h}px;overflow:hidden;background:#fbfaf7`;
    const clone = win.cloneNode(true);
    clone.removeAttribute("id");
    clone.style.cssText = `left:${w.left - s.left - rect.x}px;top:${w.top - s.top - rect.y}px;width:${w.width}px;height:${w.height}px;transform:none;box-shadow:none;border-radius:0`;
    node.appendChild(clone);
    return { node, w: rect.w, h: rect.h };
  }

  async function pressHeroKeys() {
    for (const k of heroKeys) { k.classList.add("down"); await sleep(70); }
    await sleep(240);
    heroKeys.forEach((k) => k.classList.remove("down"));
  }

  async function fly(shot, rect) {
    const s = stage.getBoundingClientRect();
    const sx = s.left + rect.x, sy = s.top + rect.y;
    const el = document.createElement("div");
    el.className = "fly";
    el.style.width = shot.w + "px";
    el.style.height = shot.h + "px";
    el.appendChild(shot.node.cloneNode(true));
    flyLayer.appendChild(el);
    const L = shelfBtn.getBoundingClientRect();
    const k = 22 / shot.w;
    const tx = L.left + L.width / 2 - (shot.w * k) / 2;
    const ty = L.top + L.height / 2 - (shot.h * k) / 2;
    const midX = lerp(sx, tx, 0.55) + (tx > sx ? -40 : 40);
    const midY = lerp(sy, ty, 0.45) - 10;
    const a = el.animate(
      [
        { transform: `translate(${sx}px,${sy}px) scale(1)`, opacity: 1, borderRadius: "4px", easing: SPRING },
        { offset: 0.2, transform: `translate(${sx}px,${sy - 8}px) scale(1.035)`, opacity: 1, easing: "cubic-bezier(.5,0,.75,.2)" },
        { offset: 0.62, transform: `translate(${midX}px,${midY}px) scale(${lerp(1, k, 0.6)})`, opacity: 1, easing: EMPH },
        { offset: 0.9, transform: `translate(${tx}px,${ty}px) scale(${k})`, opacity: 1, borderRadius: "40px" },
        { transform: `translate(${tx}px,${ty + 2}px) scale(${k * 0.6})`, opacity: 0, borderRadius: "40px" },
      ],
      { duration: 1250, fill: "forwards" }
    );
    setTimeout(() => landOnLedge(shot), 1050);
    await a.finished;
    el.remove();
  }

  async function capture(key) {
    const st = stage.getBoundingClientRect();
    const r = regionRect(key);
    cursor.classList.add("on");
    cursor.classList.remove("x");
    if (!cx) placeCursor(st.width * 0.78, st.height * 0.86);
    await sleep(350);
    await pressHeroKeys();
    toolbar.classList.add("on");
    cursor.classList.add("x");
    await moveCursor(r.x, r.y, 800);
    await sleep(140);
    sel.classList.add("on");
    const scale = 2560 / st.width;
    await tween(820, ease.std, (t) => {
      const w = r.w * t, h = r.h * t;
      sel.style.transform = `translate(${r.x}px,${r.y}px)`;
      sel.style.width = w + "px";
      sel.style.height = h + "px";
      selDim.textContent = `${Math.round(w * scale)} × ${Math.round(h * scale)}`;
      placeCursor(r.x + w, r.y + h);
    });
    await sleep(380);
    // Release: grab the frame
    flash.animate([{ opacity: 0 }, { opacity: 0.5, offset: 0.25 }, { opacity: 0 }], { duration: 320, easing: STD });
    const shot = makeShot(r);
    sel.classList.remove("on");
    toolbar.classList.remove("on");
    cursor.classList.remove("x");
    fly(shot, r);
    await sleep(500);
    await moveCursor(st.width * (0.62 + Math.random() * 0.2), st.height * (0.8 + Math.random() * 0.1), 900);
    await sleep(1700);
  }

  (async () => {
    if (reduced) return;
    const order = ["open", "row", "cards", "head"];
    for (let i = 0; ; i++) {
      await until(() => heroVisible && docked && !document.hidden);
      await capture(order[i % order.length]);
    }
  })();

  // ───────── Scroll reveals ─────────
  const revealIO = new IntersectionObserver(
    (ents) => ents.forEach((e) => {
      if (!e.isIntersecting) return;
      e.target.classList.add("in");
      e.target.dispatchEvent(new CustomEvent("reveal"));
      revealIO.unobserve(e.target);
    }),
    { threshold: 0.18, rootMargin: "0px 0px -6% 0px" }
  );
  $$(".reveal").forEach((el) => revealIO.observe(el));

  // Spotlight that follows the pointer
  document.addEventListener("pointermove", (e) => {
    const p = e.target.closest?.(".spot");
    if (!p) return;
    const r = p.getBoundingClientRect();
    p.style.setProperty("--mx", e.clientX - r.left + "px");
    p.style.setProperty("--my", e.clientY - r.top + "px");
  }, { passive: true });


  // Which section owns the keyboard right now?
  const activeSection = () => {
    const y = innerHeight * 0.5;
    for (const id of ["seal", "tools", "ledge-section"]) {
      const r = document.getElementById(id).getBoundingClientRect();
      if (r.top < y && r.bottom > y) return id;
    }
    return null;
  };
  const typing = (e) => e.target.closest?.("input,textarea,[contenteditable]");

  // ───────── Shelf demo ─────────
  const grid = $("#shelfGrid");
  const shMerge = $("#shMerge");
  const shStatus = $("#shStatus");
  const toast = $("#toast");
  const START = ["inv", "pal", "clock", "map", "chat", "poster", "code", "chart"];
  let items = [], picked = [], lastClick = -1, uid = 0;
  let toastT;
  const showToast = (msg) => {
    toast.textContent = msg;
    toast.classList.add("on");
    clearTimeout(toastT);
    toastT = setTimeout(() => toast.classList.remove("on"), 2000);
  };
  const flip = (fn) => {
    const before = new Map($$(".tile", grid).map((t) => [t.dataset.id, t.getBoundingClientRect()]));
    fn();
    $$(".tile", grid).forEach((t) => {
      const b = before.get(t.dataset.id);
      if (!b) return;
      const a = t.getBoundingClientRect();
      const dx = b.left - a.left, dy = b.top - a.top;
      if (dx || dy) t.animate([{ transform: `translate(${dx}px,${dy}px)` }, { transform: "none" }], { duration: 700, easing: SPRING });
    });
  };
  function shelfTile(it, enterIdx) {
    const t = tileEl(ART[it.art] + (it.pdf ? `<div class="pdfov">PDF</div>` : "") + `<span class="tick"></span>` + (it.pin ? pinBadge : ""));
    t.dataset.id = it.id;
    if (enterIdx != null) { t.classList.add("enter"); t.style.setProperty("--i", enterIdx); }
    return t;
  }
  const pinBadge = `<span class="pinb"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round"><path d="M12 17v5M6 10l1-6h10l1 6-3 3H9z"/></svg></span>`;
  function renderShelf(enter = false) {
    grid.innerHTML = "";
    items.forEach((it, i) => grid.appendChild(shelfTile(it, enter ? i : null)));
    syncPicks();
  }
  function syncPicks() {
    $$(".tile", grid).forEach((t) => {
      const n = picked.indexOf(t.dataset.id);
      t.classList.toggle("sel", n >= 0);
      t.querySelector(".tick").textContent = n >= 0 ? n + 1 : "";
    });
    grid.classList.toggle("picking", picked.length > 0);
    shMerge.classList.toggle("dim", picked.length < 2);
    shStatus.innerHTML = picked.length === 0
      ? "Tick two or more shots, then press the arrow"
      : picked.length === 1 ? "<b>1</b> selected. Tick one more to merge"
      : `<b>${picked.length}</b> selected. Press the arrow to merge them into a PDF`;
  }
  function resetShelf() {
    items = START.map((art) => ({ id: "s" + uid++, art }));
    picked = [];
    renderShelf(true);
  }
  resetShelf();
  grid.addEventListener("click", (e) => {
    const t = e.target.closest(".tile");
    if (!t) return;
    const id = t.dataset.id;
    const idx = items.findIndex((x) => x.id === id);
    if (e.shiftKey && lastClick >= 0) {
      const [a, b] = [Math.min(lastClick, idx), Math.max(lastClick, idx)];
      items.slice(a, b + 1).forEach((x) => { if (!picked.includes(x.id)) picked.push(x.id); });
    } else {
      const n = picked.indexOf(id);
      n >= 0 ? picked.splice(n, 1) : picked.push(id);
    }
    lastClick = idx;
    syncPicks();
  });
  async function merge() {
    if (picked.length < 2) return;
    const tiles = picked.map((id) => grid.querySelector(`[data-id="${id}"]`));
    const target = tiles[0].getBoundingClientRect();
    await Promise.all(tiles.slice(1).map((t, i) => {
      const r = t.getBoundingClientRect();
      t.style.zIndex = 5;
      return t.animate(
        [{ transform: "none" }, { transform: `translate(${target.left - r.left}px,${target.top - r.top}px) scale(.86) rotate(${(i % 2 ? 1 : -1) * 4}deg)`, opacity: 0.2 }],
        { duration: 620, delay: i * 70, easing: EMPH, fill: "forwards" }
      ).finished;
    }));
    const first = items.find((x) => x.id === picked[0]);
    const pdf = { id: "s" + uid++, art: first.art, pdf: true };
    flip(() => {
      items = items.filter((x) => !picked.includes(x.id) || x.id === first.id).map((x) => (x.id === first.id ? pdf : x));
      const n = picked.length;
      picked = [];
      renderShelf();
      const pt = grid.querySelector(`[data-id="${pdf.id}"]`);
      pt.animate([{ transform: "scale(.82)" }, { transform: "scale(1)" }], { duration: 750, easing: SPRING });
      showToast(`Merged ${n} shots into one PDF`);
    });
  }
  function del() {
    if (!picked.length) return;
    const gone = [...picked];
    Promise.all(gone.map((id) => grid.querySelector(`[data-id="${id}"]`).animate(
      [{ transform: "none", opacity: 1 }, { transform: "scale(.6)", opacity: 0 }], { duration: 320, easing: STD, fill: "forwards" }).finished))
      .then(() => {
        flip(() => { items = items.filter((x) => !gone.includes(x.id)); picked = []; renderShelf(); });
        showToast(`Moved ${gone.length} to the Recycle Bin`);
      });
  }
  shMerge.addEventListener("click", merge);
  $("#shDel").addEventListener("click", del);
  $("#shReset").addEventListener("click", resetShelf);
  $("#shCopy").addEventListener("click", () => showToast(picked.length ? `Copied ${picked.length} to the clipboard` : "Select a shot to copy"));
  $("#shPin").addEventListener("click", () => {
    if (!picked.length) return showToast("Select a shot to pin");
    items.forEach((x) => { if (picked.includes(x.id)) x.pin = !x.pin; });
    renderShelf();
    showToast("Pinned shots stay on the ledge after seven days");
  });
  $$("#shelfTabs button").forEach((b) => b.addEventListener("click", () => {
    $$("#shelfTabs button").forEach((x) => x.classList.toggle("on", x === b));
    $$(".tile", grid).forEach((t, i) => { t.classList.remove("enter"); void t.offsetWidth; t.style.setProperty("--i", i); t.classList.add("enter"); });
  }));
  let overShelf = false;
  $("#shelfScene").addEventListener("pointerenter", () => (overShelf = true));
  $("#shelfScene").addEventListener("pointerleave", () => (overShelf = false));
  addEventListener("keydown", (e) => {
    if (!overShelf || typing(e)) return;
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "a") { e.preventDefault(); picked = items.map((x) => x.id); syncPicks(); }
    else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "d") { e.preventDefault(); picked = []; syncPicks(); }
    else if (e.key === "Delete") del();
    else if (e.key === "ArrowRight" || e.key === "Enter") merge();
  });

  // ───────── Redaction ─────────
  const doc = $("#doc");
  let sealState = 0; // 0 clean, 1 found, 2 sealed, 3 erased
  function setSeal(s) {
    sealState = s;
    doc.classList.remove("found", "sealed", "erased", "scanning");
    if (s === 1) {
      void doc.offsetWidth;
      doc.classList.add("scanning");
      setTimeout(() => sealState === 1 && doc.classList.add("found"), 380);
    }
    if (s === 2) doc.classList.add("sealed");
    if (s === 3) doc.classList.add("erased");
    $$(".seal-side .step").forEach((st) => {
      const n = +st.dataset.step;
      st.classList.toggle("on", (s === 1 && n === 1) || (s === 2 && n === 2) || (s === 3 && n === 0));
    });
  }
  const pressKbd = (el) => { el.classList.add("down"); setTimeout(() => el.classList.remove("down"), 180); };
  $$(".seal-side .step").forEach((st) => st.addEventListener("click", () => {
    pressKbd(st.querySelector(".kbd"));
    const n = +st.dataset.step;
    setSeal(n === 0 ? (sealState === 3 ? 0 : 3) : n === sealState ? 0 : n);
  }));
  let sealAuto = true;
  doc.closest(".reveal").addEventListener("reveal", async () => {
    await sleep(700);
    if (!sealAuto) return;
    pressKbd($('.step[data-step="1"] .kbd'));
    setSeal(1);
    await sleep(2400);
    if (!sealAuto) return;
    pressKbd($('.step[data-step="2"] .kbd'));
    setSeal(2);
  });

  // ───────── Tools playground ─────────
  const TOOLS = [
    ["R", "Region"], ["B", "Blur"], ["P", "Pixelate"], ["E", "Smart erase"], ["X", "Box"],
    ["A", "Arrow"], ["D", "Pen"], ["H", "Highlight"], ["T", "Text"], ["N", "Steps"],
  ];
  const toolGrid = $("#toolGrid");
  TOOLS.forEach(([k, n]) => {
    const b = document.createElement("button");
    b.className = "toolkey";
    b.dataset.k = k;
    b.innerHTML = `<b>${k}</b><span>${n}</span>`;
    toolGrid.appendChild(b);
  });
  const ov = $("#ov");
  const mini = $("#mini");
  const tgtMail = $("#tgtMail");
  const tgtAmt = $("#tgtAmt");
  const NS = "http://www.w3.org/2000/svg";
  const svg = (tag, attrs) => {
    const el = document.createElementNS(NS, tag);
    for (const k in attrs) el.setAttribute(k, attrs[k]);
    ov.appendChild(el);
    return el;
  };
  const box = (el) => {
    const m = mini.getBoundingClientRect(), r = el.getBoundingClientRect(), k = 470 / m.width;
    return { x: (r.left - m.left) * k, y: (r.top - m.top) * k, w: r.width * k, h: r.height * k };
  };
  const draw = (el, dur = 600, delay = 0) => {
    el.setAttribute("pathLength", "1");
    el.style.strokeDasharray = "1";
    el.style.strokeDashoffset = "1";
    return el.animate([{ strokeDashoffset: 1 }, { strokeDashoffset: 0 }], { duration: dur, delay, easing: EMPH, fill: "forwards" }).finished;
  };
  const popIn = (el, delay = 0) => {
    el.style.transformBox = "fill-box";
    el.style.transformOrigin = "center";
    el.style.opacity = 0;
    return el.animate([{ transform: "scale(0)", opacity: 0 }, { transform: "scale(1)", opacity: 1 }], { duration: 650, delay, easing: SPRING, fill: "forwards" }).finished;
  };
  let runId = 0;
  function clearCanvas() {
    ov.innerHTML = "";
    [tgtMail, tgtAmt].forEach((t) => { t.style.filter = ""; t.style.opacity = ""; });
    tgtMail.querySelector(".pix").style.opacity = 0;
  }
  async function dragRect(b, id) {
    const r = svg("rect", { x: b.x, y: b.y, width: 0, height: 0, fill: "none", stroke: "#3b9eff", "stroke-width": 1.5, "stroke-dasharray": "4 3" });
    await tween(480, ease.std, (t) => { r.setAttribute("width", b.w * t); r.setAttribute("height", b.h * t); });
    if (id !== runId) return;
    r.animate([{ opacity: 1 }, { opacity: 0 }], { duration: 260, delay: 120, fill: "forwards" });
  }
  const RED = "#f0506e";
  const runners = {
    async R(id) {
      const a = box(tgtMail), c = box(tgtAmt);
      const b = { x: 18, y: a.y - 14, w: 434, h: c.y + c.h - a.y + 28 };
      const dim = svg("path", { d: "", fill: "rgba(4,6,12,.5)", "fill-rule": "evenodd" });
      const line = svg("rect", { x: b.x, y: b.y, width: 0, height: 0, fill: "none", stroke: "#fff", "stroke-width": 1.5 });
      const lbl = svg("text", { x: b.x, y: b.y + b.h + 18, fill: "#fff", "font-size": 11, "font-weight": 600, opacity: 0 });
      await tween(800, ease.std, (t) => {
        const w = b.w * t, h = b.h * t;
        dim.setAttribute("d", `M0 0H470V300H0Z M${b.x} ${b.y}h${w}v${h}h${-w}Z`);
        line.setAttribute("width", w); line.setAttribute("height", h);
      });
      if (id !== runId) return;
      lbl.textContent = "1736 × 712";
      lbl.style.paintOrder = "stroke"; lbl.setAttribute("stroke", "rgba(0,0,0,.6)"); lbl.setAttribute("stroke-width", 3);
      lbl.animate([{ opacity: 0 }, { opacity: 1 }], { duration: 300, fill: "forwards" });
    },
    async B(id) {
      await dragRect(box(tgtMail), id);
      if (id !== runId) return;
      tgtMail.style.filter = "blur(5px) saturate(0)";
    },
    async P(id) {
      await dragRect(box(tgtMail), id);
      if (id !== runId) return;
      tgtMail.querySelector(".pix").style.opacity = 1;
    },
    async E(id) {
      const b = box(tgtMail);
      const c = svg("circle", { cx: b.x, cy: b.y + b.h / 2, r: 12, fill: "rgba(255,255,255,.6)", stroke: "#3b9eff", "stroke-width": 1.5 });
      tgtMail.style.transition = "none";
      await tween(900, ease.std, (t) => {
        c.setAttribute("cx", b.x + b.w * t);
        c.setAttribute("cy", b.y + b.h / 2 + Math.sin(t * 18) * 3);
        tgtMail.style.clipPath = `inset(0 0 0 ${t * 100}%)`;
      });
      tgtMail.style.transition = "";
      tgtMail.style.opacity = 0;
      tgtMail.style.clipPath = "";
      if (id === runId) c.animate([{ opacity: 1 }, { opacity: 0 }], { duration: 250, fill: "forwards" });
    },
    async X() {
      const b = box(tgtAmt);
      await draw(svg("rect", { x: b.x - 8, y: b.y - 7, width: b.w + 16, height: b.h + 14, rx: 3, fill: "none", stroke: RED, "stroke-width": 2.5 }), 750);
    },
    async A() {
      const b = box(tgtAmt);
      const x2 = b.x - 4, y2 = b.y + b.h + 6;
      const x1 = x2 - 120, y1 = 286;
      await draw(svg("path", { d: `M${x1} ${y1}L${x2} ${y2}`, stroke: RED, "stroke-width": 3, "stroke-linecap": "round", fill: "none" }), 520);
      const ang = Math.atan2(y2 - y1, x2 - x1);
      const p = (a, l) => `${x2 - l * Math.cos(ang + a)} ${y2 - l * Math.sin(ang + a)}`;
      await popIn(svg("path", { d: `M${x2} ${y2}L${p(0.45, 15)}L${p(-0.45, 15)}Z`, fill: RED, stroke: RED, "stroke-width": 1.5, "stroke-linejoin": "round" }));
    },
    async D() {
      const b = box(tgtMail);
      const y = b.y + b.h + 4;
      let d = `M${b.x} ${y}`;
      for (let i = 1; i <= 12; i++) d += ` Q${b.x + (b.w / 12) * (i - 0.5)} ${y + (i % 2 ? 6 : -4)} ${b.x + (b.w / 12) * i} ${y + 1}`;
      await draw(svg("path", { d, stroke: RED, "stroke-width": 2.5, fill: "none", "stroke-linecap": "round", "stroke-linejoin": "round" }), 900);
    },
    async H() {
      const b = box(tgtAmt);
      const row = box(tgtAmt.parentElement);
      const r = svg("rect", { x: row.x - 6, y: b.y - 3, width: 0, height: b.h + 6, rx: 3, fill: "rgba(250,204,21,.45)" });
      await tween(700, ease.emph, (t) => r.setAttribute("width", (row.w + 12) * t));
    },
    async T(id) {
      const t = svg("text", { x: 300, y: 278, fill: RED, "font-size": 20, "font-weight": 700 });
      const msg = "Pay by Friday";
      for (let i = 1; i <= msg.length; i++) {
        if (id !== runId) return;
        t.textContent = msg.slice(0, i) + (i < msg.length ? "|" : "");
        await sleep(55 + Math.random() * 40);
      }
    },
    async N() {
      const rows = $$(".m-row", mini).slice(0, 3);
      await Promise.all(rows.map((r, i) => {
        const b = box(r);
        const g = document.createElementNS(NS, "g");
        ov.appendChild(g);
        const c = document.createElementNS(NS, "circle");
        Object.entries({ cx: b.x + b.w + 14, cy: b.y + b.h / 2, r: 10, fill: RED }).forEach(([k, v]) => c.setAttribute(k, v));
        const tx = document.createElementNS(NS, "text");
        Object.entries({ x: b.x + b.w + 14, y: b.y + b.h / 2 + 4, fill: "#fff", "font-size": 11, "font-weight": 700, "text-anchor": "middle" }).forEach(([k, v]) => tx.setAttribute(k, v));
        tx.textContent = i + 1;
        g.append(c, tx);
        return popIn(g, i * 180);
      }));
    },
  };
  let currentTool = null;
  async function runTool(k) {
    const id = ++runId;
    currentTool = k;
    $$(".toolkey").forEach((b) => b.classList.toggle("on", b.dataset.k === k));
    const [, name] = TOOLS.find((t) => t[0] === k);
    $("#tnameKey").textContent = k;
    const tn = $("#tname");
    tn.animate([{ opacity: 0, transform: "translateY(6px)" }, { opacity: 1, transform: "none" }], { duration: 450, easing: EMPH });
    tn.textContent = name;
    clearCanvas();
    await runners[k](id);
  }
  let toolsVisible = false, lastManual = 0, cycleIdx = 5;
  new IntersectionObserver(([e]) => (toolsVisible = e.isIntersecting), { threshold: 0.4 }).observe(mini);
  toolGrid.addEventListener("click", (e) => {
    const b = e.target.closest(".toolkey");
    if (!b) return;
    lastManual = Date.now();
    runTool(b.dataset.k);
  });
  (async () => {
    if (reduced) { runTool("A"); return; }
    for (;;) {
      await until(() => toolsVisible && !document.hidden && Date.now() - lastManual > 7000);
      runTool(TOOLS[cycleIdx % TOOLS.length][0]);
      cycleIdx++;
      await sleep(2800);
    }
  })();

  // Global keyboard: B / E for redaction, tool keys for the playground
  addEventListener("keydown", (e) => {
    if (e.ctrlKey || e.metaKey || e.altKey || e.repeat || typing(e)) return;
    const k = e.key.toUpperCase();
    const sec = activeSection();
    if (sec === "seal" && (k === "B" || k === "E")) {
      sealAuto = false;
      if (k === "B") { pressKbd($(`.step[data-step="${sealState === 1 ? 2 : 1}"] .kbd`)); setSeal(sealState === 0 || sealState === 3 ? 1 : sealState === 1 ? 2 : 0); }
      else { pressKbd($('.step[data-step="0"] .kbd')); setSeal(sealState === 3 ? 0 : 3); }
    } else if (sec === "tools" && TOOLS.some((t) => t[0] === k)) {
      lastManual = Date.now();
      runTool(k);
    }
  });

  // ───────── Stats ─────────
  $$("[data-count]").forEach((card) => card.addEventListener("reveal", () => {
    const sp = card.querySelector("[data-to]");
    const to = +sp.dataset.to, from = +sp.dataset.from, dec = +(sp.dataset.dec || 0);
    tween(1500, ease.emph, (t) => (sp.textContent = (to * t).toFixed(dec)));
  }));
  const race = $("#race");
  const runRace = () => {
    $$(".lane i", race).forEach((i) => {
      const ms = +i.dataset.ms;
      i.animate([{ width: "0%" }, { width: (ms / 150) * 100 + "%" }], { duration: 300 + ms * 15, easing: STD, fill: "forwards" });
    });
  };
  race.addEventListener("reveal", () => setTimeout(runRace, 300));
  race.addEventListener("click", runRace);

  // ───────── CTA icon tilt ─────────
  const icon = $("#icon3d .in");
  const cta = $("#download");
  cta.addEventListener("pointermove", (e) => {
    const r = icon.getBoundingClientRect();
    const dx = (e.clientX - (r.left + r.width / 2)) / innerWidth;
    const dy = (e.clientY - (r.top + r.height / 2)) / innerHeight;
    icon.style.transform = `rotateY(${dx * 38}deg) rotateX(${-dy * 38}deg)`;
  });
  cta.addEventListener("pointerleave", () => (icon.style.transform = ""));
})();
