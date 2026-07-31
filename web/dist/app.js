const state = {
  user: null,
  settings: null,
  taxonomy: [],
  users: [],
  currentWork: null,
  book: null,
  rendition: null,
  readerWorkId: null,
};

async function api(path, opts = {}) {
  const headers = Object.assign({}, opts.headers || {});
  if (opts.json) {
    headers["Content-Type"] = "application/json";
    opts.body = JSON.stringify(opts.json);
    delete opts.json;
  }
  const res = await fetch(path, { ...opts, headers, credentials: "include" });
  if (!res.ok) {
    const text = await res.text();
    throw new Error(text || res.statusText);
  }
  if (res.status === 204) return null;
  const ct = res.headers.get("content-type") || "";
  if (ct.includes("application/json")) return res.json();
  return res;
}

function show(view) {
  document.querySelectorAll(".view").forEach((el) => (el.hidden = true));
  const el = document.getElementById(`view-${view}`);
  if (el) el.hidden = false;
  document.querySelectorAll("button.nav").forEach((b) => {
    b.classList.toggle("active", b.dataset.view === view);
  });
}

function escapeHtml(s) {
  return String(s)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function taxonomyLabel(code) {
  const n = state.taxonomy.find((t) => t.code === code);
  return n ? `${code} — ${n.name}` : String(code ?? "—");
}

async function boot() {
  try {
    state.user = await api("/api/auth/me");
    await enterApp();
  } catch {
    document.getElementById("login-view").hidden = false;
    document.getElementById("main-view").hidden = true;
  }
}

async function enterApp() {
  document.getElementById("login-view").hidden = true;
  document.getElementById("main-view").hidden = false;
  document.getElementById("who").textContent = state.user.username;
  document.getElementById("nav-admin").hidden = !state.user.is_admin;
  show("library");
  try {
    state.settings = await api("/api/settings");
  } catch {
    state.settings = { show_audio_gaps: true, reader_infinite_scroll: false };
  }
  try {
    state.taxonomy = await api("/api/taxonomy");
  } catch {
    state.taxonomy = [];
  }
  if (state.user.is_admin) {
    try {
      state.users = await api("/api/users");
    } catch {
      state.users = [];
    }
  }
  try {
    await loadWorks();
  } catch (e) {
    document.getElementById("work-list").innerHTML =
      `<p class="empty error">Could not load library: ${escapeHtml(e.message || String(e))}</p>`;
  }
}

document.getElementById("login-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const fd = new FormData(e.target);
  const err = document.getElementById("login-error");
  err.hidden = true;
  try {
    const data = await api("/api/auth/login", {
      method: "POST",
      json: { username: fd.get("username"), password: fd.get("password") },
    });
    state.user = data.user;
    await enterApp();
  } catch {
    err.textContent = "Login failed";
    err.hidden = false;
    document.getElementById("login-view").hidden = false;
    document.getElementById("main-view").hidden = true;
  }
});

document.getElementById("logout").addEventListener("click", async () => {
  await api("/api/auth/logout", { method: "POST" });
  location.reload();
});

document.querySelectorAll("button.nav").forEach((b) => {
  b.addEventListener("click", async () => {
    const v = b.dataset.view;
    show(v);
    if (v === "library") await loadWorks();
    if (v === "currently-reading") await loadCurrentlyReading();
    if (v === "to-read") await loadToRead();
    if (v === "wishlist") await loadWishlist();
    if (v === "attention") await loadAttention();
    if (v === "admin") await loadAdmin();
    if (v === "settings") fillSettings();
  });
});

async function loadWorks() {
  const status = document.getElementById("filter-status").value;
  const q = status ? `?status=${encodeURIComponent(status)}` : "";
  const works = await api(`/api/works${q}`);
  renderCards(document.getElementById("work-list"), works, {
    emptyTitle: "Your library is empty",
    emptyHint: "Import an EPUB, or add something to your wishlist.",
  });
}

document.getElementById("filter-status").addEventListener("change", loadWorks);

async function loadCurrentlyReading() {
  const works = await api("/api/works?status=reading");
  const cap = document.getElementById("currently-reading-cap");
  if (cap) cap.textContent = `${works.length} / 3 slots`;
  renderCards(document.getElementById("currently-reading-list"), works, {
    emptyTitle: "Nothing in progress",
    emptyHint: "Set a book’s status to Reading (max 3) from its detail page.",
  });
}

