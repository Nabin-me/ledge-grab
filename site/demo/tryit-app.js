// Ledge Grab playground: the hands-on hero demo.
//
// The visitor works a mock desktop the way the app works on Windows:
//   Try a capture (or Ctrl+Shift+S) freezes the desktop.
//   Drag over anything for a region, click a window for that window, or press Screen.
//   B finds emails, card numbers and API keys; B again blurs them. A draws an arrow, X a box.
//   Enter copies: the shot goes to the (demo) clipboard and tucks under the notch.
//   The notch is the ledge: hover it, click a shot to copy it, drag it into the chat,
//   Float it over the desktop, tick two and Merge them into a PDF.
//   In the chat, Ctrl+V pastes whatever was copied last.
//
// Performance rules kept here:
//   - Moving pieces animate transform/opacity only (Web Animations API).
//   - No requestAnimationFrame loop while idle. Pointer moves are batched to one
//     update per frame and only while a drag is in progress.
//   - The stage uses `contain: layout paint`, so nothing here relayouts the page.

const RM = matchMedia("(prefers-reduced-motion: reduce)").matches;
const EMPH = "cubic-bezier(.32,.72,0,1)";
const STD = "cubic-bezier(.4,0,.2,1)";
const SPRING = (() => {
  const v = getComputedStyle(document.documentElement).getPropertyValue("--spring").trim();
  return v && CSS.supports("transition-timing-function", v) ? v : "cubic-bezier(.34,1.4,.64,1)";
})();
const DOWNLOAD = "https://github.com/Nabin-me/ledge-grab/releases/latest";

const ms = (n) => (RM ? 1 : n);
const wait = (n) => new Promise((r) => setTimeout(r, RM ? 0 : n));
const play = (el, frames, o = {}) =>
  el.animate(frames, { duration: ms(o.duration ?? 400), easing: o.easing ?? EMPH, delay: o.delay ? ms(o.delay) : 0, fill: o.fill ?? "forwards" }).finished;
const clamp = (v, a, b) => Math.max(a, Math.min(b, v));
const inside = (r, x, y) => x >= r.left && x <= r.right && y >= r.top && y <= r.bottom;
const overlaps = (a, b) => a.x < b.x + b.w && a.x + a.w > b.x && a.y < b.y + b.h && a.y + a.h > b.y;
const clock = () => new Date().toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
const NS = "http://www.w3.org/2000/svg";
const capture = (el, e) => { try { el.setPointerCapture(e.pointerId); } catch {} };

const ICON = {
  shot: '<path d="M6 2v14a2 2 0 0 0 2 2h14M18 22V8a2 2 0 0 0-2-2H2"/>',
  screen: '<rect x="3" y="4" width="18" height="13" rx="2"/><path d="M8 21h8M12 17v4"/>',
  select: '<path d="M4 8V5a1 1 0 0 1 1-1h3M16 4h3a1 1 0 0 1 1 1v3M20 16v3a1 1 0 0 1-1 1h-3M8 20H5a1 1 0 0 1-1-1v-3"/>',
  blur: '<circle cx="7" cy="7" r="1.6"/><circle cx="12" cy="7" r="1.6"/><circle cx="17" cy="7" r="1.6"/><circle cx="7" cy="12" r="1.6"/><circle cx="12" cy="12" r="1.6"/><circle cx="17" cy="12" r="1.6"/><circle cx="7" cy="17" r="1.6"/><circle cx="12" cy="17" r="1.6"/><circle cx="17" cy="17" r="1.6"/>',
  arrow: '<path d="M5 19 19 5M10 5h9v9"/>',
  box: '<rect x="4" y="6" width="16" height="12" rx="2"/>',
  check: '<path d="m5 12 4.5 4.5L19 7"/>',
  close: '<path d="M6 6l12 12M6 18 18 6"/>',
  copy: '<rect x="8" y="8" width="12" height="12" rx="3"/><path d="M16 8V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h2"/>',
  float: '<rect x="7" y="7" width="13" height="13" rx="3"/><path d="M4 15V6a2 2 0 0 1 2-2h9"/>',
  trash: '<path d="M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13"/>',
  merge: '<path d="M5 12h14M13 6l6 6-6 6"/>',
  paste: '<rect x="6" y="4" width="12" height="17" rx="2"/><path d="M9 4V3h6v1M9 10h6M9 14h4"/>',
  reset: '<path d="M3 12a9 9 0 1 0 3-6.7L3 8"/><path d="M3 3v5h5"/>',
  down: '<path d="M12 4v12M7 11l5 5 5-5M5 20h14"/>',
};
const svgIcon = (k, cls = "") => `<svg class="${cls}" viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${ICON[k]}</svg>`;

const QUESTS = [
  { id: "capture", label: "Take a capture", hint: "Press <b>Try a capture</b>, then drag over anything or click a window." },
  { id: "blur", label: "Blur a secret", hint: "While capturing, press <b>B</b> to find the emails and keys, then <b>B</b> again to blur them." },
  { id: "mark", label: "Mark it up", hint: "While capturing, press <b>A</b> for an arrow or <b>X</b> for a box, then drag." },
  { id: "share", label: "Drag into the chat", hint: "Hover the notch and drag a shot into the chat." },
  { id: "paste", label: "Paste with Ctrl+V", hint: "Every capture is on the clipboard. Click the message box and press <b>Ctrl+V</b>." },
  { id: "float", label: "Float a shot", hint: "Hover the notch, then press <b>Float</b> on a shot. Scroll on it to resize." },
  { id: "pdf", label: "Merge into a PDF", hint: "Tick two shots in the notch, then press <b>Merge</b>." },
];

