const state = {
  user: null,
  settings: null,
  taxonomy: [],
  users: [],
  currentWork: null,
  book: null,
  rendition: null,
  readerWorkId: null,
  readerHasEpub: false,
  readerHasMd: false,
  readerHasAudio: false,
  audioPlayerHidden: false,
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
  if (view !== "detail") {
    destroyReviewEditor();
    closeMdPreviewFloat();
  }
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

function parseTranscribeProgress(detail) {
  const m = String(detail || "").match(/transcribing chunk\s+(\d+)\s*\/\s*(\d+)/i);
  if (!m) return null;
  const cur = Number(m[1]);
  const total = Number(m[2]);
  if (!Number.isFinite(cur) || !Number.isFinite(total) || total <= 0) return null;
  return { cur, total };
}

function pollTranscribeJob(workId, jobId) {
  if (state._transcribePoll) clearTimeout(state._transcribePoll);
  state._transcribePoll = setTimeout(async () => {
    if (state.currentWork?.work?.id !== workId) return;
    try {
      const job = await api(`/api/jobs/${jobId}`);
      const row = document.getElementById("transcribe-progress");
      const fill = document.getElementById("transcribe-bar-fill");
      const text = document.getElementById("transcribe-progress-text");
      const status = job?.status;
      if (status === "pending" || status === "running") {
        if (row) row.hidden = false;
        const prog = parseTranscribeProgress(job?.detail);
        if (prog) {
          if (fill) fill.style.width = `${Math.max(4, Math.round((prog.cur / prog.total) * 100))}%`;
          if (text) text.textContent = `${prog.cur}/${prog.total}`;
        } else if (text) {
          text.textContent = status === "pending" ? "queued" : "working…";
        }
        pollTranscribeJob(workId, jobId);
        return;
      }
      // Finished or failed — refresh detail once for new assets / errors.
      await openDetail(workId);
    } catch (_) {
      pollTranscribeJob(workId, jobId);
    }
  }, 2500);
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
  const settingsP = api("/api/settings").catch(() => ({
    show_audio_gaps: true,
    reader_infinite_scroll: false,
    reader_typography: { ...TYPO_DEFAULTS },
  }));
  const taxonomyP = api("/api/taxonomy").catch(() => []);
  const usersP = state.user.is_admin ? api("/api/users").catch(() => []) : Promise.resolve([]);
  const [settings, taxonomy, users] = await Promise.all([settingsP, taxonomyP, usersP]);
  state.settings = settings;
  state.taxonomy = taxonomy;
  state.users = users;
  hydrateTypoFromSettings(state.settings);
  refreshYearFilterOptions();
  try {
    await loadWorks();
  } catch (e) {
    document.getElementById("work-list").innerHTML =
      `<p class="empty error">Could not load library: ${escapeHtml(e.message || String(e))}</p>`;
  }
}

function loadScriptOnce(src, globalCheck) {
  if (globalCheck()) return Promise.resolve();
  const key = `script:${src}`;
  if (!state._cdnLoads) state._cdnLoads = {};
  if (state._cdnLoads[key]) return state._cdnLoads[key];
  state._cdnLoads[key] = new Promise((resolve, reject) => {
    const s = document.createElement("script");
    s.src = src;
    s.async = true;
    s.onload = () => resolve();
    s.onerror = () => reject(new Error(`Failed to load ${src}`));
    document.head.appendChild(s);
  });
  return state._cdnLoads[key];
}

function loadCssOnce(href) {
  const key = `css:${href}`;
  if (!state._cdnLoads) state._cdnLoads = {};
  if (state._cdnLoads[key]) return state._cdnLoads[key];
  if (document.querySelector(`link[href="${href}"]`)) {
    state._cdnLoads[key] = Promise.resolve();
    return state._cdnLoads[key];
  }
  state._cdnLoads[key] = new Promise((resolve, reject) => {
    const l = document.createElement("link");
    l.rel = "stylesheet";
    l.href = href;
    l.onload = () => resolve();
    l.onerror = () => reject(new Error(`Failed to load ${href}`));
    document.head.appendChild(l);
  });
  return state._cdnLoads[key];
}

async function ensureEpubLibs() {
  await loadScriptOnce(
    "https://cdn.jsdelivr.net/npm/jszip@3.10.1/dist/jszip.min.js",
    () => typeof JSZip !== "undefined"
  );
  await loadScriptOnce(
    "https://cdn.jsdelivr.net/npm/epubjs@0.3.93/dist/epub.min.js",
    () => typeof ePub !== "undefined"
  );
}

async function ensureMarkedLibs() {
  await loadScriptOnce(
    "https://cdn.jsdelivr.net/npm/marked@15.0.7/marked.min.js",
    () => typeof marked !== "undefined"
  );
  await loadScriptOnce(
    "https://cdn.jsdelivr.net/npm/dompurify@3.2.4/dist/purify.min.js",
    () => typeof DOMPurify !== "undefined"
  );
}

async function ensureVditor() {
  const cdn = "https://cdn.jsdelivr.net/npm/vditor@3.10.9";
  await loadCssOnce(`${cdn}/dist/index.css`);
  await loadScriptOnce(`${cdn}/dist/index.min.js`, () => typeof Vditor !== "undefined");
}

async function ensureZxing() {
  await loadScriptOnce(
    "https://cdn.jsdelivr.net/npm/@zxing/library@0.21.3/umd/index.min.js",
    () => typeof ZXing !== "undefined"
  );
}

async function refreshGrantList(workId) {
  const el = document.getElementById("grant-list");
  if (!el) return;
  const grants = await api(`/api/works/${workId}/grants`);
  if (!grants.length) {
    el.textContent = "No shared access yet.";
    return;
  }
  el.innerHTML = grants
    .map(
      (g) =>
        `<div class="grant-row"><span>${escapeHtml(g.username)}</span>
          <button type="button" class="grant-revoke" data-uid="${escapeHtml(g.user_id)}">Revoke</button></div>`
    )
    .join("");
  el.querySelectorAll(".grant-revoke").forEach((btn) => {
    btn.addEventListener("click", async () => {
      btn.disabled = true;
      try {
        await api(`/api/works/${workId}/grants/${btn.dataset.uid}`, { method: "DELETE" });
        await refreshGrantList(workId);
      } catch (e) {
        btn.disabled = false;
        alert(e.message || String(e));
      }
    });
  });
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
    if (v === "integrations") await loadIntegrations();
    if (v === "admin") await loadAdmin();
    if (v === "settings") fillSettings();
  });
});

async function loadWorks() {
  const status = document.getElementById("filter-status").value;
  const year = document.getElementById("filter-year")?.value || "";
  const search = document.getElementById("library-search")?.value?.trim() || "";
  const params = new URLSearchParams();
  if (status) params.set("status", status);
  if (year) params.set("year_list", year);
  if (search) params.set("q", search);
  const q = params.toString() ? `?${params}` : "";
  const works = await api(`/api/works${q}`);
  if (!year && !search) refreshYearFilterOptions(works);
  const emptyHint = search
    ? `No books match “${search}”. Try another title, author, or ISBN.`
    : "Import an EPUB, or add something to your wishlist.";
  renderCards(document.getElementById("work-list"), works, {
    emptyTitle: search ? "No matches" : "Your library is empty",
    emptyHint,
    libraryActions: true,
    onStatusChanged: loadWorks,
  });
}

document.getElementById("filter-status").addEventListener("change", loadWorks);
document.getElementById("filter-year")?.addEventListener("change", loadWorks);

let librarySearchTimer = null;
document.getElementById("library-search")?.addEventListener("input", () => {
  clearTimeout(librarySearchTimer);
  librarySearchTimer = setTimeout(() => {
    loadWorks().catch((e) => setImportStatus(e.message || String(e), true));
  }, 220);
});
document.getElementById("library-search")?.addEventListener("keydown", (e) => {
  if (e.key === "Escape") {
    e.target.value = "";
    clearTimeout(librarySearchTimer);
    loadWorks().catch((err) => setImportStatus(err.message || String(err), true));
  }
});

function refreshYearFilterOptions(worksHint) {
  const sel = document.getElementById("filter-year");
  if (!sel) return;
  const current = sel.value;
  const years = new Set();
  const yNow = new Date().getFullYear();
  for (let y = yNow + 1; y >= yNow - 8; y--) years.add(String(y));
  if (Array.isArray(worksHint)) {
    for (const w of worksHint) {
      if (w.year_list != null && w.year_list !== "") years.add(String(w.year_list));
    }
  }
  const sorted = [...years].sort((a, b) => Number(b) - Number(a));
  sel.innerHTML =
    `<option value="">All years</option>` +
    sorted.map((y) => `<option value="${escapeHtml(y)}">${escapeHtml(y)} list</option>`).join("");
  if ([...sel.options].some((o) => o.value === current)) sel.value = current;
}

async function loadCurrentlyReading() {
  const works = await api("/api/works?status=reading");
  const cap = document.getElementById("currently-reading-cap");
  if (cap) cap.textContent = `${works.length} / 3 slots`;
  renderCards(document.getElementById("currently-reading-list"), works, {
    emptyTitle: "Nothing in progress",
    emptyHint: "Use Reading on a library card (max 3), or set status on the book’s detail page.",
    readAction: true,
  });
}