async function loadToRead() {
  const works = await api("/api/works?status=to_read");
  const cap = document.getElementById("to-read-cap");
  if (cap) cap.textContent = `${works.length} / 9 slots`;
  renderCards(document.getElementById("to-read-list"), works, {
    emptyTitle: "To Read is empty",
    emptyHint: "Promote books here from Library (status: To read, max 9).",
  });
}

async function loadWishlist() {
  const works = await api("/api/works?status=wishlist");
  renderCards(document.getElementById("wishlist-list"), works, {
    emptyTitle: "No wishlist items",
    emptyHint: "Add a title, ISBN, or scan a barcode above.",
  });
}

async function loadAttention() {
  const att = document.getElementById("attention-filter").value;
  const works = await api(`/api/works?attention=${encodeURIComponent(att)}`);
  renderCards(document.getElementById("attention-list"), works, {
    emptyTitle: "Nothing in this attention queue",
    emptyHint: "Flags appear here for review, covers, StoryGraph, audio gaps, etc.",
  });
}

document.getElementById("attention-filter").addEventListener("change", loadAttention);

function renderCards(el, works, empty = {}) {
  el.innerHTML = "";
  if (!works.length) {
    el.innerHTML = `<div class="empty">
      <strong>${escapeHtml(empty.emptyTitle || "Nothing here")}</strong>
      <p>${escapeHtml(empty.emptyHint || "")}</p>
    </div>`;
    return;
  }
  for (const w of works) {
    const card = document.createElement("div");
    card.className = "card";
    const badges = [];
    if (w.status && w.status !== "unread") {
      const statusLabel = { to_read: "to read", reading: "reading", read: "read", wishlist: "wishlist" }[w.status] || w.status;
      badges.push(statusLabel);
    }
    if (w.primary_code) badges.push(taxonomyLabel(w.primary_code).split(" — ")[1] || `#${w.primary_code}`);
    if (w.needs_review) badges.push("review");
    if (state.settings?.show_audio_gaps && w.needs_audio) badges.push("no audio");
    if (w.is_manga) badges.push("manga");
    card.innerHTML = `
      <img src="/api/works/${w.id}/cover?v=${encodeURIComponent(w.updated_at || "")}" alt="" loading="lazy" onerror="this.style.opacity=0.25" />
      <div class="meta">
        <strong>${escapeHtml(w.title)}</strong>
        <span>${escapeHtml(w.authors || "")}</span>
        <div>${badges.map((b) => `<span class="badge">${escapeHtml(b)}</span>`).join("")}</div>
      </div>`;
    card.addEventListener("click", () => openDetail(w.id));
    el.appendChild(card);
  }
}

/* —— Import / wishlist actions —— */
const importFile = document.getElementById("library-import-file");
const importStatus = document.getElementById("import-status");

function setImportStatus(text, isError = false) {
  importStatus.textContent = text || "";
  importStatus.classList.toggle("error", !!isError);
}

async function importLibraryFile(file) {
  if (!file) return;
  setImportStatus(`Uploading ${file.name}…`);
  const fd = new FormData();
  fd.append("file", file);
  try {
    const res = await fetch("/api/library/import", {
      method: "POST",
      body: fd,
      credentials: "include",
    });
    if (!res.ok) throw new Error(await res.text());
    const j = await res.json();
    setImportStatus(`Importing “${j.work.title}”…`);
    await pollJob(j.job_id, async (job) => {
      if (job.status === "failed") {
        setImportStatus(`Import failed: ${job.detail || "unknown error"}`, true);
        return;
      }
      setImportStatus(`Imported “${j.work.title}”. Opening…`);
      await loadWorks();
      await openDetail(j.work.id);
      setImportStatus("");
    });
  } catch (e) {
    setImportStatus(e.message || String(e), true);
  }
}

importFile.addEventListener("change", (e) => {
  const file = e.target.files?.[0];
  importLibraryFile(file);
  e.target.value = "";
});

document.getElementById("btn-add-wishlist").addEventListener("click", () => {
  show("wishlist");
  loadWishlist();
  document.getElementById("wl-isbn")?.focus();
});

