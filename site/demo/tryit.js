// Ledge Grab playground: loader.
// This is the only demo file fetched with the page (~1.5 KB). The playground's code
// (tryit-app.js) and styles (tryit.css) load at idle time once the hero is on screen,
// so first paint and the hero title never wait on them. index.html reserves the
// playground's box, so nothing shifts when it appears.
//
// ?tryit=0 (or a load failure) puts back the original hero stage from
// <template id="tryitFallback">. Notes and removal steps: docs/try-capture-demo.md

const mount = document.getElementById("tryit");
const off = new URLSearchParams(location.search).get("tryit") === "0";

function restore() {
  const t = document.getElementById("tryitFallback");
  if (t) mount.replaceWith(t.content.cloneNode(true));
  else mount.remove();
}

if (mount && off) {
  restore();
} else if (mount) {
  let started = false;
  const start = () => {
    if (started) return;
    started = true;
    const link = document.createElement("link");
    link.rel = "stylesheet";
    link.href = new URL("./tryit.css?v=4", import.meta.url).href;
    const cssReady = new Promise((done, fail) => {
      link.onload = done;
      link.onerror = fail;
    });
    document.head.append(link);
    Promise.all([import("./tryit-app.js?v=4"), cssReady])
      .then(([app]) => app.mount(mount))
      .catch(restore);
  };
  const idle = (fn) => (window.requestIdleCallback ? requestIdleCallback(fn, { timeout: 1200 }) : setTimeout(fn, 150));

  if ("IntersectionObserver" in window) {
    const io = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) {
          io.disconnect();
          idle(start);
        }
      },
      { rootMargin: "400px 0px" },
    );
    io.observe(mount);
  } else {
    idle(start);
  }
}
