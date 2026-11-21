// Fills the download links from the latest GitHub release. This is the site's only network
// request besides its own files. Without it (no JS, offline, rate-limited) every link still
// points at the Releases page.

const REPO = "vcostin/keywhere";
const RELEASES = `https://github.com/${REPO}/releases/latest`;
const API = `https://api.github.com/repos/${REPO}/releases/latest`;

/** Release asset name patterns (see packaging/release-notes.sh). */
const PATTERNS = {
  appimage: /^Keywhere-.+-x86_64\.AppImage$/,
  deb: /^keywhere_.+_amd64\.deb$/,
  rpm: /^keywhere-.+\.x86_64\.rpm$/,
  "mac-arm64": /^Keywhere-.+-macos-arm64\.dmg$/,
  "mac-x64": /^Keywhere-.+-macos-x64\.dmg$/,
  "win-x64": /^Keywhere-.+-windows-x64-portable\.exe$/,
};

const OS_NAMES = { linux: "Linux", macos: "macOS", windows: "Windows" };
/** The file the big button downloads directly, per OS. macOS needs a choice (chip), so it scrolls. */
const PRIMARY = { linux: "appimage", windows: "win-x64" };

function detectOs() {
  const ua = navigator.userAgent;
  const platform = navigator.userAgentData?.platform || navigator.platform || "";
  if (/Android|iPhone|iPad|iPod/i.test(ua)) return "mobile";
  if (/Win/i.test(platform) || /Windows/i.test(ua)) return "windows";
  if (/Mac/i.test(platform) || /Macintosh|Mac OS X/i.test(ua)) {
    // iPadOS reports itself as a Mac but has touch.
    return navigator.maxTouchPoints > 1 ? "mobile" : "macos";
  }
  if (/Linux|X11|CrOS/i.test(platform + ua)) return "linux";
  return "unknown";
}

function formatBytes(n) {
  if (!Number.isFinite(n) || n <= 0) return "";
  const mb = n / (1024 * 1024);
  return mb >= 10 ? `${Math.round(mb)} MB` : `${mb.toFixed(1)} MB`;
}

function sha256Of(asset) {
  const d = String(asset.digest ?? "");
  return /^sha256:[0-9a-f]{64}$/i.test(d) ? d.slice(7).toLowerCase() : "";
}

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text) node.textContent = text;
  return node;
}

const $ = (id) => document.getElementById(id);
const primary = $("primary-download");
const primaryLabel = $("primary-label");
const primaryMeta = $("primary-meta");
const ctaStatus = $("cta-status");
const dlStatus = $("dl-status");
const dlVersion = $("dl-version");
const sumsLink = $("sums-link");

const os = detectOs();

function markYours() {
  const card = document.querySelector(`.platform[data-os="${os}"]`);
  if (!card) return;
  card.classList.add("is-yours");
  const head = card.querySelector(".platform-head");
  head.append(el("span", "tag tag-ok yours-badge", "Your system"));
}

function setStatus(text, link) {
  const parts = [text];
  if (link) {
    const a = el("a", "", link.text);
    a.href = link.href;
    parts.push(" ", a, ".");
  }
  dlStatus.replaceChildren(...parts);
}

function fallback(message) {
  ctaStatus.textContent = message;
  setStatus(message + " All files are on ", { text: "GitHub Releases", href: RELEASES });
}