function taxonomyOptions(selected) {
  const popular = [8201, 8403, 8920, 8940, 6100, 9200, 3100];
  const codes = new Map(state.taxonomy.map((t) => [t.code, t]));
  let html = `<option value="">— none —</option>`;
  for (const c of popular) {
    const t = codes.get(c);
    if (t) {
      html += `<option value="${c}" ${selected === c ? "selected" : ""}>${escapeHtml(taxonomyLabel(c))}</option>`;
    }
  }
  html += `<option disabled>────────</option>`;
  for (const t of state.taxonomy) {
    if (popular.includes(t.code)) continue;
    html += `<option value="${t.code}" ${selected === t.code ? "selected" : ""}>${escapeHtml(taxonomyLabel(t.code))}</option>`;
  }
  return html;
}

async function openDetail(id) {
  const data = await api(`/api/works/${id}`);
  state.currentWork = data;
  show("detail");
  const w = data.work;
  const assets = data.assets || [];
  const hasEpub = assets.some((a) => a.kind === "epub");
  const hasMd = assets.some((a) => a.kind === "markdown");
  const audio = assets.find((a) => a.kind === "audio");
  const codes = data.codes || [];

  document.getElementById("detail").innerHTML = `
    <img src="/api/works/${w.id}/cover?t=${Date.now()}" alt="" />
    <div>
      <form id="detail-form" class="form-grid detail-form">
        <label>Title <input name="title" value="${escapeHtml(w.title)}" required /></label>
        <label>Authors <input name="authors" value="${escapeHtml(w.authors || "")}" /></label>
        <label>ISBN <input name="isbn" value="${escapeHtml(w.isbn || "")}" /></label>
        <label>Status
          <select name="status">
            ${[
              ["unread", "Unread"],
              ["to_read", "To read"],
              ["reading", "Currently reading"],
              ["read", "Read"],
              ["wishlist", "Wishlist"],
            ].map(([s, label]) =>
              `<option value="${s}" ${w.status === s ? "selected" : ""}>${label}</option>`).join("")}
          </select>
        </label>
        <label>Primary taxonomy
          <select name="primary_code">${taxonomyOptions(w.primary_code)}</select>
        </label>
        <label>Extra codes (comma-separated)
          <input name="extra_codes" value="${escapeHtml(codes.filter((c) => c !== w.primary_code).join(", "))}" placeholder="8940, 8212" />
        </label>
        <label>Year reading list
          <input name="year_list" type="number" value="${w.year_list ?? ""}" placeholder="${new Date().getFullYear()}" />
        </label>
        <label class="check"><input type="checkbox" name="is_manga" ${w.is_manga ? "checked" : ""} /> Manga / RTL</label>
        <div class="row">
          <button type="submit">Save metadata</button>
        </div>
      </form>
      <p class="muted">${hasEpub ? "EPUB ready" : "No EPUB yet"} · ${hasMd ? "Markdown ready" : "No Markdown"} · Direction: ${escapeHtml(w.reading_direction)}</p>
      <div class="actions">
        ${hasEpub || hasMd ? `<button type="button" id="btn-read">Read</button>` : ""}
        <button type="button" id="btn-refresh-meta">Refresh metadata</button>
        <label class="btn-file">Replace / import file <input type="file" id="import-file" accept=".epub,.pdf,.md,.markdown" hidden /></label>
        ${w.needs_review ? `<button type="button" id="btn-confirm">Confirm → EPUB</button>` : ""}
        ${hasEpub ? `<a href="/api/works/${w.id}/download/epub"><button type="button">Download EPUB</button></a>` : ""}
        ${hasMd ? `<a href="/api/works/${w.id}/download/markdown"><button type="button">Download MD</button></a>` : ""}
        <label class="btn-file">Upload cover <input type="file" id="cover-file" accept="image/*" hidden /></label>
        ${state.user.is_admin ? `
          <label>Grant access
            <select id="grant-user">
              <option value="">Select user…</option>
              ${state.users.filter((u) => !u.is_admin).map((u) =>
                `<option value="${escapeHtml(u.username)}">${escapeHtml(u.username)}</option>`).join("")}
            </select>
          </label>
          <button type="button" id="btn-grant">Grant</button>
        ` : ""}
      </div>
      <pre id="detail-msg"></pre>
    </div>`;

  const msg = (t, isError = false) => {
    const el = document.getElementById("detail-msg");
    el.textContent = t || "";
    el.classList.toggle("error", !!isError);
  };

  document.getElementById("detail-form").addEventListener("submit", async (e) => {
    e.preventDefault();
    const fd = new FormData(e.target);
    const primary = fd.get("primary_code") ? Number(fd.get("primary_code")) : null;
    const extra = String(fd.get("extra_codes") || "")
      .split(",")
      .map((s) => Number(s.trim()))
      .filter((n) => Number.isFinite(n) && n > 0);
    const allCodes = [...new Set([...(primary ? [primary] : []), ...extra])];
    const yearRaw = String(fd.get("year_list") || "").trim();
    await api(`/api/works/${w.id}`, {
      method: "PUT",
      json: {
        title: fd.get("title"),
        authors: fd.get("authors"),
        isbn: fd.get("isbn") || null,
        status: fd.get("status"),
        primary_code: primary,
        year_list: yearRaw ? Number(yearRaw) : null,
        is_manga: fd.get("is_manga") === "on",
        reading_direction: fd.get("is_manga") === "on" ? "rtl" : "ltr",
      },
    });
    await api(`/api/works/${w.id}/codes`, {
      method: "PUT",
      json: { codes: allCodes, primary_code: primary },
    });
    msg("Saved");
    await openDetail(w.id);
  });

  document.getElementById("btn-read")?.addEventListener("click", () => openReader(w, hasEpub, hasMd, audio));
  document.getElementById("btn-refresh-meta")?.addEventListener("click", async () => {
    msg("Refreshing from EPUB / ISBN…");
    try {
      await api(`/api/works/${w.id}/refresh-metadata`, { method: "POST" });
      msg("Metadata refreshed");
      await openDetail(w.id);
    } catch (e) {
      msg(e.message || String(e), true);
    }
  });
  document.getElementById("import-file")?.addEventListener("change", async (e) => {
    const file = e.target.files[0];
    if (!file) return;
    msg(`Importing ${file.name}…`);
    const fd = new FormData();
    fd.append("file", file);
    const res = await fetch(`/api/works/${w.id}/import`, { method: "POST", body: fd, credentials: "include" });
    if (!res.ok) {
      msg(await res.text(), true);
      return;
    }
    const j = await res.json();
    await pollJob(j.job_id, async (job) => {
      if (job.status === "failed") msg(job.detail || "Import failed", true);
      else {
        msg("Import complete");
        await openDetail(w.id);
      }
    });
  });
  document.getElementById("btn-confirm")?.addEventListener("click", async () => {
    const j = await api(`/api/works/${w.id}/confirm`, { method: "POST" });
    msg("Building EPUB…");
    await pollJob(j.job_id, async (job) => {
      if (job.status === "failed") msg(job.detail || "Failed", true);
      else await openDetail(w.id);
    });
  });
  document.getElementById("cover-file")?.addEventListener("change", async (e) => {
    const file = e.target.files[0];
    if (!file) return;
    const fd = new FormData();
    fd.append("file", file);
    await fetch(`/api/works/${w.id}/cover`, { method: "POST", body: fd, credentials: "include" });
    await openDetail(w.id);
  });
  document.getElementById("btn-grant")?.addEventListener("click", async () => {
    const username = document.getElementById("grant-user").value;
    if (!username) {
      msg("Pick a user", true);
      return;
    }
    await api(`/api/works/${w.id}/grants`, { method: "POST", json: { username } });
    msg(`Granted to ${username}`);
  });
}

