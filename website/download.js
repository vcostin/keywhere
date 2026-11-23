// Fills the Download section from the latest GitHub release. This is the site's only network
// request besides its own files. Nothing here names a version or a versioned file: release
// files are sorted into types by their file name pattern, and a file that matches no pattern
// still shows up under "Other files". Without the API (no JS, offline, rate-limited) every
// link points at the Releases page.

const REPO = "vcostin/keywhere";
const RELEASES = `https://github.com/${REPO}/releases`;
const LATEST = `${RELEASES}/latest`;
const API = `https://api.github.com/repos/${REPO}/releases/latest`;

/**
 * Download types, in display order. `test` runs on the file name. The text is about the kind
 * of file, so it stays true from one release to the next (see packaging/release-notes.sh).
 */
const TYPES = [
  {
    id: "appimage",
    os: "linux",
    test: /\.AppImage$/i,
    kind: "AppImage",
    tag: "recommended",
    desc: "Any distribution. Updates itself from the app.",
  },
  {
    id: "deb",
    os: "linux",
    test: /\.deb$/i,
    kind: ".deb",
    desc: "Debian, Ubuntu 22.04+. Install the next .deb to update.",
  },
  {
    id: "rpm",
    os: "linux",
    test: /\.rpm$/i,
    kind: ".rpm",
    desc: "Fedora, openSUSE. Install the next .rpm to update.",
  },
  {
    id: "mac-arm64",
    os: "macos",
    test: /-macos-arm64\.dmg$/i,
    kind: "Apple silicon",
    desc: "M1 and newer. Updates itself from the app.",
  },
  {
    id: "mac-x64",
    os: "macos",
    test: /-macos-x64\.dmg$/i,
    kind: "Intel",
    desc: "Older Intel Macs. Updates itself from the app.",
  },
  {
    id: "win-setup",
    os: "windows",
    test: /-setup\.exe$/i,
    kind: "Installer (.exe)",
    tag: "recommended",
    desc: "Installs for your user, no admin rights, with a Start menu entry and an uninstaller. Updates itself.",
  },
  {
    id: "win-portable",
    os: "windows",
    test: /-portable\.exe$/i,
    kind: "Portable .exe",
    desc: "A single program, nothing to install. Put it anywhere and run it. Updates itself.",
  },
  {
    id: "win-msi",
    os: "windows",
    test: /\.msi$/i,
    kind: "MSI",
    desc: "Installs for all users in Program Files (admin rights). Install the next MSI to update.",
  },
];

/** Files that aren't downloads for people. They're listed under "Other files" with a note. */
const SIDE_FILES = [
  { test: /\.app\.zip$/i, desc: "For the in-app updater on macOS. Download the .dmg instead." },
  { test: /^latest\.json$/i, desc: "Update manifest, read by the in-app updater." },
  { test: /^latest\.json\.minisig$/i, desc: "Signature of the update manifest." },
];
const SUMS_FILE = /^SHA256SUMS$/;

/** The file the big button downloads, per OS, first match wins. macOS needs a choice, so it scrolls. */
const PRIMARY = { linux: ["appimage"], windows: ["win-setup", "win-portable"] };
const OS_NAMES = { linux: "Linux", macos: "macOS", windows: "Windows" };

/** Only links into this repository are used; anything else keeps the static fallback. */
const SAFE_URL = /^https:\/\/github\.com\/vcostin\/keywhere\//;
const safeUrl = (u) => (typeof u === "string" && SAFE_URL.test(u) ? u : null);

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
  if (n < 1024 * 1024) return `${Math.max(1, Math.round(n / 1024))} KB`;
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
const otherFiles = $("other-files");

const os = detectOs();
const cards = [...document.querySelectorAll(".platform[data-os]")];

