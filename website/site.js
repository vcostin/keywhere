// Theme switch, screenshot tabs and copy buttons. No dependencies.

const root = document.documentElement;
const media = window.matchMedia("(prefers-color-scheme: dark)");

/* ── Theme: system / dark / light ─────────────────────── */
function currentPref() {
  return root.getAttribute("data-theme-pref") || "system";
}

function applyTheme(pref) {
  const dark = pref === "dark" || (pref === "system" && media.matches);
  root.setAttribute("data-theme", dark ? "dark" : "light");
  root.setAttribute("data-theme-pref", pref);
  // Browser chrome colour follows the resolved theme, not just the system setting.
  for (const meta of document.querySelectorAll('meta[name="theme-color"]')) {
    meta.content = dark ? "#0A1631" : "#F3F6FB";
  }
  for (const btn of document.querySelectorAll("[data-theme-choice]")) {
    btn.setAttribute("aria-pressed", String(btn.dataset.themeChoice === pref));
  }
}

for (const btn of document.querySelectorAll("[data-theme-choice]")) {
  btn.addEventListener("click", () => {
    const pref = btn.dataset.themeChoice;
    try {
      if (pref === "system") localStorage.removeItem("kw-theme");
      else localStorage.setItem("kw-theme", pref);
    } catch {
      /* storage blocked: the choice lasts for this page view */
    }
    applyTheme(pref);
  });
}
media.addEventListener("change", () => {
  if (currentPref() === "system") applyTheme("system");
});
applyTheme(currentPref());

/* ── Screenshot tabs (WAI-ARIA tabs pattern) ──────────── */
const tabs = [...document.querySelectorAll('.gallery-tabs [role="tab"]')];

function selectTab(tab, focus) {
  for (const t of tabs) {
    const on = t === tab;
    t.setAttribute("aria-selected", String(on));
    t.tabIndex = on ? 0 : -1;
    document.getElementById(t.getAttribute("aria-controls")).hidden = !on;
  }
  if (focus) tab.focus();
}

tabs.forEach((tab, i) => {
  tab.addEventListener("click", () => selectTab(tab, false));
  tab.addEventListener("keydown", (e) => {
    let next = null;
    if (e.key === "ArrowRight") next = tabs[(i + 1) % tabs.length];
    else if (e.key === "ArrowLeft") next = tabs[(i - 1 + tabs.length) % tabs.length];
    else if (e.key === "Home") next = tabs[0];
    else if (e.key === "End") next = tabs[tabs.length - 1];
    if (next) {
      e.preventDefault();
      selectTab(next, true);
    }
  });
});

/* ── Copy buttons on command snippets ─────────────────── */
const COPY_ICON =
  '<svg viewBox="0 0 20 20" aria-hidden="true"><rect x="7" y="7" width="10" height="10" rx="2"/><path d="M13 7V5a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v6a2 2 0 0 0 2 2h2"/></svg>';
const DONE_ICON = '<svg viewBox="0 0 20 20" aria-hidden="true"><path d="M4 10.5l4 4 8-9"/></svg>';

if (navigator.clipboard) {
  for (const pre of document.querySelectorAll("pre.cmd")) {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "copy-btn";
    btn.setAttribute("aria-label", "Copy command");
    btn.innerHTML = COPY_ICON;
    btn.addEventListener("click", async () => {
      try {
        await navigator.clipboard.writeText(pre.querySelector("code").textContent);
        btn.classList.add("is-done");
        btn.innerHTML = DONE_ICON;
        btn.setAttribute("aria-label", "Copied");
        setTimeout(() => {
          btn.classList.remove("is-done");
          btn.innerHTML = COPY_ICON;
          btn.setAttribute("aria-label", "Copy command");
        }, 1600);
      } catch {
        /* clipboard refused: leave the text selectable */
      }
    });
    pre.append(btn);
  }
}