document.getElementById("back-library").addEventListener("click", () => {
  show("library");
  loadWorks();
});

async function pollJob(id, done) {
  for (let i = 0; i < 120; i++) {
    await new Promise((r) => setTimeout(r, 500));
    const j = await api(`/api/jobs/${id}`);
    if (j.status === "done" || j.status === "failed") {
      await done(j);
      return j;
    }
  }
  await done({ status: "failed", detail: "Timed out waiting for import job" });
}

async function openReader(w, hasEpub, hasMd, audio) {
  state.readerWorkId = w.id;
  state.readerHasEpub = !!hasEpub;
  state.readerHasMd = !!hasMd;
  show("reader");
  closeReaderMenu();
  document.getElementById("reader-title").textContent = w.title;
  setReaderProgress("");
  const preferScroll = !!state.settings?.reader_infinite_scroll && hasMd;
  const useMd = preferScroll || (!hasEpub && hasMd);
  setInfiniteScrollActive(useMd);
  setReaderChrome(useMd);
  document.getElementById("epub-area").hidden = useMd;
  document.getElementById("md-area").hidden = !useMd;

  const audioEl = document.getElementById("audio-player");
  if (audio) {
    audioEl.hidden = false;
    audioEl.src = `/api/works/${w.id}/audio/${audio.relative_path.split("/").pop()}`;
  } else {
    audioEl.hidden = true;
  }

  if (useMd) await loadMarkdown(w.id);
  else if (hasEpub) await loadEpub(w);
}