async function loadToRead() {
  const works = await api("/api/works?status=to_read");
  const cap = document.getElementById("to-read-cap");
  if (cap) cap.textContent = `${works.length} / 9 slots`;
  renderCards(document.getElementById("to-read-list"), works, {
    emptyTitle: "To Read is empty",
    emptyHint: "Use To read on a library card (max 9), or set status on the book’s detail page.",
    readAction: true,
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
  const softClearable = new Set([
    "needs_cover",
    "needs_tts",
    "needs_audio",
    "needs_transcription",
    "sg_review_dirty",
    "sg_needs_add",
    "sg_audio_only_remote",
  ]);
  renderCards(document.getElementById("attention-list"), works, {
    emptyTitle: "Nothing in this attention queue",
    emptyHint: "Flags appear here for review, covers, StoryGraph, audio gaps, etc.",
    clearFlag: softClearable.has(att) ? att : null,
    onCleared: loadAttention,
  });
}

document.getElementById("attention-filter").addEventListener("change", loadAttention);

const ATTENTION_CLEAR_LABELS = {
  needs_cover: "Dismiss cover flag",
  needs_tts: "Dismiss TTS flag",
  needs_audio: "Dismiss audio flag",
  needs_transcription: "Dismiss transcription flag",
  sg_review_dirty: "Clear SG review flag",
  sg_needs_add: "Clear add-to-SG flag",
  sg_audio_only_remote: "Dismiss SG audio gap",
};

async function openReaderForWork(workId, hints = {}) {
  if (hints.has_epub === false && hints.has_md === false && hints.has_audio === false) {
    throw new Error("No EPUB, markdown, or audiobook to open yet");
  }
  // Card list items already carry has_epub/has_md/has_audio (and the work
  // fields openReader needs) — skip the extra /api/works fetch when present,
  // and only hit the network if a hint is missing.
  const hintsComplete =
    hints.has_epub !== undefined && hints.has_md !== undefined && hints.has_audio !== undefined;
  let w, hasEpub, hasMd, audio;
  if (hintsComplete && hints.id === workId && hints.title) {
    w = hints;
    hasEpub = !!hints.has_epub;
    hasMd = !!hints.has_md;
    audio = hints.has_audio ? { relative_path: "book.m4b" } : null;
  } else {
    const data = await api(`/api/works/${workId}`);
    w = data.work;
    const assets = data.assets || [];
    hasEpub = assets.some((a) => a.kind === "epub");
    hasMd = assets.some((a) => a.kind === "markdown");
    const audioAssets = assets.filter((a) => a.kind === "audio");
    audio =
      audioAssets.find((a) => (a.relative_path || "").endsWith("book.m4b")) ||
      audioAssets[0] ||
      null;
  }
  if (!hasEpub && !hasMd && !audio) {
    throw new Error("No EPUB, markdown, or audiobook to open yet");
  }
  if (hasEpub) await ensureEpubLibs();
  if (hasMd || !hasEpub) await ensureMarkedLibs();
  await openReader(w, hasEpub, hasMd, audio);
}

async function setWorkStatus(workId, status) {
  return api(`/api/works/${workId}`, {
    method: "PUT",
    json: { status },
  });
}

function renderCards(el, works, empty = {}) {
  el.innerHTML = "";
  if (!works.length) {
    el.innerHTML = `<div class="empty">
      <strong>${escapeHtml(empty.emptyTitle || "Nothing here")}</strong>
      <p>${escapeHtml(empty.emptyHint || "")}</p>
    </div>`;
    return;
  }
  const clearFlag = empty.clearFlag || null;
  const libraryActions = !!empty.libraryActions;
  const readAction = !!empty.readAction || libraryActions;
  for (const w of works) {
    const card = document.createElement("div");
    card.className =
      "card" +
      (clearFlag ? " card-with-clear" : "") +
      (libraryActions || readAction ? " card-with-actions" : "");
    const badges = [];
    if (w.status && w.status !== "unread") {
      const statusLabel = { to_read: "to read", reading: "reading", read: "read", wishlist: "wishlist" }[w.status] || w.status;
      badges.push(statusLabel);
    }
    if (w.primary_code) badges.push(taxonomyLabel(w.primary_code).split(" — ")[1] || `#${w.primary_code}`);
    if (w.needs_review) badges.push("review");
    if (w.needs_cover) badges.push("needs cover");
    if (state.settings?.show_audio_gaps && w.needs_audio) badges.push("no audio");
    if (w.sg_review_dirty) badges.push("update SG");
    if (w.sg_needs_add) badges.push("add to SG");
    if (w.sg_audio_only_remote) badges.push("SG audio only");
    if (w.is_manga) badges.push("manga");
    const onToRead = w.status === "to_read";
    const onReading = w.status === "reading";
    const readDisabled = w.has_epub === false && w.has_md === false && w.has_audio === false;
    card.innerHTML = `
      <img src="/api/works/${w.id}/cover?v=${encodeURIComponent(w.updated_at || "")}" alt="" loading="lazy" onerror="this.style.opacity=0.25" />
      <div class="meta">
        <strong>${escapeHtml(w.title)}</strong>
        <span>${escapeHtml(w.authors || "")}</span>
        <div>${badges.map((b) => `<span class="badge">${escapeHtml(b)}</span>`).join("")}</div>
        ${
          readAction || libraryActions
            ? `<div class="card-actions">
                <button type="button" class="card-act card-read"${readDisabled ? " disabled" : ""} title="${readDisabled ? "No EPUB, markdown, or audio yet" : "Open reader"}">Read</button>
                ${
                  libraryActions
                    ? `<button type="button" class="card-act card-to-read"${onToRead ? " disabled" : ""} title="Add to To Read (max 9)">${onToRead ? "On To Read" : "To read"}</button>
                <button type="button" class="card-act card-reading"${onReading ? " disabled" : ""} title="Add to Currently Reading (max 3)">${onReading ? "On Reading" : "Reading"}</button>`
                    : ""
                }
              </div>`
            : ""
        }
      </div>
      ${
        clearFlag
          ? `<button type="button" class="card-clear" title="${escapeHtml(ATTENTION_CLEAR_LABELS[clearFlag] || "Clear flag")}">${escapeHtml(ATTENTION_CLEAR_LABELS[clearFlag] || "Clear")}</button>`
          : ""
      }`;
    card.addEventListener("click", () => openDetail(w.id));
    card.querySelector(".card-clear")?.addEventListener("click", async (e) => {
      e.preventDefault();
      e.stopPropagation();
      const btn = e.currentTarget;
      btn.disabled = true;
      try {
        await api(`/api/works/${w.id}/flags/clear`, {
          method: "POST",
          json: { flags: [clearFlag] },
        });
        if (typeof empty.onCleared === "function") await empty.onCleared();
        else card.remove();
      } catch (err) {
        btn.disabled = false;
        alert(err.message || String(err));
      }
    });
    card.querySelector(".card-read")?.addEventListener("click", async (e) => {
      e.preventDefault();
      e.stopPropagation();
      if (readDisabled) return;
      const btn = e.currentTarget;
      btn.disabled = true;
      try {
        await openReaderForWork(w.id, w);
      } catch (err) {
        btn.disabled = false;
        alert(err.message || String(err));
      }
    });
    card.querySelector(".card-to-read")?.addEventListener("click", async (e) => {
      e.preventDefault();
      e.stopPropagation();
      const btn = e.currentTarget;
      btn.disabled = true;
      try {
        await setWorkStatus(w.id, "to_read");
        if (typeof empty.onStatusChanged === "function") await empty.onStatusChanged();
      } catch (err) {
        btn.disabled = false;
        alert(err.message || String(err));
      }
    });
    card.querySelector(".card-reading")?.addEventListener("click", async (e) => {
      e.preventDefault();
      e.stopPropagation();
      const btn = e.currentTarget;
      btn.disabled = true;
      try {
        await setWorkStatus(w.id, "reading");
        if (typeof empty.onStatusChanged === "function") await empty.onStatusChanged();
      } catch (err) {
        btn.disabled = false;
        alert(err.message || String(err));
      }
    });
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
  const lower = file.name.toLowerCase();
  const isAudio = lower.endsWith(".aax") || lower.endsWith(".m4b") || lower.endsWith(".m4a");
  setImportStatus(
    isAudio && lower.endsWith(".aax")
      ? `Uploading AAX “${file.name}” for convert…`
      : `Uploading ${file.name}…`
  );
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
    const finish = async () => {
      setImportStatus(`Imported “${j.work.title}”. Opening…`);
      await loadWorks();
      await openDetail(j.work.id);
      setImportStatus("");
    };
    // M4B attach has no background job; AAX/ebook import poll until done.
    if (!j.job_id) {
      await finish();
      return;
    }
    setImportStatus(
      j.kind === "aax_to_m4b"
        ? `Converting AAX “${j.work.title}”…`
        : `Importing “${j.work.title}”…`
    );
    await pollJob(j.job_id, async (job) => {
      if (job.status === "failed") {
        setImportStatus(`Import failed: ${job.detail || "unknown error"}`, true);
        return;
      }
      await finish();
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

function setDetailMsg(t, isError = false) {
  const el = document.getElementById("detail-msg");
  if (!el) return;
  el.textContent = t || "";
  el.classList.toggle("error", !!isError);
}

// Cache-bust key for cover images — tied to the work's updated_at so the
// browser can actually cache covers between renders instead of refetching
// on every openDetail/patchDetail via Date.now().
function coverCacheKey(w) {
  return encodeURIComponent(w.updated_at || "");
}

function renderDetailCovers(data) {
  const w = data.work;
  const v = coverCacheKey(w);
  return `
      <img src="/api/works/${w.id}/cover?v=${v}" alt="" />
      ${data.has_cover_candidate ? `
        <div class="cover-candidate">
          <p class="muted">AI candidate</p>
          <img src="/api/works/${w.id}/cover/candidate?v=${v}" alt="Candidate cover" />
          <div class="row">
            <button type="button" id="btn-approve-cover">Approve cover</button>
            <button type="button" id="btn-discard-cover">Discard</button>
          </div>
        </div>` : ""}`;
}

function renderDetailStatusLine(data) {
  const w = data.work;
  const assets = data.assets || [];
  const hasEpub = assets.some((a) => a.kind === "epub");
  const hasMd = assets.some((a) => a.kind === "markdown");
  const hasAudio = assets.some((a) => a.kind === "audio");
  const trJob = data.transcription_job || null;
  const trFailed = (trJob?.status || null) === "failed";
  let transcriptBadge = "";
  if (data.has_transcript) transcriptBadge = " · Transcript ready";
  else if (trFailed) transcriptBadge = " · Transcription failed";
  return `${hasEpub ? "EPUB ready" : "No EPUB yet"} · ${hasMd ? "Markdown ready" : "No Markdown"} · ${hasAudio ? "M4B ready" : (w.needs_audio ? "Needs audio" : "No audio")} · Direction: ${escapeHtml(w.reading_direction)}${w.needs_review ? " · Needs review" : ""}${w.needs_cover ? " · Needs cover" : ""}${transcriptBadge}${w.sg_matched ? " · On StoryGraph" : ""}${w.sg_review_dirty ? " · Update SG review" : ""}${w.sg_needs_add ? " · Add to StoryGraph" : ""}${w.sg_audio_only_remote ? " · SG audio, missing local" : ""}`;
}

function renderDetailFlagButtons(data) {
  const w = data.work;
  return `
        <button type="button" id="btn-fetch-cover"${w.isbn ? "" : " disabled title=\"Add an ISBN first\""}>Fetch cover</button>
        <button type="button" id="btn-generate-cover" title="Generate via LocalAI (staged — approve before it replaces the cover)">Generate cover</button>
        <button type="button" id="btn-placeholder-cover" title="Replace cover with SVG from current title &amp; authors">Reset to placeholder</button>
        <label class="btn-file">Upload cover <input type="file" id="cover-file" accept="image/*" hidden /></label>
        ${w.needs_cover ? `<button type="button" id="btn-clear-needs-cover">Dismiss needs cover</button>` : ""}
        ${w.sg_review_dirty ? `<button type="button" id="btn-clear-sg-review" title="Clear after you update StoryGraph">Clear SG review flag</button>` : ""}
        ${w.sg_needs_add ? `<button type="button" id="btn-clear-sg-add" title="Clear after you add this book on StoryGraph">Clear add-to-SG flag</button>` : ""}
        ${w.sg_audio_only_remote ? `<button type="button" id="btn-clear-sg-audio">Dismiss SG audio gap</button>` : ""}`;
}

// Wires the cover/flag buttons rendered by renderDetailCovers /
// renderDetailFlagButtons. Shared by openDetail's initial render and by
// patchDetail's targeted re-render so both stay in sync.
function wireDetailCoverAndFlagButtons(id) {
  document.getElementById("btn-fetch-cover")?.addEventListener("click", async () => {
    setDetailMsg("Fetching cover from Open Library / Google Books…");
    try {
      const r = await api(`/api/works/${id}/cover/fetch`, { method: "POST" });
      if (!r?.ok) {
        setDetailMsg(r?.message || "No remote cover found", true);
        return;
      }
      setDetailMsg("Cover fetched");
      await patchDetail(id);
    } catch (e) {
      setDetailMsg(e.message || String(e), true);
    }
  });

  document.getElementById("btn-generate-cover")?.addEventListener("click", async () => {
    const promptEl = document.getElementById("cover-prompt");
    setDetailMsg("Generating cover via LocalAI (may take a minute)…");
    try {
      const r = await api(`/api/works/${id}/cover/generate`, { method: "POST" });
      if (r?.prompt && promptEl) {
        promptEl.hidden = false;
        promptEl.textContent = `Prompt: ${r.prompt}`;
      }
      if (!r?.ok) {
        setDetailMsg(r?.message || "Cover generation failed", true);
        return;
      }
      setDetailMsg("Candidate ready — approve or discard");
      await patchDetail(id);
    } catch (e) {
      setDetailMsg(e.message || String(e), true);
    }
  });

  document.getElementById("btn-placeholder-cover")?.addEventListener("click", async () => {
    const form = document.getElementById("detail-form");
    const fd = form ? new FormData(form) : null;
    const title = fd ? String(fd.get("title") || "").trim() : "";
    const authors = fd ? String(fd.get("authors") || "").trim() : "";
    setDetailMsg("Resetting cover to placeholder…");
    try {
      const r = await api(`/api/works/${id}/cover/placeholder`, {
        method: "POST",
        json: {
          title: title || undefined,
          authors: authors || undefined,
        },
      });
      if (!r?.ok) {
        setDetailMsg(r?.message || "Could not reset cover", true);
        return;
      }
      setDetailMsg("Cover reset to placeholder SVG");
      await patchDetail(id);
    } catch (e) {
      setDetailMsg(e.message || String(e), true);
    }
  });

  document.getElementById("btn-approve-cover")?.addEventListener("click", async () => {
    setDetailMsg("Approving candidate cover…");
    try {
      const r = await api(`/api/works/${id}/cover/approve`, { method: "POST" });
      if (!r?.ok) {
        setDetailMsg(r?.message || "Approve failed", true);
        return;
      }
      setDetailMsg("Cover approved");
      await patchDetail(id);
    } catch (e) {
      setDetailMsg(e.message || String(e), true);
    }
  });

  document.getElementById("btn-discard-cover")?.addEventListener("click", async () => {
    await api(`/api/works/${id}/cover/discard`, { method: "POST" });
    setDetailMsg("Candidate discarded");
    await patchDetail(id);
  });

  document.getElementById("btn-clear-needs-cover")?.addEventListener("click", async () => {
    await api(`/api/works/${id}/flags/clear`, {
      method: "POST",
      json: { flags: ["needs_cover"] },
    });
    setDetailMsg("Cleared needs_cover");
    await patchDetail(id);
  });
  async function clearSgFlag(flag, label) {
    await api(`/api/works/${id}/flags/clear`, {
      method: "POST",
      json: { flags: [flag] },
    });
    setDetailMsg(label);
    await patchDetail(id);
  }
  document.getElementById("btn-clear-sg-review")?.addEventListener("click", () =>
    clearSgFlag("sg_review_dirty", "Cleared StoryGraph review flag")
  );
  document.getElementById("btn-clear-sg-add")?.addEventListener("click", () =>
    clearSgFlag("sg_needs_add", "Cleared add-to-StoryGraph flag")
  );
  document.getElementById("btn-clear-sg-audio")?.addEventListener("click", () =>
    clearSgFlag("sg_audio_only_remote", "Cleared SG audio gap flag")
  );

  document.getElementById("cover-file")?.addEventListener("change", async (e) => {
    const file = e.target.files[0];
    if (!file) return;
    const fd = new FormData();
    fd.append("file", file);
    await fetch(`/api/works/${id}/cover`, { method: "POST", body: fd, credentials: "include" });
    await patchDetail(id);
  });
}

// Lightweight refresh for flag/status/cover changes on the work currently
// shown in the detail view: re-fetches and patches just the covers, status
// line, and flag buttons instead of doing openDetail's full innerHTML
// rebuild (which also tears down the review editor and refocuses fields).
// Falls back to a full openDetail when the work isn't the one on screen.
async function patchDetail(id) {
  const viewingThis =
    state.currentWork?.work?.id === id && !document.getElementById("view-detail")?.hidden;
  if (!viewingThis) return openDetail(id);
  const data = await api(`/api/works/${id}`);
  state.currentWork = data;
  const coversEl = document.getElementById("detail-covers");
  if (coversEl) coversEl.innerHTML = renderDetailCovers(data);
  const statusEl = document.getElementById("detail-status-line");
  if (statusEl) statusEl.innerHTML = renderDetailStatusLine(data);
  const flagButtonsEl = document.getElementById("detail-flag-buttons");
  if (flagButtonsEl) flagButtonsEl.innerHTML = renderDetailFlagButtons(data);
  wireDetailCoverAndFlagButtons(id);
}

async function openDetail(id) {
  destroyReviewEditor();
  closeMdPreviewFloat();
  const data = await api(`/api/works/${id}`);
  state.currentWork = data;
  show("detail");
  const w = data.work;
  const assets = data.assets || [];
  const hasEpub = assets.some((a) => a.kind === "epub");
  const hasMd = assets.some((a) => a.kind === "markdown");
  const hasImportPdf = !!data.has_import_pdf;
  const audioAssets = assets.filter((a) => a.kind === "audio");
  const audio =
    audioAssets.find((a) => (a.relative_path || "").endsWith("book.m4b")) ||
    audioAssets[0] ||
    null;
  const hasAudio = !!audio;
  const codes = data.codes || [];
  const trJob = data.transcription_job || null;
  const trStatus = trJob?.status || null;
  const trBusy = trStatus === "pending" || trStatus === "running";
  const trFailed = trStatus === "failed";
  const trProgress = parseTranscribeProgress(trJob?.detail);
  let transcriptBadge = "";
  if (data.has_transcript) transcriptBadge = " · Transcript ready";
  else if (trFailed) transcriptBadge = " · Transcription failed";
  // In-progress status uses the inline progress row instead of badge text.
  const mdSideReview = !!(w.needs_review && !hasImportPdf);
  const pdfReview = !!(w.needs_review && hasImportPdf);
  const reviewMdPane = `
        <div class="review-pane review-md"${mdSideReview ? ' aria-label="Transcript markdown"' : ""}>
          <div class="review-md-head">
            <h3>Markdown</h3>
            <p class="muted review-md-hint">Typora-style live editing · Ctrl+S save · toggle Source / Live / WYSIWYG in the toolbar</p>
          </div>
          <div id="review-find" class="review-find" hidden>
            <input type="text" id="review-find-q" placeholder="Find" />
            <input type="text" id="review-find-r" placeholder="Replace" />
            <button type="button" id="review-find-next">Next</button>
            <button type="button" id="review-find-replace">Replace</button>
            <button type="button" id="review-find-all">Replace all</button>
            <button type="button" id="review-find-close">Close</button>
          </div>
          <div id="review-md-vditor" class="review-md-vditor" role="textbox" aria-label="Markdown editor"></div>
          <div class="review-actions">
            <button type="button" id="btn-save-md">Save markdown</button>
            <span id="review-dirty" class="review-dirty" hidden>Unsaved changes</span>
            <button type="button" id="btn-md-find" title="Find / Replace (Ctrl+F)">Find</button>
            <button type="button" id="btn-preview-md" title="Open floating HTML preview">Preview</button>
            <a href="/api/works/${w.id}/download/markdown"><button type="button">Download MD</button></a>
            <button type="button" id="btn-confirm">Confirm → EPUB</button>
          </div>
        </div>`;

  const detailEl = document.getElementById("detail");
  detailEl.classList.toggle("detail-md-side", mdSideReview);
  detailEl.innerHTML = `
    <div class="detail-covers" id="detail-covers">${renderDetailCovers(data)}
    </div>
    <div class="detail-main">
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
        <label>Description
          <textarea name="description" rows="4" placeholder="Optional blurb">${escapeHtml(w.description || "")}</textarea>
        </label>
        <label class="check"><input type="checkbox" name="is_manga" ${w.is_manga ? "checked" : ""} /> Manga / RTL</label>
        <div class="row">
          <button type="submit">Save metadata</button>
          <button type="button" id="btn-enrich-meta">Enrich from ISBN / search</button>
        </div>
        <div id="meta-hits" class="meta-hits" hidden></div>
      </form>
      <p class="muted" id="detail-status-line">${renderDetailStatusLine(data)}</p>
      <div id="transcribe-progress" class="transcribe-progress"${trBusy || w.needs_transcription ? "" : " hidden"}>
        <span class="transcribe-label">Transcribing…</span>
        <div class="transcribe-bar" aria-hidden="true"><i id="transcribe-bar-fill" style="width:${trProgress ? Math.max(4, Math.round((trProgress.cur / trProgress.total) * 100)) : 8}%"></i></div>
        <span id="transcribe-progress-text" class="muted">${trProgress ? `${trProgress.cur}/${trProgress.total}` : "starting"}</span>
      </div>
      ${trFailed && trJob?.detail ? `<p class="error">${escapeHtml(String(trJob.detail).slice(0, 280))}</p>` : ""}
      <p id="cover-prompt" class="muted cover-prompt" hidden></p>
      ${hasAudio ? `
        <div class="detail-audio">
          <audio id="detail-audio" controls preload="metadata"
            src="/api/works/${w.id}/audio/${escapeHtml((audio.relative_path || "book.m4b").split("/").pop() || "book.m4b")}"></audio>
          <label class="audio-speed">
            Speed
            <input type="range" id="detail-audio-speed" min="0.25" max="2.5" step="0.05" value="1" />
            <span id="detail-audio-speed-val">1.00×</span>
          </label>
        </div>` : ""}
      <div class="actions">
        ${hasEpub || hasMd ? `<button type="button" id="btn-read">Read</button>` : ""}
        ${hasAudio ? `<button type="button" id="btn-listen">${hasEpub || hasMd ? "Listen" : "Play audiobook"}</button>` : ""}
        <button type="button" id="btn-refresh-meta">Refresh metadata</button>
        <label class="btn-file">Replace / import file <input type="file" id="import-file" accept=".epub,.pdf,.md,.markdown" hidden /></label>
        ${hasEpub ? `<a href="/api/works/${w.id}/download/epub"><button type="button">Download EPUB</button></a>` : ""}
        ${hasMd ? `<a href="/api/works/${w.id}/download/markdown"><button type="button">Download MD</button></a>` : ""}
        ${hasEpub ? `<button type="button" id="btn-remarkable">Send to reMarkable</button>` : ""}
        <span id="detail-flag-buttons">${renderDetailFlagButtons(data)}</span>
        <label class="btn-file">Upload audiobook (.m4b / .aax) <input type="file" id="audio-file" accept=".m4b,.aax,audio/mp4" hidden /></label>
        <button type="button" id="btn-transcribe"${hasAudio && !trBusy ? "" : ` disabled title="${!hasAudio ? "Upload an audiobook first" : "Transcription already in progress"}"`}>${trFailed ? "Retry transcription" : "Transcribe"}</button>
        ${data.has_transcript ? `<a href="/api/works/${w.id}/transcript"><button type="button">Download transcript</button></a>` : ""}
        ${state.user.is_admin ? `
          <div class="grant-panel">
            <label>Grant access
              <select id="grant-user">
                <option value="">Select user…</option>
                ${state.users.filter((u) => !u.is_admin).map((u) =>
                  `<option value="${escapeHtml(u.username)}">${escapeHtml(u.username)}</option>`).join("")}
              </select>
            </label>
            <button type="button" id="btn-grant">Grant</button>
            <div id="grant-list" class="grant-list muted">Loading grants…</div>
          </div>
        ` : ""}
        ${(state.user.is_admin || w.created_by === state.user.id)
          ? `<button type="button" id="btn-delete-work" class="danger">Delete book</button>`
          : ""}
      </div>
      ${pdfReview ? `
      <section class="review-workspace" aria-label="Import review">
        <div class="review-pane review-pdf">
          <h3>Source PDF</h3>
          <iframe title="Quarantined PDF" src="/api/works/${w.id}/content/pdf"></iframe>
        </div>
        ${reviewMdPane}
      </section>
      ` : ""}
      <pre id="detail-msg"></pre>
    </div>
    ${mdSideReview ? reviewMdPane : ""}`;

  const msg = setDetailMsg;

  if (trBusy && trJob?.id) {
    pollTranscribeJob(w.id, trJob.id);
  } else if (state._transcribePoll) {
    clearTimeout(state._transcribePoll);
    state._transcribePoll = null;
  }

  if (trFailed && trJob?.detail) {
    msg(String(trJob.detail).slice(0, 400), true);
  }

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
        description: String(fd.get("description") || "").trim() || null,
        status: fd.get("status"),
        primary_code: primary,
        year_list: yearRaw ? Number(yearRaw) : undefined,
        clear_year_list: !yearRaw,
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

  document.getElementById("btn-read")?.addEventListener("click", async () => {
    try {
      if (hasEpub) await ensureEpubLibs();
      if (hasMd) await ensureMarkedLibs();
      await openReader(w, hasEpub, hasMd, audio);
    } catch (e) {
      msg(e.message || String(e), true);
    }
  });
  document.getElementById("btn-listen")?.addEventListener("click", () => openReader(w, hasEpub, hasMd, audio));
  if (hasAudio) {
    const detailAudio = document.getElementById("detail-audio");
    wireAudioSpeed(detailAudio, "detail-audio-speed", "detail-audio-speed-val");
  }
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
  async function applyMetaHit(hit) {
    if (!hit) return;
    try {
      const r = await api(`/api/works/${w.id}/apply-meta`, {
        method: "POST",
        json: hit,
      });
      const primary = r?.work?.primary_code ?? r?.applied_primary;
      const tax =
        primary != null
          ? ` · primary ${taxonomyLabel(primary)}`
          : Array.isArray(hit.subjects) && hit.subjects.length
            ? " · no taxonomy match for provider subjects"
            : "";
      msg(`Applied metadata from ${hit.source || "lookup"}${tax}`);
      await openDetail(w.id);
    } catch (e) {
      msg(e.message || String(e), true);
    }
  }

  function showMetaHits(hits) {
    const box = document.getElementById("meta-hits");
    if (!box) return;
    if (!hits?.length) {
      box.hidden = true;
      box.innerHTML = "";
      return;
    }
    box.hidden = false;
    box.innerHTML = `<p class="muted">Pick a match:</p>` + hits.map((h, i) => {
      const tax = h.primary_code != null
        ? ` · ${taxonomyLabel(h.primary_code)}`
        : (Array.isArray(h.subjects) && h.subjects.length
            ? ` · ${escapeHtml(h.subjects.slice(0, 2).join(", "))}`
            : "");
      return `
      <button type="button" class="meta-hit" data-hit="${i}">
        <strong>${escapeHtml(h.title || "Untitled")}</strong>
        <span>${escapeHtml(h.authors || "Unknown author")}</span>
        <span class="muted">${escapeHtml(h.source || "")}${h.isbn ? " · " + escapeHtml(h.isbn) : ""}${tax}</span>
      </button>`;
    }).join("");
    box.querySelectorAll(".meta-hit").forEach((btn) => {
      btn.addEventListener("click", () => applyMetaHit(hits[Number(btn.dataset.hit)]));
    });
  }

  document.getElementById("btn-enrich-meta")?.addEventListener("click", async () => {
    const form = document.getElementById("detail-form");
    const fd = new FormData(form);
    const isbn = String(fd.get("isbn") || "").trim();
    const title = String(fd.get("title") || "").trim();
    const authors = String(fd.get("authors") || "").trim();
    msg(isbn ? `Looking up ISBN ${isbn}…` : "Searching Open Library / Google Books / LoC / StoryGraph…");
    showMetaHits([]);
    try {
      let providerNote = "";
      if (isbn) {
        const report = await api(`/api/metadata/isbn/${encodeURIComponent(isbn)}`);
        const hit = report?.hit ?? (report?.title ? report : null);
        if (hit && hit.title) {
          await applyMetaHit(hit);
          return;
        }
        const providers = Array.isArray(report?.providers) ? report.providers : [];
        const errors = providers.filter((p) => p.status === "error");
        const misses = providers.filter((p) => p.status === "miss").map((p) => p.name);
        if (errors.length) {
          providerNote = errors
            .map((p) => `${p.name}: ${p.detail || "error"}`)
            .join("; ");
          msg(`ISBN lookup issue (${providerNote}). Trying title search…`, true);
        } else if (misses.length) {
          providerNote = `ISBN not in ${misses.join("/")}`;
          msg(`${providerNote}. Trying title search…`);
        }
      }
      if (!title) {
        msg(
          providerNote
            ? `${providerNote}; no title to search`
            : "No metadata match found",
          true
        );
        return;
      }
      const q = new URLSearchParams({ title });
      if (authors) q.set("author", authors);
      const report = await api(`/api/metadata/search?${q}`);
      const hits = Array.isArray(report) ? report : report?.hits || [];
      const notes = Array.isArray(report?.notes) ? report.notes : [];
      const noteText = notes.join("; ");
      if (!hits.length) {
        const base = providerNote
          ? `${providerNote}; title search also empty`
          : "No metadata match found";
        const extra = noteText ? ` — ${noteText}` : "";
        const hint =
          !noteText && /rate limited|DIARCH_GOOGLE_BOOKS_KEY/i.test(providerNote)
            ? " — set DIARCH_GOOGLE_BOOKS_KEY for Google Books quota"
            : "";
        msg(base + extra + hint, true);
        return;
      }
      msg(
        hits.length === 1
          ? `Found 1 match${noteText ? ` (${noteText})` : ""} — confirm below`
          : `Found ${hits.length} matches${noteText ? ` (${noteText})` : ""} — pick one`
      );
      showMetaHits(hits);
    } catch (e) {
      msg(e.message || String(e), true);
    }
  });

  wireDetailCoverAndFlagButtons(w.id);

  document.getElementById("btn-transcribe")?.addEventListener("click", async () => {
    msg("Queueing transcription…");
    try {
      const r = await api(`/api/works/${w.id}/transcribe`, { method: "POST" });
      if (!r?.ok) {
        msg(r?.message || "Transcription unavailable", true);
        return;
      }
      msg(r.message || "Transcription queued");
      await openDetail(w.id);
    } catch (e) {
      msg(e.message || String(e), true);
    }
  });

  document.getElementById("btn-remarkable")?.addEventListener("click", async () => {
    msg("Sending EPUB to reMarkable via rmapi…");
    try {
      const r = await api(`/api/works/${w.id}/remarkable`, { method: "POST" });
      if (!r?.ok) {
        msg(r?.error || "reMarkable send failed", true);
        return;
      }
      msg("Sent to reMarkable (Diarch folder)");
    } catch (e) {
      msg(e.message || String(e), true);
    }
  });
  if (w.needs_review) {
    wireReviewMarkdownEditor(w.id, msg);
  }
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
    if (document.getElementById("review-md-vditor") || reviewVditor) {
      const md = getReviewMarkdown();
      const res = await fetch(`/api/works/${w.id}/content/markdown`, {
        method: "PUT",
        credentials: "include",
        headers: { "Content-Type": "text/markdown; charset=utf-8" },
        body: md,
      });
      if (!res.ok) {
        msg(await res.text() || "Could not save markdown before confirm", true);
        return;
      }
      setReviewDirty(false, md);
    }
    const j = await api(`/api/works/${w.id}/confirm`, { method: "POST" });
    msg("Building EPUB…");
    await pollJob(j.job_id, async (job) => {
      if (job.status === "failed") msg(job.detail || "Failed", true);
      else await openDetail(w.id);
    });
  });
  document.getElementById("audio-file")?.addEventListener("change", async (e) => {
    const file = e.target.files?.[0];
    e.target.value = "";
    if (!file) return;
    const lower = file.name.toLowerCase();
    msg(lower.endsWith(".aax") ? `Uploading AAX “${file.name}” for convert…` : `Uploading audiobook “${file.name}”…`);
    const fd = new FormData();
    fd.append("file", file);
    try {
      const res = await fetch(`/api/works/${w.id}/audio`, {
        method: "POST",
        body: fd,
        credentials: "include",
      });
      const textBody = await res.text();
      let body = null;
      try {
        body = textBody ? JSON.parse(textBody) : null;
      } catch {
        body = { message: textBody };
      }
      if (!res.ok) {
        msg(body?.message || textBody || `Upload failed (${res.status})`, true);
        return;
      }
      if (body?.job_id) {
        msg("Converting AAX → M4B…");
        await pollJob(body.job_id, async (job) => {
          if (job.status === "failed") msg(job.detail || "AAX convert failed", true);
          else {
            msg("Audiobook ready (book.m4b)");
            await openDetail(w.id);
          }
        });
        return;
      }
      msg(`Attached ${body?.filename || "book.m4b"}`);
      await openDetail(w.id);
    } catch (err) {
      msg(err.message || String(err), true);
    }
  });

  document.getElementById("btn-grant")?.addEventListener("click", async () => {
    const username = document.getElementById("grant-user").value;
    if (!username) {
      msg("Pick a user", true);
      return;
    }
    await api(`/api/works/${w.id}/grants`, { method: "POST", json: { username } });
    msg(`Granted to ${username}`);
    await refreshGrantList(w.id);
  });

  if (state.user.is_admin) {
    refreshGrantList(w.id).catch(() => {
      const el = document.getElementById("grant-list");
      if (el) el.textContent = "Could not load grants";
    });
  }

  document.getElementById("btn-delete-work")?.addEventListener("click", async () => {
    const ok = window.confirm(
      `Delete “${w.title}” permanently?\n\nThis removes the book, EPUB/Markdown, audio, and cover from the library.`
    );
    if (!ok) return;
    try {
      msg("Deleting…");
      await api(`/api/works/${w.id}`, { method: "DELETE" });
      state.currentWork = null;
      show("library");
      await loadWorks();
    } catch (e) {
      msg(e.message || String(e), true);
    }
  });
}

document.getElementById("back-library").addEventListener("click", () => {
  destroyReviewEditor();
  closeMdPreviewFloat();
  show("library");
  loadWorks();
});


/* —— Review markdown editor (Vditor IR ≈ Typora) —— */
let reviewSavedText = "";
let reviewPreviewWin = null;
let reviewVditor = null;
let reviewFindFrom = 0;
const VDITOR_CDN = "https://cdn.jsdelivr.net/npm/vditor@3.10.9";

function destroyReviewEditor() {
  if (!reviewVditor) return;
  try {
    reviewVditor.destroy();
  } catch (_) {
    /* ignore */
  }
  reviewVditor = null;
}

function getReviewMarkdown() {
  if (reviewVditor) {
    try {
      return reviewVditor.getValue();
    } catch (_) {
      /* fall through */
    }
  }
  return "";
}

function setReviewDirty(dirty, savedText) {
  if (typeof savedText === "string") reviewSavedText = savedText;
  const badge = document.getElementById("review-dirty");
  const confirm = document.getElementById("btn-confirm");
  if (badge) badge.hidden = !dirty;
  if (confirm) confirm.disabled = !!dirty;
}

function markdownToSafeHtml(md) {
  let html = "";
  try {
    if (typeof marked !== "undefined") {
      html = marked.parse(md || "", { async: false });
    } else {
      html = `<pre>${escapeHtml(md || "")}</pre>`;
    }
  } catch (e) {
    html = `<p class="error">Preview error: ${escapeHtml(e.message || String(e))}</p>`;
  }
  if (typeof DOMPurify !== "undefined") return DOMPurify.sanitize(html);
  return `<pre>${escapeHtml(md || "")}</pre>`;
}

function closeMdPreviewFloat() {
  document.getElementById("md-preview-float")?.remove();
}

function fillMdPreviewBody(md) {
  const body = document.getElementById("md-preview-body");
  if (body) body.innerHTML = markdownToSafeHtml(md);
}

function openMdPreviewFloat() {
  closeMdPreviewFloat();
  const float = document.createElement("div");
  float.id = "md-preview-float";
  float.className = "md-preview-float";
  float.setAttribute("role", "dialog");
  float.setAttribute("aria-label", "Markdown preview");
  float.innerHTML = `
    <div class="md-preview-float-head" id="md-preview-drag">
      <strong>Markdown preview</strong>
      <div class="md-preview-float-actions">
        <button type="button" id="md-preview-refresh" title="Refresh from editor">Refresh</button>
        <button type="button" id="md-preview-tab" title="Open in new tab">New tab</button>
        <button type="button" id="md-preview-close" aria-label="Close preview">×</button>
      </div>
    </div>
    <div id="md-preview-body" class="md-preview-body"></div>`;
  document.body.appendChild(float);
  fillMdPreviewBody(getReviewMarkdown());

  document.getElementById("md-preview-close")?.addEventListener("click", closeMdPreviewFloat);
  document.getElementById("md-preview-refresh")?.addEventListener("click", () => {
    fillMdPreviewBody(getReviewMarkdown());
  });
  document.getElementById("md-preview-tab")?.addEventListener("click", () => {
    openMdPreviewTab(getReviewMarkdown());
  });

  const head = document.getElementById("md-preview-drag");
  let dragging = false;
  let ox = 0;
  let oy = 0;
  head?.addEventListener("pointerdown", (e) => {
    if (e.target.closest("button")) return;
    dragging = true;
    const rect = float.getBoundingClientRect();
    ox = e.clientX - rect.left;
    oy = e.clientY - rect.top;
    head.setPointerCapture?.(e.pointerId);
  });
  head?.addEventListener("pointermove", (e) => {
    if (!dragging) return;
    float.style.left = `${Math.max(0, e.clientX - ox)}px`;
    float.style.top = `${Math.max(0, e.clientY - oy)}px`;
    float.style.right = "auto";
    float.style.bottom = "auto";
  });
  head?.addEventListener("pointerup", () => {
    dragging = false;
  });
}

function openMdPreviewTab(md) {
  const html = markdownToSafeHtml(md);
  const doc = `<!DOCTYPE html><html lang="en"><head><meta charset="UTF-8" />
<title>Markdown preview — Diarch</title>
<style>
  body { margin: 0; padding: 1.5rem max(1rem, calc(50vw - 18rem));
    font: 1.05rem/1.6 system-ui, sans-serif; background: #0f1814; color: #e8efe6; }
  h1,h2,h3,h4 { color: #c4a35a; font-family: Palatino, Georgia, serif; }
  a { color: #e8b87a; } pre { overflow: auto; padding: .75rem; background: #0a100e;
    border: 1px solid #2d4038; border-radius: 3px; }
  blockquote { margin: .5rem 0; padding-left: .75rem; border-left: 3px solid #c4a35a; color: #9aaf9f; }
  code { font-family: ui-monospace, monospace; font-size: .9em; }
</style></head><body>${html}</body></html>`;
  if (reviewPreviewWin && !reviewPreviewWin.closed) {
    reviewPreviewWin.document.open();
    reviewPreviewWin.document.write(doc);
    reviewPreviewWin.document.close();
    reviewPreviewWin.focus();
    return;
  }
  reviewPreviewWin = window.open("", "diarch-md-preview");
  if (!reviewPreviewWin) {
    alert("Pop-up blocked — allow pop-ups for Diarch to open preview in a new tab.");
    return;
  }
  reviewPreviewWin.document.open();
  reviewPreviewWin.document.write(doc);
  reviewPreviewWin.document.close();
}

function toggleReviewFind() {
  const box = document.getElementById("review-find");
  if (!box) return;
  box.hidden = !box.hidden;
  if (!box.hidden) document.getElementById("review-find-q")?.focus();
}

function wireReviewMarkdownEditor(workId, msg) {
  const mount = document.getElementById("review-md-vditor");
  if (!mount) return;

  const confirmBtn = document.getElementById("btn-confirm");
  if (confirmBtn) confirmBtn.disabled = false;
  destroyReviewEditor();
  closeMdPreviewFloat();
  reviewFindFrom = 0;
  setReviewDirty(false, "");
  mount.textContent = "Loading editor…";

  ensureVditor()
    .then(() => ensureMarkedLibs())
    .then(() => initReviewVditor(workId, msg))
    .catch((e) => {
      mount.textContent = "";
      msg(e.message || "Markdown editor failed to load (Vditor CDN).", true);
    });
}

function initReviewVditor(workId, msg) {
  const mount = document.getElementById("review-md-vditor");
  if (!mount) return;
  if (typeof Vditor === "undefined") {
    msg("Markdown editor failed to load (Vditor CDN).", true);
    return;
  }
  mount.textContent = "";

  let pendingMd = null;
  let loadError = null;

  const loadPromise = fetch(`/api/works/${workId}/content/markdown`, { credentials: "include" })
    .then(async (res) => {
      pendingMd = res.ok ? await res.text() : "";
      if (!res.ok) loadError = `Could not load markdown (${res.status})`;
    })
    .catch((e) => {
      pendingMd = "";
      loadError = e.message || String(e);
    });

  const editorHeight = Math.max(420, Math.round(window.innerHeight * 0.72));

  reviewVditor = new Vditor("review-md-vditor", {
    cdn: VDITOR_CDN,
    height: editorHeight,
    minHeight: 360,
    mode: "ir",
    theme: "dark",
    icon: "ant",
    typewriterMode: true,
    placeholder: "Loading…",
    cache: { enable: false },
    toolbarConfig: { pin: true },
    counter: { enable: true, type: "text" },
    outline: { enable: true, position: "left" },
    preview: {
      theme: { current: "dark" },
      hljs: { style: "native", lineNumber: true },
      math: { engine: "KaTeX", inlineDigit: true },
    },
    toolbar: [
      "headings",
      "bold",
      "italic",
      "strike",
      "link",
      "|",
      "list",
      "ordered-list",
      "check",
      "outdent",
      "indent",
      "|",
      "quote",
      "line",
      "code",
      "inline-code",
      "table",
      "|",
      "undo",
      "redo",
      "|",
      "edit-mode",
      "both",
      "outline",
      "preview",
      "fullscreen",
      "|",
      "export",
    ],
    input: (value) => {
      setReviewDirty(value !== reviewSavedText);
    },
    after: async () => {
      await loadPromise;
      if (loadError) msg(loadError, true);
      const md = pendingMd ?? "";
      reviewVditor?.setValue(md, true);
      setReviewDirty(false, md);
      reviewVditor?.focus();
    },
  });

  document.getElementById("btn-save-md")?.addEventListener("click", async () => {
    const md = getReviewMarkdown();
    msg("Saving markdown…");
    const res = await fetch(`/api/works/${workId}/content/markdown`, {
      method: "PUT",
      credentials: "include",
      headers: { "Content-Type": "text/markdown; charset=utf-8" },
      body: md,
    });
    if (!res.ok) {
      msg((await res.text()) || `Save failed (${res.status})`, true);
      return;
    }
    setReviewDirty(false, md);
    msg("Markdown saved (still needs confirm)");
  });

  document.getElementById("btn-preview-md")?.addEventListener("click", async () => {
    try {
      await ensureMarkedLibs();
      openMdPreviewFloat();
    } catch (e) {
      msg(e.message || String(e), true);
    }
  });

  document.getElementById("btn-md-find")?.addEventListener("click", toggleReviewFind);

  const findNext = () => {
    const q = document.getElementById("review-find-q")?.value || "";
    if (!q) return;
    const val = getReviewMarkdown();
    let idx = val.indexOf(q, reviewFindFrom);
    if (idx < 0) idx = val.indexOf(q, 0);
    if (idx < 0) {
      msg("No matches", true);
      return;
    }
    reviewFindFrom = idx + q.length;
    const startLine = val.slice(0, idx).split("\n").length;
    msg(`Match at line ${startLine} (use Replace / Replace all, or Source mode)`);
  };
  document.getElementById("review-find-next")?.addEventListener("click", findNext);
  document.getElementById("review-find-replace")?.addEventListener("click", () => {
    const q = document.getElementById("review-find-q")?.value || "";
    const r = document.getElementById("review-find-r")?.value ?? "";
    if (!q || !reviewVditor) return;
    const val = getReviewMarkdown();
    let idx = val.indexOf(q, Math.max(0, reviewFindFrom - q.length));
    if (idx < 0) idx = val.indexOf(q, 0);
    if (idx < 0) {
      msg("No matches", true);
      return;
    }
    const next = val.slice(0, idx) + r + val.slice(idx + q.length);
    reviewVditor.setValue(next);
    reviewFindFrom = idx + r.length;
    setReviewDirty(next !== reviewSavedText);
    findNext();
  });
  document.getElementById("review-find-all")?.addEventListener("click", () => {
    const q = document.getElementById("review-find-q")?.value || "";
    const r = document.getElementById("review-find-r")?.value ?? "";
    if (!q || !reviewVditor) return;
    const val = getReviewMarkdown();
    if (!val.includes(q)) {
      msg("No matches", true);
      return;
    }
    const next = val.split(q).join(r);
    reviewVditor.setValue(next);
    setReviewDirty(next !== reviewSavedText);
    msg("Replaced all");
  });
  document.getElementById("review-find-close")?.addEventListener("click", () => {
    const box = document.getElementById("review-find");
    if (box) box.hidden = true;
  });
}

document.addEventListener("keydown", (e) => {
  if (!(e.ctrlKey || e.metaKey) || !reviewVditor) return;
  if (!document.getElementById("review-md-vditor")) return;
  if (e.key === "s") {
    e.preventDefault();
    document.getElementById("btn-save-md")?.click();
  } else if (e.key === "f") {
    const t = e.target;
    if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA") && t.id?.startsWith("review-find")) {
      return;
    }
    e.preventDefault();
    toggleReviewFind();
  }
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


/* —— Reader typography —— */
const TYPO_KEY = "diarch-reader-typo";
const TYPO_DEFAULTS = {
  palette: "dark",
  font: "serif",
  size: 100,
  lineHeight: "normal",
  measure: "medium",
  justify: false,
  letterSpacing: "normal",
  paragraphSpacing: "normal",
  indent: false,
  hyphenate: false,
};

const PALETTES = {
  dark: { ink: "#f3e6d4", pageBg: "#0c1210", link: "#e8b87a" },
  sepia: { ink: "#5b4636", pageBg: "#f4ecd8", link: "#8a5a2b" },
  light: { ink: "#1a1a1a", pageBg: "#fafafa", link: "#8a5a2b" },
  paper: { ink: "#2a241c", pageBg: "#f7f1e5", link: "#7a4e28" },
  night: { ink: "#d7e0f0", pageBg: "#0b1220", link: "#8fb4ff" },
  contrast: { ink: "#ffffff", pageBg: "#000000", link: "#ffe566" },
};

const FONTS = {
  serif: '"Iowan Old Style", "Palatino Linotype", Palatino, Georgia, serif',
  literata:
    'Literata, "Source Serif 4", "Iowan Old Style", "Palatino Linotype", Georgia, serif',
  sans: '"Source Sans 3", "Segoe UI", system-ui, sans-serif',
  dyslexia:
    '"Atkinson Hyperlegible", "OpenDyslexic", "Comic Sans MS", Verdana, sans-serif',
  mono: 'ui-monospace, "Cascadia Code", "Source Code Pro", monospace',
};

const LINE_HEIGHTS = { tight: 1.35, normal: 1.7, loose: 2.0 };
const LETTER_SPACING = { tight: "-0.01em", normal: "0", wide: "0.04em" };
const PARA_SPACING = { compact: "0.35em", normal: "0.85em", roomy: "1.35em" };
const MEASURES = {
  narrow: { maxCh: 52, padX: "max(2.5rem, calc(50vw - 14rem))", epubMargin: "12%" },
  medium: { maxCh: 68, padX: "max(1rem, calc(50vw - 18rem))", epubMargin: "6%" },
  wide: { maxCh: 88, padX: "1rem", epubMargin: "2%" },
};

let typoSaveTimer = null;

function normalizeTypo(raw) {
  const t = { ...TYPO_DEFAULTS, ...(raw || {}) };
  // Accept snake_case from older server payloads if any.
  if (raw?.line_height && !raw.lineHeight) t.lineHeight = raw.line_height;
  if (raw?.letter_spacing && !raw.letterSpacing) t.letterSpacing = raw.letter_spacing;
  if (raw?.paragraph_spacing && !raw.paragraphSpacing) {
    t.paragraphSpacing = raw.paragraph_spacing;
  }
  t.size = Math.min(200, Math.max(80, Number(t.size) || 100));
  t.size = Math.round(t.size / 5) * 5;
  if (!PALETTES[t.palette]) t.palette = TYPO_DEFAULTS.palette;
  if (!FONTS[t.font]) t.font = TYPO_DEFAULTS.font;
  if (!LINE_HEIGHTS[t.lineHeight]) t.lineHeight = TYPO_DEFAULTS.lineHeight;
  if (!MEASURES[t.measure]) t.measure = TYPO_DEFAULTS.measure;
  if (!LETTER_SPACING[t.letterSpacing]) t.letterSpacing = TYPO_DEFAULTS.letterSpacing;
  if (!PARA_SPACING[t.paragraphSpacing]) t.paragraphSpacing = TYPO_DEFAULTS.paragraphSpacing;
  t.justify = !!t.justify;
  t.indent = !!t.indent;
  t.hyphenate = !!t.hyphenate;
  return t;
}

function loadTypoLocal() {
  try {
    const raw = localStorage.getItem(TYPO_KEY);
    if (!raw) return { ...TYPO_DEFAULTS };
    return normalizeTypo(JSON.parse(raw));
  } catch {
    return { ...TYPO_DEFAULTS };
  }
}

function saveTypoLocal(typo) {
  try {
    localStorage.setItem(TYPO_KEY, JSON.stringify(typo));
  } catch {}
}

function setTypoSaveStatus(text, isError) {
  const el = document.getElementById("typo-save-status");
  if (!el) return;
  el.textContent = text || "";
  el.classList.toggle("error", !!isError);
}

async function persistTypoServer(typo) {
  const t = normalizeTypo(typo);
  setTypoSaveStatus("Saving…");
  try {
    await api("/api/settings", {
      method: "PUT",
      json: { reader_typography: t },
    });
    if (state.settings) state.settings.reader_typography = t;
    setTypoSaveStatus("Saved to account");
  } catch (e) {
    setTypoSaveStatus(e.message || "Save failed (kept locally)", true);
  }
}

function scheduleTypoServerSave(typo) {
  clearTimeout(typoSaveTimer);
  typoSaveTimer = setTimeout(() => persistTypoServer(typo), 450);
}

function loadTypo() {
  if (state.readerTypo) return normalizeTypo(state.readerTypo);
  return loadTypoLocal();
}

function saveTypo(typo) {
  const t = normalizeTypo(typo);
  state.readerTypo = t;
  saveTypoLocal(t);
  scheduleTypoServerSave(t);
}

function hydrateTypoFromSettings(settings) {
  const server = settings?.reader_typography;
  const local = loadTypoLocal();
  const serverEmpty =
    !server ||
    (typeof server === "object" &&
      Object.keys(server).length === 0);
  let t;
  if (!serverEmpty) {
    t = normalizeTypo(server);
  } else if (local && JSON.stringify(local) !== JSON.stringify(TYPO_DEFAULTS)) {
    // One-time migrate browser prefs → server.
    t = local;
    scheduleTypoServerSave(t);
  } else {
    t = { ...TYPO_DEFAULTS };
  }
  state.readerTypo = t;
  saveTypoLocal(t);
  fillTypoForm(t);
  applyReaderTypography(t);
}

function fillTypoForm(typo) {
  const form = document.getElementById("reader-typo-form");
  if (!form) return;
  const t = normalizeTypo(typo);
  const setRadio = (name, value) => {
    const el = form.querySelector(`input[name="${name}"][value="${value}"]`);
    if (el) el.checked = true;
  };
  setRadio("palette", t.palette);
  setRadio("font", t.font);
  form.size.value = t.size;
  setRadio("lineHeight", t.lineHeight);
  setRadio("measure", t.measure);
  setRadio("letterSpacing", t.letterSpacing);
  setRadio("paragraphSpacing", t.paragraphSpacing);
  form.justify.checked = !!t.justify;
  if (form.indent) form.indent.checked = !!t.indent;
  if (form.hyphenate) form.hyphenate.checked = !!t.hyphenate;
  const sv = document.getElementById("typo-size-val");
  if (sv) sv.textContent = `${t.size}%`;
}

function readTypoForm() {
  const form = document.getElementById("reader-typo-form");
  if (!form) return loadTypo();
  return normalizeTypo({
    palette: form.palette?.value || "dark",
    font: form.font?.value || "serif",
    size: Number(form.size?.value) || 100,
    lineHeight: form.lineHeight?.value || "normal",
    measure: form.measure?.value || "medium",
    justify: !!form.justify?.checked,
    letterSpacing: form.letterSpacing?.value || "normal",
    paragraphSpacing: form.paragraphSpacing?.value || "normal",
    indent: !!form.indent?.checked,
    hyphenate: !!form.hyphenate?.checked,
  });
}

function applyReaderTypography(typo) {
  const t = normalizeTypo(typo || loadTypo());
  state.readerTypo = t;
  const pal = PALETTES[t.palette] || PALETTES.dark;
  const ff = FONTS[t.font] || FONTS.serif;
  const lh = LINE_HEIGHTS[t.lineHeight] || LINE_HEIGHTS.normal;
  const measure = MEASURES[t.measure] || MEASURES.medium;
  const tracking = LETTER_SPACING[t.letterSpacing] || LETTER_SPACING.normal;
  const paraGap = PARA_SPACING[t.paragraphSpacing] || PARA_SPACING.normal;
  const fs = `${(1.1 * t.size) / 100}rem`;
  const stage = document.querySelector(".reader-stage");
  if (stage) {
    stage.style.setProperty("--reader-page-bg", pal.pageBg);
    stage.style.setProperty("--reader-ink", pal.ink);
  }
  const md = document.getElementById("md-area");
  if (md) {
    md.style.setProperty("--reader-ink", pal.ink);
    md.style.setProperty("--reader-ff", ff);
    md.style.setProperty("--reader-fs", fs);
    md.style.setProperty("--reader-lh", String(lh));
    md.style.setProperty("--reader-pad-x", measure.padX);
    md.style.setProperty("--reader-pad-y", "2rem");
    md.style.setProperty("--reader-max-w", `${measure.maxCh}ch`);
    md.style.setProperty("--reader-align", t.justify ? "justify" : "start");
    md.style.setProperty("--reader-tracking", tracking);
    md.style.setProperty("--reader-para-gap", paraGap);
    md.style.setProperty("--reader-indent", t.indent ? "1.25em" : "0");
    md.style.setProperty("--reader-hyphens", t.hyphenate ? "auto" : "manual");
    md.style.background = pal.pageBg;
    md.style.color = pal.ink;
  }
  if (state.rendition) {
    applyEpubTypography(t, pal, ff, lh, measure, tracking, paraGap);
  }
}

function applyEpubTypography(t, pal, ff, lh, measure, tracking, paraGap) {
  if (!state.rendition) return;
  t = normalizeTypo(t || state.readerTypo || TYPO_DEFAULTS);
  pal = pal || PALETTES[t.palette] || PALETTES.dark;
  ff = ff || FONTS[t.font] || FONTS.serif;
  lh = lh || LINE_HEIGHTS[t.lineHeight] || LINE_HEIGHTS.normal;
  measure = measure || MEASURES[t.measure] || MEASURES.medium;
  tracking = tracking || LETTER_SPACING[t.letterSpacing] || LETTER_SPACING.normal;
  paraGap = paraGap || PARA_SPACING[t.paragraphSpacing] || PARA_SPACING.normal;
  const align = t.justify ? "justify" : "start";
  const hyphens = t.hyphenate ? "auto" : "manual";
  const indent = t.indent ? "1.25em" : "0";
  try {
    state.rendition.themes.fontSize(`${t.size}%`);
  } catch {}
  try {
    state.rendition.themes.font(ff);
  } catch {}
  const w = state.currentWork?.work;
  const rtl = w && (w.reading_direction === "rtl" || w.is_manga);
  const bodyRules = {
    color: `${pal.ink} !important`,
    background: `${pal.pageBg} !important`,
    "font-family": `${ff} !important`,
    "line-height": `${lh} !important`,
    "letter-spacing": `${tracking} !important`,
    "text-align": `${align} !important`,
    hyphens: `${hyphens} !important`,
    "margin-left": `${measure.epubMargin} !important`,
    "margin-right": `${measure.epubMargin} !important`,
  };
  if (rtl) bodyRules.direction = "rtl";
  try {
    state.rendition.themes.default({
      body: bodyRules,
      "p, div, span, li, td, th, h1, h2, h3, h4, h5, h6, blockquote, pre, code": {
        color: `${pal.ink} !important`,
        "font-family": `${ff} !important`,
        "line-height": `${lh} !important`,
        "letter-spacing": `${tracking} !important`,
      },
      p: {
        "text-align": `${align} !important`,
        "margin-top": `${paraGap} !important`,
        "margin-bottom": `${paraGap} !important`,
        "text-indent": `${indent} !important`,
        hyphens: `${hyphens} !important`,
      },
      a: { color: `${pal.link} !important` },
    });
  } catch {}
}

/* —— Reader TOC —— */
function closeReaderPanels() {
  const toc = document.getElementById("reader-toc");
  const typo = document.getElementById("reader-typo");
  if (toc) toc.hidden = true;
  if (typo) typo.hidden = true;
}

function setTocEnabled(on) {
  const btn = document.getElementById("reader-toc-btn");
  if (btn) btn.disabled = !on;
}

function renderTocList(entries) {
  const list = document.getElementById("reader-toc-list");
  if (!list) return;
  list.innerHTML = "";
  if (!entries.length) {
    list.innerHTML = `<p class="reader-toc-empty">No contents available.</p>`;
    setTocEnabled(false);
    return;
  }
  setTocEnabled(true);
  for (const e of entries) {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.textContent = e.label;
    btn.className = e.level >= 3 ? "toc-l3" : e.level === 2 ? "toc-l2" : "";
    btn.addEventListener("click", () => {
      if (e.kind === "epub" && state.rendition && e.href) {
        state.rendition.display(e.href);
      } else if (e.kind === "md") {
        const area = document.getElementById("md-area");
        const target = area?.querySelector(`[data-toc-id="${e.id}"]`);
        if (target) target.scrollIntoView({ behavior: "smooth", block: "start" });
      }
      closeReaderPanels();
    });
    list.appendChild(btn);
  }
}

function flattenEpubToc(items, level = 1, out = []) {
  if (!items) return out;
  for (const item of items) {
    const label = (item.label || "").trim() || "Untitled";
    const href = item.href || "";
    out.push({ kind: "epub", label, href, level });
    if (item.subitems?.length) flattenEpubToc(item.subitems, level + 1, out);
  }
  return out;
}

async function populateEpubToc() {
  try {
    const nav = state.book?.navigation;
    if (nav?.toc) {
      renderTocList(flattenEpubToc(nav.toc));
      return;
    }
    if (typeof state.book?.loaded?.navigation?.then === "function") {
      const n = await state.book.loaded.navigation;
      renderTocList(flattenEpubToc(n?.toc));
      return;
    }
  } catch {}
  renderTocList([]);
}

function populateMarkdownToc(rawText) {
  const area = document.getElementById("md-area");
  if (!area) return;
  const lines = String(rawText || "").split("\n");
  const entries = [];
  area.textContent = "";
  let headingIdx = 0;
  for (const line of lines) {
    const m = /^(#{1,6})\s+(.+)$/.exec(line);
    if (m) {
      const level = m[1].length;
      const label = m[2].trim();
      const id = `md-toc-${headingIdx}`;
      entries.push({ kind: "md", label, level, id });
      const span = document.createElement("span");
      span.dataset.tocId = id;
      span.textContent = line + "\n";
      area.appendChild(span);
      headingIdx += 1;
    } else {
      area.appendChild(document.createTextNode(line + "\n"));
    }
  }
  renderTocList(entries);
}

async function openReader(w, hasEpub, hasMd, audio) {
  state.readerWorkId = w.id;
  state.readerHasEpub = !!hasEpub;
  state.readerHasMd = !!hasMd;
  state.readerHasAudio = !!audio;
  state.audioPlayerHidden = false;
  show("reader");
  closeReaderMenu();
  closeReaderPanels();
  fillTypoForm(loadTypo());
  applyReaderTypography(loadTypo());
  document.getElementById("reader-title").textContent = w.title;
  setReaderProgress("");
  const audioOnly = !!audio && !hasEpub && !hasMd;
  const preferScroll = !!state.settings?.reader_infinite_scroll && hasMd;
  const useMd = !audioOnly && (preferScroll || (!hasEpub && hasMd));
  setInfiniteScrollActive(useMd);
  setReaderChrome(useMd || audioOnly);
  document.getElementById("epub-area").hidden = useMd || audioOnly;
  document.getElementById("md-area").hidden = !useMd || audioOnly;
  setTocEnabled(false);
  renderTocList([]);

  const audioEl = document.getElementById("audio-player");
  if (audio) {
    const name = (audio.relative_path || "book.m4b").split("/").pop() || "book.m4b";
    audioEl.src = `/api/works/${w.id}/audio/${name}`;
    wireAudioSpeed(audioEl, "audio-speed", "audio-speed-val");
    setAudioPlayerVisible(true);
  } else {
    audioEl.pause?.();
    audioEl.removeAttribute("src");
    audioEl.load?.();
    setAudioPlayerVisible(false);
  }

  if (audioOnly) return;
  if (useMd) await loadMarkdown(w.id);
  else if (hasEpub) await loadEpub(w);
}

/** Show/hide the bottom audiobook bar. Only meaningful when the work has audio. */
function setAudioPlayerVisible(visible) {
  const wrap = document.getElementById("audio-wrap");
  const menuBtn = document.getElementById("reader-audio-toggle");
  const hasAudio = !!state.readerHasAudio;
  if (!hasAudio) {
    state.audioPlayerHidden = false;
    if (wrap) wrap.hidden = true;
    if (menuBtn) menuBtn.hidden = true;
    return;
  }
  const show = !!visible;
  state.audioPlayerHidden = !show;
  if (wrap) wrap.hidden = !show;
  if (menuBtn) {
    menuBtn.hidden = false;
    menuBtn.textContent = show ? "Hide audio player" : "Show audio player";
  }
  // Let flex layout settle, then resize the EPUB viewport so pages aren't
  // clipped under (or short of) the player bar.
  requestAnimationFrame(() => resizeReaderEpub());
}

function toggleAudioPlayer() {
  if (!state.readerHasAudio) return;
  setAudioPlayerVisible(!!state.audioPlayerHidden);
}

function resizeReaderEpub() {
  if (!state.rendition || isInfiniteScrollActive()) return;
  const area = document.getElementById("epub-area");
  if (!area || area.hidden) return;
  const width = Math.max(area.clientWidth || 0, 1);
  const height = Math.max(area.clientHeight || 0, 1);
  try {
    state.rendition.resize(width, height);
  } catch {}
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
  closeReaderPanels();
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

/* —— Reading progress save (debounced, like scheduleTypoServerSave) —— */
let progressSaveTimer = null;
let pendingProgressSave = null;

function persistProgress(workId, payload) {
  return api(`/api/works/${workId}/progress`, {
    method: "PUT",
    json: payload,
  }).catch(() => {});
}

function scheduleProgressSave(workId, payload) {
  pendingProgressSave = { workId, payload };
  clearTimeout(progressSaveTimer);
  progressSaveTimer = setTimeout(() => {
    progressSaveTimer = null;
    const p = pendingProgressSave;
    pendingProgressSave = null;
    if (p) persistProgress(p.workId, p.payload);
  }, 400);
}

function flushProgressSave() {
  clearTimeout(progressSaveTimer);
  progressSaveTimer = null;
  const p = pendingProgressSave;
  pendingProgressSave = null;
  if (p) persistProgress(p.workId, p.payload);
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
  if (!loc?.start?.cfi) return 0;
  let pct = 0;
  try {
    if (state.book?.locations && state.epubLocationsReady) {
      const frac = state.book.locations.percentageFromCfi(loc.start.cfi);
      if (Number.isFinite(frac)) pct = Math.round(Math.min(1, Math.max(0, frac)) * 100);
    } else if (Number.isFinite(loc.start.percentage)) {
      pct = Math.round(Math.min(1, Math.max(0, loc.start.percentage)) * 100);
    }
  } catch {
    if (Number.isFinite(loc.start.percentage)) {
      pct = Math.round(Math.min(1, Math.max(0, loc.start.percentage)) * 100);
    }
  }
  setReaderProgress(`${pct}% read`);
  return pct;
}

async function loadMarkdown(id) {
  const res = await fetch(`/api/works/${id}/content/markdown`, { credentials: "include" });
  const raw = await res.text();
  const area = document.getElementById("md-area");
  applyReaderTypography(loadTypo());
  populateMarkdownToc(raw);
  const prog = await api(`/api/works/${id}/progress?mode=markdown`).catch(() => null);
  if (prog?.percent) {
    area.scrollTop = (prog.percent / 100) * (area.scrollHeight - area.clientHeight);
  }
  updateMarkdownProgressUi(area);
  area.onscroll = () => {
    const percent = updateMarkdownProgressUi(area);
    scheduleProgressSave(id, { mode: "markdown", position: String(area.scrollTop), percent });
  };
}

async function loadEpub(w) {
  const area = document.getElementById("epub-area");
  area.innerHTML = "";
  setReaderProgress("");
  renderTocList([]);
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
    state.book = ePub(buf);
    state.epubLocationsReady = false;
    void area.offsetHeight;
    // Size to the stage area only — never window.innerHeight. The bottom
    // audio bar (when present) must shrink the page, not cover the last lines.
    const width = Math.max(area.clientWidth || 0, 320);
    const height = Math.max(area.clientHeight || 0, 240);
    state.rendition = state.book.renderTo(area, {
      width,
      height,
      flow: "paginated",
      allowScriptedContent: false,
    });
    applyReaderTypography(loadTypo());
    await state.book.ready;
    await populateEpubToc();

    const locKey = `diarch-epub-loc:${w.id}`;
    setReaderProgress("…");
    try {
      const stored = localStorage.getItem(locKey);
      if (stored) {
        state.book.locations.load(stored);
        state.epubLocationsReady = true;
      } else {
        await state.book.locations.generate(1024);
        state.epubLocationsReady = true;
        try {
          localStorage.setItem(locKey, state.book.locations.save());
        } catch {}
      }
    } catch {
      try {
        await state.book.locations.generate(1024);
        state.epubLocationsReady = true;
      } catch {
        state.epubLocationsReady = false;
      }
    }

    const prog = await api(`/api/works/${w.id}/progress?mode=epub`).catch(() => null);
    if (prog?.position) await state.rendition.display(prog.position);
    else await state.rendition.display();
    state.rendition.on("relocated", (loc) => {
      // Typography is applied on open and on typo-form changes; re-applying
      // it on every page turn is redundant and was causing jank.
      const percent = updateEpubProgressUi(loc);
      scheduleProgressSave(w.id, { mode: "epub", position: loc.start.cfi, percent });
    });
    try {
      const loc = state.rendition.currentLocation();
      if (loc) updateEpubProgressUi(loc);
    } catch {}
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
  if (dir === "left") {
    if (rtl) state.rendition.next();
    else state.rendition.prev();
  } else {
    if (rtl) state.rendition.prev();
    else state.rendition.next();
  }
}

function closeReaderView() {
  flushProgressSave();
  closeReaderMenu();
  closeReaderPanels();
  if (state.book) {
    try { state.book.destroy(); } catch {}
    state.book = null;
    state.rendition = null;
  }
  const audioEl = document.getElementById("audio-player");
  if (audioEl) {
    try { audioEl.pause(); } catch {}
    audioEl.removeAttribute("src");
    try { audioEl.load(); } catch {}
  }
  state.readerHasAudio = false;
  state.audioPlayerHidden = false;
  setAudioPlayerVisible(false);
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

document.getElementById("reader-audio-toggle")?.addEventListener("click", () => {
  closeReaderMenu();
  toggleAudioPlayer();
});

document.getElementById("audio-hide-btn")?.addEventListener("click", () => {
  setAudioPlayerVisible(false);
});

document.getElementById("reader-close").addEventListener("click", () => {
  closeReaderView();
});

document.getElementById("reader-toc-btn").addEventListener("click", () => {
  closeReaderMenu();
  const toc = document.getElementById("reader-toc");
  const typo = document.getElementById("reader-typo");
  if (typo) typo.hidden = true;
  if (toc) toc.hidden = !toc.hidden;
});

document.getElementById("reader-typo-btn").addEventListener("click", () => {
  closeReaderMenu();
  const toc = document.getElementById("reader-toc");
  const typo = document.getElementById("reader-typo");
  if (toc) toc.hidden = true;
  fillTypoForm(loadTypo());
  if (typo) typo.hidden = !typo.hidden;
});

document.getElementById("reader-toc-close").addEventListener("click", () => {
  document.getElementById("reader-toc").hidden = true;
});

document.getElementById("reader-typo-close").addEventListener("click", () => {
  document.getElementById("reader-typo").hidden = true;
});

document.getElementById("reader-typo-form").addEventListener("input", () => {
  const typo = readTypoForm();
  const sv = document.getElementById("typo-size-val");
  if (sv) sv.textContent = `${typo.size}%`;
  saveTypo(typo);
  applyReaderTypography(typo);
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
    closeReaderPanels();
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

let readerResizeTimer = null;
window.addEventListener("resize", () => {
  if (!readerIsOpen()) return;
  clearTimeout(readerResizeTimer);
  readerResizeTimer = setTimeout(() => resizeReaderEpub(), 100);
});

document.addEventListener("visibilitychange", () => {
  if (document.visibilityState === "hidden") flushProgressSave();
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
  try {
    await ensureZxing();
  } catch (e) {
    alert(e.message || "Barcode library failed to load");
    return;
  }
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

function renderIntegrations(health) {
  const tbody = document.querySelector("#integrations-table tbody");
  if (!tbody) return;
  if (!Array.isArray(health) || !health.length) {
    tbody.innerHTML = `<tr><td colspan="5" class="muted">No health rows yet — run Probe all</td></tr>`;
    return;
  }
  tbody.innerHTML = health
    .map((h) => {
      const st = h.status || "unknown";
      const okAt = h.last_ok_at
        ? new Date(h.last_ok_at).toLocaleString()
        : "—";
      return `<tr>
        <td><code>${escapeHtml(h.name)}</code></td>
        <td><span class="health-status health-${escapeHtml(st)}">${escapeHtml(st)}</span></td>
        <td class="muted">${escapeHtml(okAt)}</td>
        <td class="muted">${escapeHtml(h.last_error || "")}</td>
        <td><button type="button" class="btn-repair" data-name="${escapeHtml(h.name)}">Repair</button></td>
      </tr>`;
    })
    .join("");
  tbody.querySelectorAll(".btn-repair").forEach((btn) => {
    btn.addEventListener("click", async () => {
      const name = btn.dataset.name;
      btn.disabled = true;
      try {
        const r = await api(`/api/integrations/${encodeURIComponent(name)}/repair`, {
          method: "POST",
        });
        if (!r?.ok) alert(r?.error || `Repair failed for ${name}`);
      } catch (e) {
        alert(e.message || String(e));
      }
      await loadIntegrations();
    });
  });
}

function renderRemarkableStatus(st) {
  const el = document.getElementById("rm-status");
  const link = document.getElementById("rm-connect-link");
  if (link && st?.connect_url) link.href = st.connect_url;
  if (!el) return;
  if (!st) {
    el.textContent = "Could not load reMarkable status";
    el.classList.add("error");
    return;
  }
  el.classList.remove("error");
  const parts = [];
  parts.push(st.rmapi_installed ? "rmapi installed" : "rmapi missing");
  parts.push(st.authenticated ? "cloud authenticated" : "not authenticated");
  if (st.detail) parts.push(st.detail);
  el.textContent = parts.join(" · ");
  if (!st.authenticated || !st.rmapi_installed) el.classList.add("error");
}

async function loadRemarkableStatus() {
  try {
    const st = await api("/api/remarkable/status");
    renderRemarkableStatus(st);
    return st;
  } catch (e) {
    renderRemarkableStatus(null);
    const msg = document.getElementById("rm-auth-msg");
    if (msg) {
      msg.textContent = e.message || String(e);
      msg.classList.add("error");
    }
    return null;
  }
}

function renderStoryGraphStatus(st) {
  const el = document.getElementById("sg-status");
  const userInput = document.getElementById("sg-username");
  if (!el) return;
  if (!st) {
    el.textContent = "Could not load StoryGraph status";
    el.classList.add("error");
    return;
  }
  el.classList.remove("error");
  const parts = [];
  parts.push(st.username_set ? `user ${st.username || "set"}` : "username missing");
  parts.push(st.cookie_set ? `cookie ${st.cookie_hint || "set"}` : "cookie missing");
  if (st.cookie_set) {
    parts.push(st.remember_token ? "remember_user_token" : "no remember_user_token");
    parts.push(st.cf_clearance ? "cf_clearance" : "no cf_clearance");
  }
  if (st.detail) parts.push(st.detail);
  el.textContent = parts.join(" · ");
  if (!st.username_set || !st.cookie_set || st.verified === false) el.classList.add("error");
  if (userInput && st.username && !userInput.value) userInput.value = st.username;
}

async function loadStoryGraphStatus() {
  try {
    const st = await api("/api/storygraph/status");
    renderStoryGraphStatus(st);
    return st;
  } catch (e) {
    renderStoryGraphStatus(null);
    const msg = document.getElementById("sg-auth-msg");
    if (msg) {
      msg.textContent = e.message || String(e);
      msg.classList.add("error");
    }
    return null;
  }
}

async function loadIntegrations() {
  await loadRemarkableStatus();
  await loadStoryGraphStatus();
  try {
    const health = await api("/api/integrations");
    renderIntegrations(health);
  } catch (e) {
    const tbody = document.querySelector("#integrations-table tbody");
    if (tbody) {
      tbody.innerHTML = `<tr><td colspan="5" class="error">${escapeHtml(e.message || String(e))}</td></tr>`;
    }
  }
}

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

document.getElementById("rm-auth-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const msg = document.getElementById("rm-auth-msg");
  const code = String(document.getElementById("rm-code").value || "")
    .trim()
    .toLowerCase();
  msg.classList.remove("error");
  msg.textContent = "Connecting…";
  try {
    const st = await api("/api/remarkable/auth", {
      method: "POST",
      json: { code },
    });
    document.getElementById("rm-code").value = "";
    renderRemarkableStatus(st);
    msg.textContent = st.authenticated
      ? "Connected. You can Send to reMarkable from a work with an EPUB."
      : "Saved, but still not authenticated — try a fresh code.";
    await loadIntegrations();
  } catch (err) {
    msg.textContent = err.message || String(err);
    msg.classList.add("error");
  }
});

document.getElementById("btn-rm-refresh").addEventListener("click", () => loadRemarkableStatus());

document.getElementById("sg-auth-form")?.addEventListener("submit", async (e) => {
  e.preventDefault();
  const msg = document.getElementById("sg-auth-msg");
  const username = String(document.getElementById("sg-username").value || "").trim();
  const cookie = String(document.getElementById("sg-cookie").value || "").trim();
  const userAgent = String(document.getElementById("sg-user-agent")?.value || "").trim();
  msg.classList.remove("error");
  msg.textContent = "Saving & verifying against StoryGraph…";
  try {
    const st = await api("/api/storygraph/auth", {
      method: "POST",
      json: {
        username,
        cookie,
        user_agent: userAgent || undefined,
      },
    });
    document.getElementById("sg-cookie").value = "";
    renderStoryGraphStatus(st);
    if (st.verified) {
      msg.textContent = st.cf_clearance
        ? "Verified. Sync or Enrich should work until cf_clearance expires."
        : "Verified without cf_clearance — ok for now; if Cloudflare returns, re-paste a full Cookie header.";
    } else {
      msg.textContent = st.verify_error || st.detail || "Saved but live check failed.";
      msg.classList.add("error");
    }
    await loadIntegrations();
  } catch (err) {
    msg.textContent = err.message || String(err);
    msg.classList.add("error");
  }
});

document.getElementById("btn-sg-refresh")?.addEventListener("click", () => loadStoryGraphStatus());

document.getElementById("btn-sg-sync").addEventListener("click", async () => {
  const report = document.getElementById("sg-sync-report");
  report.hidden = false;
  report.classList.remove("error");
  report.textContent = "Syncing StoryGraph…";
  try {
    const r = await api("/api/storygraph/sync", { method: "POST" });
    report.textContent =
      `SG books: ${r.sg_count ?? 0} · matched: ${r.matched ?? 0} · ` +
      `missing locally: ${r.missing_locally ?? 0} · needs add on SG: ${r.needs_add ?? 0}` +
      (r.audio_only_remote ? ` · SG audio-only: ${r.audio_only_remote}` : "");
    await loadIntegrations();
  } catch (e) {
    report.textContent = e.message || String(e);
    report.classList.add("error");
  }
});

document.getElementById("btn-tts-export").addEventListener("click", async () => {
  const r = await api("/api/queue/needs_tts/export", { method: "POST" });
  const files = Array.isArray(r.files) ? r.files : [];
  const list = files.length
    ? files.map((f) => `• ${f}`).join("\n")
    : "(no EPUBs with needs_tts)";
  alert(`Exported ${r.exported ?? 0} titled EPUB(s) to queue/needs_tts:\n${list}`);
});

function wireAudioSpeed(audioEl, rangeId, labelId) {
  if (!audioEl) return;
  const range = document.getElementById(rangeId);
  const label = document.getElementById(labelId);
  if (!range || !label) return;
  const saved = Number(localStorage.getItem("diarch-audio-speed") || "1");
  const initial = Number.isFinite(saved) ? Math.min(2.5, Math.max(0.25, saved)) : 1;
  range.value = String(initial);
  const apply = () => {
    const rate = Number(range.value);
    audioEl.playbackRate = rate;
    label.textContent = `${rate.toFixed(2)}×`;
    localStorage.setItem("diarch-audio-speed", String(rate));
  };
  range.oninput = apply;
  audioEl.onloadedmetadata = apply;
  apply();
}

document.getElementById("btn-probe").addEventListener("click", async () => {
  const btn = document.getElementById("btn-probe");
  btn.disabled = true;
  try {
    const health = await api("/api/integrations/probe", { method: "POST" });
    renderIntegrations(health);
    await loadRemarkableStatus();
  } catch (e) {
    alert(e.message || String(e));
    await loadIntegrations();
  } finally {
    btn.disabled = false;
  }
});

boot();