function markYours() {
  const card = cards.find((c) => c.dataset.os === os);
  if (!card) return;
  card.classList.add("is-yours");
  // The visitor's platform comes first, in the DOM so focus and reading order match the screen.
  card.parentElement.prepend(card);
  card.querySelector(".platform-head").append(el("span", "tag tag-ok yours-badge", "Your system"));
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

/** One download row. Without an asset it describes the type and links to the Releases page. */
function fileRow(type, asset) {
  const li = el("li", "file");
  li.dataset.type = type.id;
  const a = el("a", "file-link");
  a.href = (asset && safeUrl(asset.browser_download_url)) ?? LATEST;
  const kind = el("span", "file-kind", type.kind);
  if (type.tag) kind.append(" ", el("span", "tag tag-accent", type.tag));
  a.append(kind, el("span", "file-desc", type.desc));
  if (asset) {
    a.append(el("span", "file-name", asset.name));
    const size = formatBytes(asset.size);
    if (size) a.append(el("span", "file-meta", size));
  }
  li.append(a);
  return li;
}

/** Shows every known type without file names, all linking to the Releases page. */
function renderGeneric() {
  for (const card of cards) {
    const rows = TYPES.filter((t) => t.os === card.dataset.os).map((t) => fileRow(t, null));
    card.querySelector(".files").replaceChildren(...rows);
  }
}

/**
 * The line under the big button: text plus an in-page link. A link to a platform card also opens
 * that card's install / first-start notes, so the answer is right there after the scroll.
 */
function setCta(text, linkText, target) {
  if (!linkText) return ctaStatus.replaceChildren(text);
  const a = el("a", "", linkText);
  a.href = `#${target}`;
  opensHowto(a, target);
  ctaStatus.replaceChildren(text, a, ".");
}

/** A link to a platform card also opens that card's install / first-start notes. */
function opensHowto(link, target) {
  const card = document.getElementById(target);
  const howto = card?.classList.contains("platform") ? card.querySelector(".howto") : null;
  if (howto) link.addEventListener("click", () => (howto.open = true));
}

function fallback(message) {
  renderGeneric();
  setCta(`${message} `, "See all downloads", "download");
  setStatus(message + " All files are on ", { text: "GitHub Releases", href: LATEST });
}

/** Sorts a release's files into download types, the checksum file and everything else. */
function classify(assets) {
  const byType = new Map(TYPES.map((t) => [t.id, []]));
  const other = [];
  let sums = null;
  for (const asset of assets) {
    const name = String(asset.name ?? "");
    if (SUMS_FILE.test(name)) {
      sums = asset;
      continue;
    }
    const side = SIDE_FILES.find((s) => s.test.test(name));
    const type = side ? null : TYPES.find((t) => t.test.test(name));
    if (type) byType.get(type.id).push(asset);
    else other.push({ asset, desc: side?.desc ?? "" });
  }
  for (const list of byType.values()) list.sort((a, b) => a.name.localeCompare(b.name));
  other.sort((a, b) => a.asset.name.localeCompare(b.asset.name));
  return { byType, other, sums };
}

function renderCards(byType, version) {
  for (const card of cards) {
    const types = TYPES.filter((t) => t.os === card.dataset.os);
    const rows = types.flatMap((t) => byType.get(t.id).map((asset) => fileRow(t, asset)));
    const list = card.querySelector(".files");

    if (!rows.length) {
      const note = el("p", "platform-note", `Version ${version} has no ${OS_NAMES[card.dataset.os]} build. `);
      const a = el("a", "", "See all releases");
      a.href = RELEASES;
      note.append(a, ".");
      list.replaceWith(note);
      continue;
    }
    list.replaceChildren(...rows);

    // This platform's checksums (GitHub's asset digests), folded away under the file list.
    const withSums = types.flatMap((t) => byType.get(t.id)).filter((asset) => sha256Of(asset));
    if (withSums.length) {
      const details = el("details", "sums");
      details.append(el("summary", "", "SHA-256 checksums"));
      const dl = el("dl");
      for (const asset of withSums) dl.append(el("dt", "", asset.name), el("dd", "", sha256Of(asset)));
      details.append(dl);
      list.after(details);
    }
  }
}

function renderOther(other) {
  if (!other.length) return;
  const list = otherFiles.querySelector("ul");
  for (const { asset, desc } of other) {
    const li = el("li");
    const a = el("a", "", asset.name);
    a.href = safeUrl(asset.browser_download_url) ?? LATEST;
    li.append(a);
    const size = formatBytes(asset.size);
    if (size) li.append(el("span", "file-meta", ` ${size}`));
    if (desc) li.append(el("span", "other-desc", desc));
    list.append(li);
  }
  otherFiles.hidden = false;
}

function renderPrimary(byType, version) {
  if (os === "mobile" || os === "unknown") {
    if (os === "mobile") setCta(`Keywhere is a desktop app for Linux, macOS and Windows. Latest: ${version}.`);
    else setCta(`Latest: ${version}. `, "Pick your platform", "download");
    return;
  }
  const name = OS_NAMES[os];
  const pick = (PRIMARY[os] ?? []).map((id) => [TYPES.find((t) => t.id === id), byType.get(id)[0]]).find(([, a]) => a);
  const hasAny = TYPES.some((t) => t.os === os && byType.get(t.id).length);
  if (pick) {
    const [type, asset] = pick;
    primary.href = safeUrl(asset.browser_download_url) ?? LATEST;
    primaryLabel.textContent = `Download for ${name}`;
    primaryMeta.textContent = [type.kind, version, formatBytes(asset.size)].filter(Boolean).join(" · ");
    if (os === "linux") setCta("Prefer a .deb or .rpm? ", "All Linux downloads", "dl-linux");
    else setCta("Unsigned preview: ", "other Windows files and first start", "dl-windows");
  } else if (hasAny) {
    primary.href = `#dl-${os}`;
    opensHowto(primary, `dl-${os}`);
    primaryLabel.textContent = `Download for ${name}`;
    primaryMeta.textContent = os === "macos" ? `Apple silicon or Intel · ${version}` : `Version ${version}`;
    if (os === "linux") setCta("", "All Linux downloads", "dl-linux");
    else setCta("Unsigned preview: ", `${name} files and first start`, `dl-${os}`);
  } else {
    primary.href = "#download";
    primaryLabel.textContent = "See all downloads";
    primaryMeta.textContent = `Version ${version}`;
    setCta(`Version ${version} has no ${name} build. `, "See all downloads", "download");
  }
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

  const version = String(release?.tag_name ?? "").replace(/^v/, "");
  if (!/^\d+\.\d+\.\d+$/.test(version)) return fallback("Couldn't read the latest release from GitHub.");
  const releaseUrl = safeUrl(release.html_url) ?? LATEST;
  const assets = (Array.isArray(release.assets) ? release.assets : []).filter(
    (a) => a && typeof a.name === "string" && safeUrl(a.browser_download_url),
  );
  const { byType, other, sums } = classify(assets);

  dlVersion.textContent = ` ${version}`;
  const date = release.published_at ? new Date(release.published_at) : null;
  const when =
    date && !Number.isNaN(date.valueOf())
      ? date.toLocaleDateString("en", { year: "numeric", month: "long", day: "numeric" })
      : "";
  setStatus(`Version ${version}${when ? `, released ${when}` : ""}. Release notes and older versions are on `, {
    text: "GitHub Releases",
    href: releaseUrl,
  });
  if (sums) sumsLink.href = safeUrl(sums.browser_download_url) ?? LATEST;

  renderCards(byType, version);
  renderOther(other);
  renderPrimary(byType, version);
}

main();