function isInfiniteScrollActive() {
  return document.getElementById("reader-scroll")?.getAttribute("aria-pressed") === "true";
}

function setInfiniteScrollActive(on) {
  const btn = document.getElementById("reader-scroll");
  if (!btn) return;
  btn.classList.toggle("active", !!on);
  btn.setAttribute("aria-pressed", on ? "true" : "false");
}

function setReaderChrome(infiniteScroll) {
  document.getElementById("reader-prev").hidden = !!infiniteScroll;
  document.getElementById("reader-next").hidden = !!infiniteScroll;
}

function setReaderProgress(text) {
  const el = document.getElementById("reader-progress");
  if (el) el.textContent = text || "";
}

function closeReaderMenu() {
  const menu = document.getElementById("reader-menu");
  const btn = document.getElementById("reader-menu-btn");
  if (menu) menu.hidden = true;
  if (btn) btn.setAttribute("aria-expanded", "false");
}

function toggleReaderMenu() {
  const menu = document.getElementById("reader-menu");
  const btn = document.getElementById("reader-menu-btn");
  const open = !!menu.hidden;
  menu.hidden = !open;
  btn.setAttribute("aria-expanded", open ? "true" : "false");
}

async function applyReaderMode(useMd) {
  const hasMd = !!state.readerHasMd;
  const hasEpub = !!state.readerHasEpub;
  if (useMd && !hasMd) return;
  if (!useMd && !hasEpub) return;
  setInfiniteScrollActive(useMd);
  setReaderChrome(useMd);
  if (useMd) {
    if (state.book) {
      try { state.book.destroy(); } catch {}
      state.book = null;
      state.rendition = null;
    }
    document.getElementById("epub-area").hidden = true;
    document.getElementById("md-area").hidden = false;
    await loadMarkdown(state.readerWorkId);
  } else {
    document.getElementById("md-area").hidden = true;
    document.getElementById("epub-area").hidden = false;
    const w = state.currentWork?.work;
    if (w) await loadEpub(w);
  }
}

function updateMarkdownProgressUi(area) {
  const percent =
    area.scrollHeight <= area.clientHeight
      ? 100
      : (area.scrollTop / (area.scrollHeight - area.clientHeight)) * 100;
  setReaderProgress(`${Math.round(percent)}% read`);
  return percent;
}

function updateEpubProgressUi(loc) {
  if (!state.book || !loc?.start) return;
  try {
    if (state.book.locations && state.book.locations.length && state.book.locations.length() > 0) {
      const total = state.book.locations.length();
      const current = Math.min(
        total,
        Math.max(1, state.book.locations.locationFromCfi(loc.start.cfi) + 1),
      );
      setReaderProgress(`Page ${current} of ${total}`);
      return;
    }
  } catch {}
  const pct = Math.round((loc.start.percentage || 0) * 100);
  setReaderProgress(`${pct}%`);
}

async function loadMarkdown(id) {
  const res = await fetch(`/api/works/${id}/content/markdown`, { credentials: "include" });
  document.getElementById("md-area").textContent = await res.text();
  const area = document.getElementById("md-area");
  const prog = await api(`/api/works/${id}/progress?mode=markdown`).catch(() => null);
  if (prog?.percent) {
    area.scrollTop = (prog.percent / 100) * (area.scrollHeight - area.clientHeight);
  }
  updateMarkdownProgressUi(area);
  area.onscroll = () => {
    const percent = updateMarkdownProgressUi(area);
    api(`/api/works/${id}/progress`, {
      method: "PUT",
      json: { mode: "markdown", position: String(area.scrollTop), percent },
    }).catch(() => {});
  };
}