const BARS = [34, 41, 38, 52, 47, 68, 88];
const DESK = `
<div class="ti-desk" data-ti="desk">
  <div class="ti-win ti-bill" data-win="Northlight Billing">
    <div class="ti-chrome"><i></i><i></i><i></i><span>Northlight Billing · March</span></div>
    <div class="ti-wbody">
      <h4>March invoices <span class="ti-badge">3 open</span></h4>
      <table>
        <tr><td>Ada Merrow</td><td><span class="ti-sec">ada@northlight.example</span></td><td>$1,240.00</td></tr>
        <tr><td>Ivo Castell</td><td><span class="ti-sec">ivo@castell.example</span></td><td>$860.00</td></tr>
        <tr><td>June Alder</td><td><span class="ti-sec">june@alder.example</span></td><td>$2,115.50</td></tr>
        <tr><td>Teo Lindqvist</td><td><span class="ti-sec">teo@fjordworks.example</span></td><td>$430.00</td></tr>
      </table>
      <div class="ti-kv">
        <div><small>Card on file</small><span class="ti-sec">4242 4242 4242 4242</span></div>
        <div><small>API key</small><code class="ti-sec">nl-demo-7QX2-KM9P-40TV</code></div>
      </div>
    </div>
  </div>
  <div class="ti-win ti-report" data-win="Pulse report">
    <div class="ti-chrome"><i></i><i></i><i></i><span>Pulse · Weekly report</span></div>
    <div class="ti-wbody">
      <small>Signups, last 7 days</small>
      <div class="ti-big"><b>4,218</b><span class="ti-up">+38%</span></div>
      <div class="ti-bars">${BARS.map((h, i) => `<i style="height:${h}%"${i === BARS.length - 1 ? ' class="hi"' : ""}></i>`).join("")}</div>
      <div class="ti-days"><span>M</span><span>T</span><span>W</span><span>T</span><span>F</span><span>S</span><span>S</span></div>
    </div>
  </div>
  <div class="ti-note" data-win="Sticky note"><b>To do</b><p>Send March numbers to Mara.<br>Blur the customer emails first!</p></div>
  <div class="ti-taskbar"><i></i><i></i><i></i><i></i><span>9:41</span></div>
  <svg class="ti-marks" data-ti="marks" xmlns="${NS}"></svg>
</div>`;

const TEMPLATE = `
<div class="ti-stage" data-ti="stage">
  ${DESK}
  <div class="ti-floats" data-ti="floats"></div>
  <div class="ti-freeze" data-ti="freeze"></div>
  <div class="ti-hover" data-ti="hover"></div>
  <div class="ti-sel" data-ti="sel"><span class="ti-dim" data-ti="dim"></span></div>

  <div class="ti-tools" data-ti="tools" role="toolbar" aria-label="Capture tools">
    <button type="button" data-tool="select" aria-label="Region or window (R)" data-tip="Region · R">${svgIcon("select")}</button>
    <button type="button" data-act="screen" aria-label="Whole screen" data-tip="Screen">${svgIcon("screen")}</button>
    <span class="ti-sep"></span>
    <button type="button" data-act="blur" aria-label="Find and blur secrets (B)" data-tip="Blur · B">${svgIcon("blur")}</button>
    <button type="button" data-tool="arrow" aria-label="Arrow (A)" data-tip="Arrow · A">${svgIcon("arrow")}</button>
    <button type="button" data-tool="box" aria-label="Box (X)" data-tip="Box · X">${svgIcon("box")}</button>
    <span class="ti-sep"></span>
    <button type="button" class="ti-copy" data-act="copy" aria-label="Copy (Enter)">${svgIcon("check")}<span>Copy</span></button>
    <button type="button" data-act="cancel" aria-label="Cancel (Esc)" data-tip="Cancel · Esc">${svgIcon("close")}</button>
  </div>

  <div class="ti-notch" data-ti="notch">
    <button type="button" class="ti-nbar" data-ti="nbar" aria-label="Open the ledge" aria-expanded="false">
      <span class="ti-cam"></span><span class="ti-count" data-ti="count"><i></i><b>0</b></span>
    </button>
    <div class="ti-ledge" data-ti="ledge">
      <div class="ti-tiles" data-ti="tiles"></div>
      <div class="ti-lbar">
        <span class="ti-lhint" data-ti="lhint">Click a shot to copy it. Drag it into the chat.</span>
        <button type="button" class="ti-merge" data-ti="merge" disabled>${svgIcon("merge")}<span>Merge</span></button>
      </div>
    </div>
  </div>

  <div class="ti-flash" data-ti="flash"></div>
  <div class="ti-toast" data-ti="toast" role="status"></div>
</div>

<div class="ti-chat" data-ti="chat" aria-label="Mock chat">
  <div class="ti-chat-head">
    <span class="ti-hash">#</span>launch-room
    <span class="ti-online"><i style="--c:#8fd0ff">M</i><i style="--c:#e9c46a">T</i><i style="--c:#f4a261">J</i></span>
  </div>
  <div class="ti-msgs" data-ti="msgs" aria-live="polite"></div>
  <div class="ti-composer" data-ti="composer" tabindex="0" role="textbox" aria-label="Message box. Press Control V to paste your last capture.">
    <span data-ti="ph">Message #launch-room</span>
    <button type="button" class="ti-paste" data-ti="paste" hidden>${svgIcon("paste")}Paste</button>
  </div>
  <div class="ti-drop"><span>${svgIcon("down")}Drop to share</span></div>
</div>

<div class="ti-bar">
  <button type="button" class="ti-btn" data-ti="go">
    ${svgIcon("shot")}<span data-ti="golabel">Try a capture</span>
    <span class="ti-keys" aria-hidden="true"><kbd>Ctrl</kbd><kbd>Shift</kbd><kbd>S</kbd></span>
  </button>
  <p class="ti-status" data-ti="status" aria-live="polite"></p>
  <div class="ti-quests">
    <button type="button" class="ti-qbtn" data-ti="qbtn" aria-expanded="false"><span class="ti-dots" data-ti="dots"></span><span data-ti="qcount">0 of ${QUESTS.length} tried</span></button>
    <div class="ti-qlist" data-ti="qlist"></div>
  </div>
</div>`;