async function main() {
  markYours();

  let release;
  try {
    const res = await fetch(API, { headers: { Accept: "application/vnd.github+json" }, referrerPolicy: "no-referrer" });
    if (res.status === 404) return fallback("No release has been published yet.");
    if (!res.ok) return fallback("Couldn't read the latest release from GitHub.");
    release = await res.json();
  } catch {
    return fallback("Couldn't reach GitHub.");
  }

  const tag = String(release.tag_name ?? "").replace(/^v/, "");
  const releaseUrl = typeof release.html_url === "string" ? release.html_url : RELEASES;
  const assets = Array.isArray(release.assets) ? release.assets : [];
  const found = {};
  for (const [key, re] of Object.entries(PATTERNS)) {
    found[key] = assets.find((a) => re.test(String(a.name)));
  }

  if (tag) dlVersion.textContent = ` ${tag}`;
  const date = release.published_at ? new Date(release.published_at) : null;
  const when =
    date && !Number.isNaN(date.valueOf())
      ? date.toLocaleDateString("en", { year: "numeric", month: "long", day: "numeric" })
      : "";
  setStatus(`Version ${tag}${when ? `, released ${when}` : ""}. Release notes and older versions are on `, {
    text: "GitHub Releases",
    href: releaseUrl,
  });

  const sums = assets.find((a) => a.name === "SHA256SUMS");
  if (sums) sumsLink.href = sums.browser_download_url;

  // Fill each file row; mark the ones this release doesn't have.
  for (const li of document.querySelectorAll(".file[data-asset]")) {
    const asset = found[li.dataset.asset];
    const link = li.querySelector(".file-link");
    if (!asset) {
      li.classList.add("is-missing");
      link.href = releaseUrl;
      link.querySelector("[data-name]").textContent = `Not in version ${tag}`;
      continue;
    }
    link.href = asset.browser_download_url;
    link.querySelector("[data-name]").textContent = asset.name;
    const size = formatBytes(asset.size);
    if (size) link.append(el("span", "file-meta", size));
  }

  // Per-platform checksums (GitHub's asset digests), folded away under the file list.
  for (const card of document.querySelectorAll(".platform")) {
    const rows = [...card.querySelectorAll(".file[data-asset]")]
      .map((li) => found[li.dataset.asset])
      .filter((asset) => asset && sha256Of(asset));
    if (!rows.length) continue;
    const details = el("details", "sums");
    details.append(el("summary", "", "SHA-256 checksums"));
    const dl = el("dl");
    for (const asset of rows) dl.append(el("dt", "", asset.name), el("dd", "", sha256Of(asset)));
    details.append(dl);
    card.querySelector(".files").after(details);
  }

  // A platform with no files at all in this release gets a short note.
  for (const card of document.querySelectorAll(".platform")) {
    const rows = card.querySelectorAll(".file[data-asset]");
    if ([...rows].every((r) => r.classList.contains("is-missing"))) {
      const note = el("p", "platform-note", `Version ${tag} has no ${OS_NAMES[card.dataset.os]} build. `);
      const a = el("a", "", "See all releases");
      a.href = "https://github.com/vcostin/keywhere/releases";
      note.append(a, ".");
      card.querySelector(".files").after(note);
    }
  }

  // The big button.
  if (os === "mobile" || os === "unknown") {
    ctaStatus.textContent =
      os === "mobile"
        ? `Keywhere is a desktop app for Linux, macOS and Windows. Latest: ${tag}.`
        : `Latest: ${tag}. Pick your platform below.`;
    return;
  }
  const name = OS_NAMES[os];
  const direct = found[PRIMARY[os]];
  if (direct) {
    primary.href = direct.browser_download_url;
    primaryLabel.textContent = `Download for ${name}`;
    const kind = os === "linux" ? "AppImage" : "Portable .exe";
    primaryMeta.textContent = [kind, tag, formatBytes(direct.size)].filter(Boolean).join(" · ");
    ctaStatus.textContent =
      os === "linux" ? "Prefer a .deb or .rpm? See all downloads below." : "Unsigned preview: see First start below.";
  } else if (os === "macos" && (found["mac-arm64"] || found["mac-x64"])) {
    primary.href = "#dl-macos";
    primaryLabel.textContent = "Download for macOS";
    primaryMeta.textContent = `Apple silicon or Intel · ${tag}`;
    ctaStatus.textContent = "Unsigned preview: see First start below.";
  } else {
    primary.href = "#download";
    primaryLabel.textContent = "See all downloads";
    primaryMeta.textContent = `Version ${tag}`;
    ctaStatus.textContent = `Version ${tag} has no ${name} build.`;
  }
}

main();