async function loadEpub(w) {
  const area = document.getElementById("epub-area");
  area.innerHTML = "";
  setReaderProgress("");
  if (state.book) {
    try { state.book.destroy(); } catch {}
    state.book = null;
    state.rendition = null;
  }
  if (typeof JSZip === "undefined") {
    area.textContent = "JSZip failed to load; cannot open EPUB.";
    return;
  }
  if (typeof ePub === "undefined") {
    area.textContent = "epub.js failed to load.";
    return;
  }
  try {
    const res = await fetch(`/api/works/${w.id}/content/epub`, { credentials: "include" });
    if (!res.ok) {
      area.textContent = `Failed to load EPUB (${res.status}). Try re-importing or downloading the file.`;
      return;
    }
    const buf = await res.arrayBuffer();
    // Pass ArrayBuffer so epub.js opens as binary (blob: URLs lack .epub and are
    // mis-detected as directories). Requires JSZip loaded before epub.js in index.html.
    state.book = ePub(buf);
    // Force layout after unhiding reader so flex #epub-area has real size.
    void area.offsetHeight;
    const width = Math.max(area.clientWidth || 0, window.innerWidth || 320);
    const height = Math.max(area.clientHeight || 0, (window.innerHeight || 480) - 56);
    state.rendition = state.book.renderTo(area, {
      width,
      height,
      flow: "paginated",
      allowScriptedContent: false,
    });
    // Dark UI: near-white text with a light orange tint (matches Diarch accent warmth).
    const ink = "#f3e6d4";
    const pageBg = "#0c1210";
    try {
      state.rendition.themes.default({
        body: {
          color: `${ink} !important`,
          background: `${pageBg} !important`,
        },
        "p, div, span, li, td, th, h1, h2, h3, h4, h5, h6, blockquote, pre, code": {
          color: `${ink} !important`,
        },
        a: { color: "#e8b87a !important" },
      });
    } catch {}
    const rtl = w.reading_direction === "rtl" || w.is_manga;
    if (rtl) {
      state.book.ready.then(() => {
        try { state.rendition.themes.default({ body: { direction: "rtl", color: `${ink} !important`, background: `${pageBg} !important` } }); } catch {}
      });
    }
    await state.book.ready;
    // Build page locations in the background for "Page X of Y".
    state.book.locations
      .generate(1024)
      .then(() => {
        try {
          const loc = state.rendition.currentLocation();
          if (loc) updateEpubProgressUi(loc);
        } catch {}
      })
      .catch(() => {});
    const prog = await api(`/api/works/${w.id}/progress?mode=epub`).catch(() => null);
    if (prog?.position) await state.rendition.display(prog.position);
    else await state.rendition.display();
    state.rendition.on("relocated", (loc) => {
      updateEpubProgressUi(loc);
      api(`/api/works/${w.id}/progress`, {
        method: "PUT",
        json: { mode: "epub", position: loc.start.cfi, percent: loc.start.percentage * 100 },
      }).catch(() => {});
    });
  } catch (e) {
    area.textContent = `EPUB render failed: ${e.message || e}`;
  }
}

function readerIsOpen() {
  return !document.getElementById("view-reader")?.hidden;
}

function readerTurn(dir) {
  if (!state.rendition || isInfiniteScrollActive()) return;
  const w = state.currentWork?.work;
  const rtl = w && (w.reading_direction === "rtl" || w.is_manga);
  // Physical left/right: for RTL, left advances; for LTR, right advances.
  if (dir === "left") {
    if (rtl) state.rendition.next();
    else state.rendition.prev();
  } else {
    if (rtl) state.rendition.prev();
    else state.rendition.next();
  }
}

function closeReaderView() {
  closeReaderMenu();
  if (state.book) {
    try { state.book.destroy(); } catch {}
    state.book = null;
    state.rendition = null;
  }
  setReaderProgress("");
  if (state.readerWorkId) openDetail(state.readerWorkId);
  else {
    show("library");
    loadWorks();
  }
}

document.getElementById("reader-menu-btn").addEventListener("click", (e) => {
  e.stopPropagation();
  toggleReaderMenu();
});