export function mount(root) {
  root.classList.add("ti-root");
  root.innerHTML = TEMPLATE;
  const $ = (k) => root.querySelector(`[data-ti="${k}"]`);
  const stage = $("stage"), desk = $("desk"), marksSvg = $("marks"), floats = $("floats");
  const freeze = $("freeze"), hover = $("hover"), sel = $("sel"), dim = $("dim"), tools = $("tools");
  const notch = $("notch"), nbar = $("nbar"), count = $("count"), tilesEl = $("tiles"), lhint = $("lhint"), mergeBtn = $("merge");
  const flash = $("flash"), toastEl = $("toast");
  const chat = $("chat"), msgs = $("msgs"), composer = $("composer"), ph = $("ph"), pasteBtn = $("paste");
  const go = $("go"), golabel = $("golabel"), status = $("status");
  const qbtn = $("qbtn"), qlist = $("qlist"), dots = $("dots"), qcount = $("qcount");

  // ── state ─────────────────────────────────────────────
  let frozen = false, tool = "select", selRect = null, blurStage = 0;
  let marks = [];
  const shots = [];          // newest first: { id, type: "img"|"pdf", r, dw, dh, master, meta, pages }
  const ticked = new Set();
  let clipboard = null, nextId = 1, inView = false, dragging = false;
  const done = new Set();

  new IntersectionObserver(([e]) => root.classList.toggle("inview", (inView = e.isIntersecting)), { threshold: 0.25 }).observe(root);

  // ── small UI helpers ──────────────────────────────────
  const say = (html) => (status.innerHTML = html);
  let toastTimer = 0;
  function toast(text) {
    toastEl.textContent = text;
    toastEl.classList.add("on");
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => toastEl.classList.remove("on"), 2200);
  }
  function nextHint() {
    const q = QUESTS.find((q) => !done.has(q.id));
    return q ? `Next: ${q.hint}` : `That is the whole loop. The real app is one 3 MB download. <a href="${DOWNLOAD}">Get Ledge Grab</a>`;
  }
  function renderQuests() {
    dots.innerHTML = QUESTS.map((q) => `<i class="${done.has(q.id) ? "on" : ""}"></i>`).join("");
    qcount.textContent = `${done.size} of ${QUESTS.length} tried`;
    qlist.innerHTML = `<b>Things to try</b>` + QUESTS.map((q) =>
      `<div class="ti-q ${done.has(q.id) ? "on" : ""}"><span>${done.has(q.id) ? svgIcon("check") : ""}</span><div><b>${q.label}</b><p>${q.hint}</p></div></div>`).join("");
    root.classList.toggle("alldone", done.size === QUESTS.length);
  }
  function quest(id) {
    if (done.has(id)) return;
    done.add(id);
    renderQuests();
    const d = dots.children[QUESTS.findIndex((q) => q.id === id)];
    d && play(d, [{ transform: "scale(.4)" }, { transform: "scale(1)" }], { duration: 600, easing: SPRING });
  }
  qbtn.addEventListener("click", () => {
    const open = !root.classList.contains("qopen");
    root.classList.toggle("qopen", open);
    qbtn.setAttribute("aria-expanded", open);
  });
  document.addEventListener("pointerdown", (e) => {
    if (!e.target.closest?.(".ti-quests")) { root.classList.remove("qopen"); qbtn.setAttribute("aria-expanded", "false"); }
  });

  // ── snapshots ─────────────────────────────────────────
  // A shot keeps a frozen copy of the desktop ("master"). Every view of it
  // (notch tile, chat, float, drag ghost) is a fresh clone of that master,
  // scaled and offset so only the captured rectangle shows.
  function pageView(shot, width) {
    const s = width / shot.r.w;
    const box = document.createElement("div");
    box.className = "ti-shotbox";
    box.style.width = `${width}px`;
    box.style.height = `${shot.r.h * s}px`;
    const c = shot.master.cloneNode(true);
    c.style.cssText = `width:${shot.dw}px;height:${shot.dh}px;transform:scale(${s}) translate(${-shot.r.x}px,${-shot.r.y}px)`;
    box.append(c);
    return box;
  }
  function view(shot, width) {
    if (shot.type !== "pdf") return pageView(shot, width);
    const wrap = document.createElement("div");
    wrap.className = "ti-pdf";
    const first = pageView(shot.pages[0], width);
    wrap.style.width = `${width}px`;
    wrap.style.height = first.style.height;
    wrap.append(first);
    wrap.insertAdjacentHTML("beforeend", `<span class="ti-pdfbadge">PDF · ${shot.pages.length} pages</span>`);
    return wrap;
  }
  const deskRect = () => ({ x: 0, y: 0, w: desk.offsetWidth, h: desk.offsetHeight });

  // ── chat ──────────────────────────────────────────────
  const SEED = [
    { who: "Mara", c: "#8fd0ff", text: "Can someone send the March billing numbers? Blur the customer emails first please." },
    { who: "Teo", c: "#e9c46a", text: "And the signup chart, if you have it open." },
  ];
  function message({ who, c, text, shot, typing }) {
    const el = document.createElement("div");
    el.className = "ti-msg";
    el.innerHTML = `<span class="ti-av" style="--c:${c}">${who[0]}</span><div><div class="ti-meta">${who}<time>${clock()}</time></div></div>`;
    const body = el.lastElementChild;
    if (typing) body.insertAdjacentHTML("beforeend", `<div class="ti-typing"><i></i><i></i><i></i></div>`);
    if (text) body.insertAdjacentHTML("beforeend", `<div class="ti-text">${text}</div>`);
    if (shot) {
      const w = Math.min(210, msgs.clientWidth - 70);
      const box = document.createElement("div");
      if (shot.type === "pdf") {
        box.className = "ti-file";
        box.append(pageView(shot.pages[0], 46));
        box.insertAdjacentHTML("beforeend", `<div><b>ledge-grab-merge.pdf</b><small>${shot.pages.length} pages · lossless</small></div>`);
      } else {
        box.className = "ti-shot";
        box.append(view(shot, w));
      }
      body.append(box);
    }
    msgs.append(el);
    msgs.scrollTop = msgs.scrollHeight;
    return el;
  }
  SEED.forEach((m) => message(m));

  let replyChain = Promise.resolve(), replyIdx = 0;
  function replyTo(shot) {
    const meta = shot.type === "pdf" ? shot.pages.reduce((a, p) => ({ blurred: a.blurred || p.meta.blurred, marks: a.marks || p.meta.marks }), {}) : shot.meta;
    let text;
    if (shot.type === "pdf") text = "One PDF instead of a pile of images. Love it.";
    else if (meta.blurred) text = "Thanks for blurring the private stuff 🙏";
    else if (meta.marks) text = "The markup makes it obvious, thanks.";
    else if (meta.exposed) text = "Got it. Heads up, I can read the customer emails in that one.";
    else text = ["Got it, thanks!", "Perfect, that's the one.", "That was fast."][replyIdx++ % 3];
    const who = meta.exposed && !meta.blurred ? { who: "Teo", c: "#e9c46a" } : { who: "Mara", c: "#8fd0ff" };
    replyChain = replyChain.then(async () => {
      await wait(450);
      const t = message({ ...who, typing: true });
      await wait(1000);
      t.remove();
      message({ ...who, text });
    });
  }
  function share(shot) {
    message({ who: "You", c: "#f3ede1", shot });
    replyTo(shot);
  }

  // Paste: the demo clipboard holds the last capture or copied shot.
  function setClipboard(shot) {
    clipboard = shot;
    pasteBtn.hidden = false;
  }
  function paste() {
    if (!clipboard) { toast("Nothing copied yet. Take a capture first."); return; }
    share(clipboard);
    quest("paste");
    say(nextHint());
  }
  composer.addEventListener("focus", () => (ph.textContent = clipboard ? "Press Ctrl+V to paste your capture" : "Take a capture, then press Ctrl+V here"));
  composer.addEventListener("blur", () => (ph.textContent = "Message #launch-room"));
  composer.addEventListener("keydown", (e) => {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "v") { e.preventDefault(); paste(); }
  });
  pasteBtn.addEventListener("click", (e) => { e.stopPropagation(); paste(); });

  // ── freeze / capture ──────────────────────────────────
  function setTool(t) {
    tool = t;
    tools.querySelectorAll("[data-tool]").forEach((b) => b.classList.toggle("on", b.dataset.tool === t));
    stage.dataset.tool = t;
    hover.classList.remove("on");
  }
  function startFreeze() {
    if (frozen) return;
    frozen = true;
    setOpen(false);
    stage.classList.add("frozen");
    const { w, h } = deskRect();
    marksSvg.setAttribute("viewBox", `0 0 ${w} ${h}`);
    setTool("select");
    selRect = null;
    sel.classList.remove("on");
    tools.querySelector(".ti-copy").disabled = true;
    go.disabled = true;
    golabel.textContent = "Capturing…";
    say("Drag over anything, or click a window. <b>Esc</b> cancels.");
    play(flash, [{ opacity: 0 }, { opacity: 0.12, offset: 0.3 }, { opacity: 0 }], { duration: 260, easing: STD });
  }
  function clearFrame() {
    marks.forEach((m) => m.remove());
    marks = [];
    desk.querySelectorAll(".ti-sec").forEach((s) => s.classList.remove("found", "blurred"));
    blurStage = 0;
  }
  function endFreeze() {
    frozen = false;
    stage.classList.remove("frozen");
    sel.classList.remove("on");
    hover.classList.remove("on");
    selRect = null;
    go.disabled = false;
    golabel.textContent = "Try a capture";
  }
  function cancel() {
    if (!frozen) return;
    clearFrame();
    endFreeze();
    say(nextHint());
  }

  function setSel(r) {
    const W = desk.offsetWidth, H = desk.offsetHeight;
    const x = clamp(r.x, 0, W), y = clamp(r.y, 0, H);
    const w = clamp(r.w, 0, W - x), h = clamp(r.h, 0, H - y);
    selRect = { x, y, w, h };
    Object.assign(sel.style, { left: `${x}px`, top: `${y}px`, width: `${w}px`, height: `${h}px` });
    sel.classList.add("on");
    const k = 2560 / W;
    dim.textContent = `${Math.round(w * k)} × ${Math.round(h * k)}`;
  }
  function selected() {
    tools.querySelector(".ti-copy").disabled = false;
    say("Press <b>B</b> to find secrets, <b>A</b> for an arrow, <b>X</b> for a box. <b>Enter</b> copies.");
  }
  const rectOf = (el) => {
    const S = stage.getBoundingClientRect(), r = el.getBoundingClientRect();
    return { x: r.left - S.left, y: r.top - S.top, w: r.width, h: r.height };
  };
  function windowAt(x, y) {
    const wins = [...desk.querySelectorAll("[data-win]")].reverse();
    return wins.find((w) => { const r = rectOf(w); return x >= r.x && x <= r.x + r.w && y >= r.y && y <= r.y + r.h; });
  }

  function blurKey() {
    if (!frozen) return;
    const area = selRect ?? deskRect();
    const hits = [...desk.querySelectorAll(".ti-sec")].filter((s) => overlaps(rectOf(s), area));
    if (!hits.length) { toast("No emails, cards or keys in this area"); return; }
    if (blurStage === 0) {
      hits.forEach((s, i) => { s.style.setProperty("--i", i); s.classList.add("found"); });
      blurStage = 1;
      say(`Found ${hits.length}: emails, a card number and an API key. Press <b>B</b> again to blur them.`);
    } else {
      hits.forEach((s) => { s.classList.remove("found"); s.classList.add("blurred"); });
      blurStage = 2;
      quest("blur");
      say(selRect ? "Blurred. The pixels are replaced, not covered. <b>Enter</b> copies." : "Blurred. Now drag over the area you want, then <b>Enter</b>.");
    }
  }

  function undo() {
    const m = marks.pop();
    if (m) m.remove();
  }

  // Pointer work on the frozen desktop: select a region, click a window, or draw.
  let hoverFrame = 0;
  freeze.addEventListener("pointermove", (e) => {
    if (!frozen || tool !== "select" || dragging || e.buttons) return;
    const S = stage.getBoundingClientRect();
    const x = e.clientX - S.left, y = e.clientY - S.top;
    if (hoverFrame) return;
    hoverFrame = requestAnimationFrame(() => {
      hoverFrame = 0;
      const w = windowAt(x, y);
      if (!w || selRect) { hover.classList.remove("on"); return; }
      const r = rectOf(w);
      Object.assign(hover.style, { left: `${r.x}px`, top: `${r.y}px`, width: `${r.w}px`, height: `${r.h}px` });
      hover.dataset.name = w.dataset.win;
      hover.classList.add("on");
    });
  });
  freeze.addEventListener("pointerleave", () => hover.classList.remove("on"));
  freeze.addEventListener("contextmenu", (e) => { e.preventDefault(); undo(); });
  freeze.addEventListener("pointerdown", (e) => {
    if (!frozen || e.button !== 0) return;
    e.preventDefault();
    capture(freeze, e);
    const S = stage.getBoundingClientRect();
    const x0 = e.clientX - S.left, y0 = e.clientY - S.top;
    let x = x0, y = y0, moved = false, frame = 0, mark = null;
    const sw = Math.max(2.5, desk.offsetWidth / 320);

    if (tool === "arrow" || tool === "box") {
      mark = document.createElementNS(NS, "g");
      mark.setAttribute("stroke", "#f0506e");
      mark.setAttribute("stroke-width", sw);
      mark.setAttribute("fill", "none");
      mark.setAttribute("stroke-linecap", "round");
      mark.setAttribute("stroke-linejoin", "round");
      marksSvg.append(mark);
    }
    const draw = () => {
      frame = 0;
      if (mark && tool === "box") {
        const rx = Math.min(x0, x), ry = Math.min(y0, y);
        mark.innerHTML = `<rect x="${rx}" y="${ry}" width="${Math.abs(x - x0)}" height="${Math.abs(y - y0)}" rx="${sw * 1.4}"/>`;
      } else if (mark) {
        const a = Math.atan2(y - y0, x - x0), L = sw * 5;
        const p = (d) => `${x - L * Math.cos(a + d)},${y - L * Math.sin(a + d)}`;
        mark.innerHTML = `<path d="M${x0} ${y0}L${x} ${y}"/><path d="M${p(0.5)}L${x} ${y}L${p(-0.5)}Z" fill="#f0506e"/>`;
      } else {
        hover.classList.remove("on");
        setSel({ x: Math.min(x0, x), y: Math.min(y0, y), w: Math.abs(x - x0), h: Math.abs(y - y0) });
      }
    };
    const move = (ev) => {
      x = ev.clientX - S.left; y = ev.clientY - S.top;
      if (!moved && Math.hypot(x - x0, y - y0) > 5) moved = true;
      if (moved && !frame) frame = requestAnimationFrame(draw);
    };
    const up = () => {
      freeze.removeEventListener("pointermove", move);
      freeze.removeEventListener("pointerup", up);
      freeze.removeEventListener("pointercancel", up);
      cancelAnimationFrame(frame);
      if (mark) {
        if (!moved) { mark.remove(); return; }
        draw();
        marks.push(mark);
        quest("mark");
        say(selRect ? "Nice. <b>Enter</b> copies, right-click undoes." : "Now pick the area: press <b>R</b> and drag, then <b>Enter</b>.");
        return;
      }
      if (moved) {
        draw();
        if (selRect.w < 12 || selRect.h < 12) { sel.classList.remove("on"); selRect = null; return; }
      } else {
        const w = windowAt(x0, y0);
        setSel(w ? rectOf(w) : deskRect());
        toast(w ? `Window: ${w.dataset.win}` : "Whole screen");
      }
      selected();
    };
    freeze.addEventListener("pointermove", move);
    freeze.addEventListener("pointerup", up);
    freeze.addEventListener("pointercancel", up);
  });

  tools.addEventListener("click", (e) => {
    const b = e.target.closest("button");
    if (!b) return;
    if (b.dataset.tool) setTool(b.dataset.tool);
    const act = b.dataset.act;
    if (act === "screen") { setSel(deskRect()); selected(); }
    if (act === "blur") blurKey();
    if (act === "copy") commit();
    if (act === "cancel") cancel();
  });

  async function commit() {
    if (!frozen) return;
    if (!selRect) setSel(deskRect());
    const r = { ...selRect };
    const blurredIn = [...desk.querySelectorAll(".ti-sec.blurred")].some((s) => overlaps(rectOf(s), r));
    const exposed = [...desk.querySelectorAll(".ti-sec:not(.blurred)")].some((s) => overlaps(rectOf(s), r));
    const master = desk.cloneNode(true);
    master.removeAttribute("data-ti");
    master.querySelectorAll("[data-ti]").forEach((n) => n.removeAttribute("data-ti"));
    master.setAttribute("aria-hidden", "true");
    const shot = { id: nextId++, type: "img", r, dw: desk.offsetWidth, dh: desk.offsetHeight, master, meta: { blurred: blurredIn, marks: marks.length > 0, exposed } };

    hover.classList.remove("on");
    sel.classList.remove("on");
    stage.classList.remove("frozen");
    play(flash, [{ opacity: 0 }, { opacity: 0.75, offset: 0.25 }, { opacity: 0 }], { duration: 300, easing: STD });
    setClipboard(shot);
    toast("Copied to clipboard · saved as PNG");

    // Fly the shot up and tuck it under the notch.
    const fly = view(shot, r.w);
    fly.classList.add("ti-flying");
    Object.assign(fly.style, { left: `${r.x}px`, top: `${r.y}px` });
    stage.append(fly);
    clearFrame();
    const S = stage.getBoundingClientRect(), N = nbar.getBoundingClientRect();
    const cx = N.left - S.left + N.width / 2;
    const s1 = clamp(150 / r.w, 0.08, 0.6), s2 = clamp(36 / r.w, 0.03, 0.2);
    const midX = cx - (r.w * s1) / 2 - r.x, midY = N.bottom - S.top + 24 - r.y;
    const endX = cx - (r.w * s2) / 2 - r.x, endY = N.bottom - S.top - r.h * s2 - 3 - r.y;
    endFreeze();
    await play(fly, [
      { transform: "translate(0,0) scale(1)" },
      { transform: `translate(${midX}px, ${midY}px) scale(${s1})`, offset: 0.6 },
      { transform: `translate(${endX}px, ${endY}px) scale(${s2})` },
    ], { duration: 900, easing: EMPH });
    fly.remove();

    shots.unshift(shot);
    known.set(shot.id, shot);
    renderLedge();
    toNav(shot);
    play(notch, [{ transform: "scale(1)" }, { transform: "scale(1.08, 1.25)", offset: 0.35 }, { transform: "scale(1)" }], { duration: 480, easing: STD, fill: "none" });
    quest("capture");
    say(nextHint());
    // Show the ledge for a moment so people learn where shots go.
    setOpen(true, true);
    peekTimer = setTimeout(() => { if (!notchHovered && !dragging) setOpen(false); }, 2200);
  }

  // ── the ledge (notch) ─────────────────────────────────
  let notchHovered = false, openTimer = 0, closeTimer = 0, peekTimer = 0;
  function setOpen(v, peek = false) {
    clearTimeout(peekTimer);
    if (v && frozen) return;
    if (v) sizeLedge();
    notch.classList.toggle("open", v);
    notch.classList.toggle("peek", v && peek);
    nbar.setAttribute("aria-expanded", v);
    tilesEl.querySelectorAll(".ti-tile").forEach((t) => (t.tabIndex = v ? 0 : -1));
  }
  function sizeLedge() {
    const th = tileW() / 1.6;
    notch.style.setProperty("--ti-oh", `${Math.round(th + (shots.length ? 92 : 64))}px`);
  }
  const tileW = () => clamp(stage.clientWidth * 0.15, 84, 124);

  function renderLedge() {
    count.lastElementChild.textContent = shots.length;
    notch.classList.toggle("has", shots.length > 0);
    tilesEl.textContent = "";
    if (!shots.length) {
      tilesEl.innerHTML = `<p class="ti-empty">No shots yet. Press <b>Try a capture</b>.</p>`;
    }
    const tw = tileW(), th = tw / 1.6;
    shots.forEach((shot, i) => {
      const t = document.createElement("div");
      t.className = "ti-tile";
      t.tabIndex = notch.classList.contains("open") ? 0 : -1;
      t.setAttribute("role", "button");
      t.setAttribute("aria-label", `${shot.type === "pdf" ? "PDF" : "Shot"} ${i + 1}. Enter sends it to the chat. Click copies it.`);
      t.style.width = `${tw}px`;
      t.style.height = `${th}px`;
      t.style.setProperty("--i", i);
      const v = view(shot, tw);
      // Center-crop the shot into the tile.
      const vh = parseFloat(v.style.height);
      v.style.marginTop = `${(th - vh) / 2}px`;
      t.append(v);
      t.insertAdjacentHTML("beforeend", `
        <button type="button" class="ti-tick${ticked.has(shot.id) ? " on" : ""}" aria-label="Select for merge">${svgIcon("check")}</button>
        <span class="ti-acts">
          <button type="button" data-a="copy" aria-label="Copy">${svgIcon("copy")}</button>
          <button type="button" data-a="float" aria-label="Float">${svgIcon("float")}</button>
          <button type="button" data-a="del" aria-label="Delete">${svgIcon("trash")}</button>
        </span>`);
      t.querySelector(".ti-tick").addEventListener("click", (e) => {
        e.stopPropagation();
        ticked.has(shot.id) ? ticked.delete(shot.id) : ticked.add(shot.id);
        e.currentTarget.classList.toggle("on", ticked.has(shot.id));
        syncMerge();
      });
      t.querySelector(".ti-acts").addEventListener("click", (e) => {
        const b = e.target.closest("button");
        if (!b) return;
        e.stopPropagation();
        if (b.dataset.a === "copy") copyShot(shot);
        if (b.dataset.a === "float") floatShot(shot);
        if (b.dataset.a === "del") { shots.splice(shots.indexOf(shot), 1); ticked.delete(shot.id); renderLedge(); sizeLedge(); syncMerge(); }
      });
      t.addEventListener("pointerdown", (e) => tileDown(e, t, shot));
      t.addEventListener("keydown", (e) => {
        if (e.target !== t) return;
        if (e.key === "Enter" || e.key === " ") { e.preventDefault(); sendFrom(t, shot); }
      });
      tilesEl.append(t);
    });
    syncMerge();
  }
  function syncMerge() {
    for (const id of ticked) if (!shots.some((s) => s.id === id)) ticked.delete(id);
    const n = ticked.size;
    mergeBtn.disabled = n < 2;
    mergeBtn.lastElementChild.textContent = n >= 2 ? `Merge ${n}` : "Merge";
    lhint.innerHTML = n === 1 ? "Tick one more to merge into a PDF." : n >= 2 ? "Press <b>Merge</b> for one PDF." : "Click a shot to copy it. Drag it into the chat.";
  }
  mergeBtn.addEventListener("click", () => {
    const pages = shots.filter((s) => ticked.has(s.id)).reverse().flatMap((s) => (s.type === "pdf" ? s.pages : [s]));
    if (pages.length < 2) return;
    ticked.clear();
    const pdf = { id: nextId++, type: "pdf", pages, r: pages[0].r };
    shots.unshift(pdf);
    renderLedge();
    sizeLedge();
    quest("pdf");
    toast(`Merged ${pages.length} shots into one PDF`);
    say(nextHint());
  });
  function copyShot(shot) {
    setClipboard(shot);
    toast("Copied. Press Ctrl+V in the chat.");
    say(nextHint());
  }

  // ── the page's nav notch ──────────────────────────────
  // Each capture is also offered to the page's own notch (main.js listens for
  // "ledgegrab:capture"), so the visitor's shots wait at the top of the screen.
  // Clicking one there sends "ledgegrab:copy" back to put it on our clipboard.
  const known = new Map();
  function toNav(shot) {
    const { r } = shot;
    const node = document.createElement("div");
    node.style.cssText = `width:${r.w}px;height:${r.h}px;overflow:hidden;background:#fbfaf7`;
    const c = shot.master.cloneNode(true);
    c.style.cssText = `inset:auto;left:${-r.x}px;top:${-r.y}px;width:${shot.dw}px;height:${shot.dh}px`;
    node.append(c);
    document.dispatchEvent(new CustomEvent("ledgegrab:capture", { detail: { id: shot.id, node, w: r.w, h: r.h } }));
  }
  document.addEventListener("ledgegrab:copy", (e) => {
    const shot = known.get(e.detail?.id);
    if (shot) copyShot(shot);
  });

  nbar.addEventListener("click", () => setOpen(!notch.classList.contains("open")));
  notch.addEventListener("pointerenter", () => {
    notchHovered = true;
    clearTimeout(closeTimer);
    clearTimeout(peekTimer);
    if (!frozen) openTimer = setTimeout(() => setOpen(true), 220);
  });
  notch.addEventListener("pointerleave", () => {
    notchHovered = false;
    clearTimeout(openTimer);
    closeTimer = setTimeout(() => { if (!dragging) setOpen(false); }, 600);
  });

  // ── dragging a tile out ───────────────────────────────
  function ghostFrom(r, shot) {
    const g = document.createElement("div");
    g.className = "ti-ghost";
    const v = view(shot, r.width);
    Object.assign(g.style, { left: `${r.left}px`, top: `${r.top}px`, width: `${r.width}px`, height: v.style.height });
    g.append(v);
    document.body.append(g);
    return g;
  }
  function tileDown(e, t, shot) {
    if (e.button > 0 || e.target.closest("button")) return;
    e.preventDefault();
    const r = t.getBoundingClientRect();
    const x0 = e.clientX, y0 = e.clientY;
    let x = x0, y = y0, px = x0, tilt = 0, frame = 0, over = false, ghost = null;
    capture(t, e);
    const render = () => {
      frame = 0;
      tilt = clamp(tilt * 0.7 + (x - px) * 0.5, -10, 10);
      px = x;
      ghost.style.transform = `translate(${x - x0}px, ${y - y0}px) rotate(${tilt}deg) scale(1.08)`;
      const o = inside(chat.getBoundingClientRect(), x, y);
      if (o !== over) chat.classList.toggle("over", (over = o));
    };
    const move = (ev) => {
      x = ev.clientX; y = ev.clientY;
      if (!ghost && Math.hypot(x - x0, y - y0) > 5) {
        ghost = ghostFrom(r, shot);
        dragging = true;
        t.classList.add("lifted");
        chat.classList.add("armed");
      }
      if (ghost && !frame) frame = requestAnimationFrame(render);
    };
    const end = (ev) => {
      t.removeEventListener("pointermove", move);
      t.removeEventListener("pointerup", end);
      t.removeEventListener("pointercancel", end);
      cancelAnimationFrame(frame);
      chat.classList.remove("over", "armed");
      if (!ghost) { if (ev.type === "pointerup") copyShot(shot); return; }
      dragging = false;
      const dropped = ev.type === "pointerup" && inside(chat.getBoundingClientRect(), ev.clientX, ev.clientY);
      if (dropped) landInChat(ghost, r, shot, t);
      else snapBack(ghost, t);
    };
    t.addEventListener("pointermove", move);
    t.addEventListener("pointerup", end);
    t.addEventListener("pointercancel", end);
  }
  async function snapBack(ghost, t) {
    await play(ghost, [{ transform: getComputedStyle(ghost).transform }, { transform: "none" }], { duration: 520, easing: SPRING });
    ghost.remove();
    t.classList.remove("lifted");
    if (!notchHovered) closeTimer = setTimeout(() => setOpen(false), 600);
  }
  async function landInChat(ghost, r, shot, t) {
    const el = message({ who: "You", c: "#f3ede1", shot });
    const target = el.querySelector(".ti-shot, .ti-file");
    target.style.visibility = "hidden";
    const D = target.getBoundingClientRect();
    await play(ghost, [{ transform: getComputedStyle(ghost).transform, opacity: 1 }, { transform: `translate(${D.left - r.left}px, ${D.top - r.top}px) scale(${D.width / r.width})`, opacity: shot.type === "pdf" ? 0 : 1 }], { duration: 560, easing: EMPH });
    target.style.visibility = "";
    ghost.remove();
    t.classList.remove("lifted");
    setOpen(false);
    quest("share");
    replyTo(shot);
    say(nextHint());
  }
  function sendFrom(t, shot) {
    const r = t.getBoundingClientRect();
    t.classList.add("lifted");
    landInChat(ghostFrom(r, shot), r, shot, t);
  }

  // ── floats ────────────────────────────────────────────
  function floatShot(shot) {
    const page = shot.type === "pdf" ? shot.pages[0] : shot;
    const f = document.createElement("div");
    f.className = "ti-float";
    f.title = "Drag to move · scroll to resize · Ctrl+scroll to fade · right-click to close";
    const w = clamp(stage.clientWidth * 0.3, 140, 260);
    f.append(pageView(page, w));
    let tx = stage.clientWidth / 2 - w / 2 + (floats.children.length * 18), ty = stage.clientHeight * 0.3, s = 1, o = 1;
    const apply = () => { f.style.transform = `translate(${tx}px, ${ty}px) scale(${s})`; f.style.opacity = o; };
    apply();
    floats.append(f);
    play(f, [{ opacity: 0, scale: 0.8 }, { opacity: o, scale: 1 }], { duration: 500, easing: SPRING, fill: "none" });
    f.addEventListener("pointerdown", (e) => {
      if (e.button !== 0) return;
      e.preventDefault();
      capture(f, e);
      const sx = e.clientX - tx, sy = e.clientY - ty;
      let frame = 0, lx = e.clientX, ly = e.clientY;
      const mv = (ev) => { lx = ev.clientX; ly = ev.clientY; if (!frame) frame = requestAnimationFrame(() => { frame = 0; tx = lx - sx; ty = ly - sy; apply(); }); };
      const upf = () => { f.removeEventListener("pointermove", mv); f.removeEventListener("pointerup", upf); };
      f.addEventListener("pointermove", mv);
      f.addEventListener("pointerup", upf);
    });
    f.addEventListener("wheel", (e) => {
      e.preventDefault();
      if (e.ctrlKey) o = clamp(o - Math.sign(e.deltaY) * 0.08, 0.2, 1);
      else s = clamp(s - Math.sign(e.deltaY) * 0.08, 0.5, 2);
      apply();
    }, { passive: false });
    const close = (e) => { e.preventDefault(); play(f, [{ opacity: o }, { opacity: 0 }], { duration: 200, easing: STD }).then(() => f.remove()); };
    f.addEventListener("contextmenu", close);
    f.addEventListener("dblclick", close);
    setOpen(false);
    quest("float");
    toast("Floating above everything. Scroll to resize, right-click to close.");
    say(nextHint());
  }

  // ── keyboard ──────────────────────────────────────────
  document.addEventListener("keydown", (e) => {
    const typing = e.target.closest?.("input, textarea, [contenteditable]");
    if (typing) return;
    const k = e.key.toLowerCase();
    if (e.ctrlKey && e.shiftKey && k === "s" && inView) { e.preventDefault(); startFreeze(); return; }
    if (frozen) {
      if (k === "escape") { e.preventDefault(); cancel(); }
      else if (k === "enter") { e.preventDefault(); commit(); }
      else if ((e.ctrlKey || e.metaKey) && k === "z") { e.preventDefault(); undo(); }
      else if (e.ctrlKey || e.metaKey || e.altKey) return;
      else if (k === "b") { e.preventDefault(); blurKey(); }
      else if (k === "a") { e.preventDefault(); setTool("arrow"); say("Drag to draw an arrow."); }
      else if (k === "x") { e.preventDefault(); setTool("box"); say("Drag to draw a box."); }
      else if (k === "r") { e.preventDefault(); setTool("select"); say("Drag over an area, or click a window."); }
      return;
    }
    if (k === "escape" && notch.classList.contains("open")) setOpen(false);
  });

  go.addEventListener("click", startFreeze);

  renderLedge();
  renderQuests();
  say("Press <b>Try a capture</b>, then drag over anything on this desktop.");
}