document.getElementById("reader-scroll").addEventListener("click", async () => {
  const next = !isInfiniteScrollActive();
  closeReaderMenu();
  await applyReaderMode(next);
});

document.getElementById("reader-close").addEventListener("click", () => {
  closeReaderView();
});

document.getElementById("reader-prev").addEventListener("click", () => readerTurn("left"));
document.getElementById("reader-next").addEventListener("click", () => readerTurn("right"));

document.addEventListener("click", (e) => {
  const wrap = document.querySelector(".reader-menu-wrap");
  if (!wrap || wrap.contains(e.target)) return;
  closeReaderMenu();
});

document.addEventListener("keydown", (e) => {
  if (!readerIsOpen()) return;
  if (e.target && (e.target.tagName === "INPUT" || e.target.tagName === "TEXTAREA" || e.target.isContentEditable)) {
    return;
  }
  if (e.key === "Escape") {
    closeReaderMenu();
    return;
  }
  if (e.key === "ArrowLeft") {
    e.preventDefault();
    readerTurn("left");
  } else if (e.key === "ArrowRight") {
    e.preventDefault();
    readerTurn("right");
  }
});

document.getElementById("wishlist-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const fd = new FormData(e.target);
  const primary = fd.get("primary_code");
  await api("/api/wishlist", {
    method: "POST",
    json: {
      isbn: fd.get("isbn") || null,
      title: fd.get("title") || null,
      authors: fd.get("authors") || null,
      primary_code: primary ? Number(primary) : null,
      codes: primary ? [Number(primary)] : [],
    },
  });
  e.target.reset();
  await loadWishlist();
});

document.getElementById("btn-scan").addEventListener("click", async () => {
  const video = document.getElementById("scan-video");
  video.hidden = false;
  const stream = await navigator.mediaDevices.getUserMedia({ video: { facingMode: "environment" } });
  video.srcObject = stream;
  await video.play();
  const codeReader = new ZXing.BrowserMultiFormatReader();
  try {
    const result = await codeReader.decodeOnceFromVideoDevice(undefined, "scan-video");
    document.getElementById("wl-isbn").value = result.text;
  } catch {
    alert("Scan failed or cancelled");
  }
  stream.getTracks().forEach((t) => t.stop());
  video.hidden = true;
});

function fillSettings() {
  const f = document.getElementById("settings-form");
  f.show_audio_gaps.checked = !!state.settings?.show_audio_gaps;
  f.reader_infinite_scroll.checked = !!state.settings?.reader_infinite_scroll;
}

document.getElementById("settings-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const f = e.target;
  await api("/api/settings", {
    method: "PUT",
    json: {
      show_audio_gaps: f.show_audio_gaps.checked,
      reader_infinite_scroll: f.reader_infinite_scroll.checked,
    },
  });
  state.settings = await api("/api/settings");
  alert("Saved");
});

async function loadAdmin() {
  if (!state.user.is_admin) return;
  state.users = await api("/api/users");
  const ul = document.getElementById("user-list");
  ul.innerHTML = state.users
    .map(
      (u) =>
        `<li><strong>${escapeHtml(u.username)}</strong>${u.is_admin ? " · admin" : " · reader"}</li>`
    )
    .join("");
  const health = await api("/api/integrations");
  document.getElementById("integrations").textContent = JSON.stringify(health, null, 2);
}

document.getElementById("user-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const fd = new FormData(e.target);
  await api("/api/users", {
    method: "POST",
    json: {
      username: fd.get("username"),
      password: fd.get("password"),
      is_admin: fd.get("is_admin") === "on",
    },
  });
  e.target.reset();
  await loadAdmin();
});

document.getElementById("btn-sg-sync").addEventListener("click", async () => {
  const r = await api("/api/storygraph/sync", { method: "POST" });
  document.getElementById("integrations").textContent = JSON.stringify(r, null, 2);
});

document.getElementById("btn-tts-export").addEventListener("click", async () => {
  const r = await api("/api/queue/needs_tts/export", { method: "POST" });
  alert(JSON.stringify(r));
});

document.getElementById("btn-probe").addEventListener("click", async () => {
  const health = await api("/api/integrations");
  for (const h of health) {
    await api(`/api/integrations/${h.name}/repair`, { method: "POST" });
  }
  await loadAdmin();
});

boot();
