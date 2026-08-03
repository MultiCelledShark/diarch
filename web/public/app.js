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
    if (v === "integrations") await loadIntegrations();
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
    if (w.needs_cover) badges.push("needs cover");
    if (state.settings?.show_audio_gaps && w.needs_audio) badges.push("no audio");
    if (w.sg_review_dirty) badges.push("update SG");
    if (w.sg_needs_add) badges.push("add to SG");
    if (w.sg_audio_only_remote) badges.push("SG audio only");
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

async function openDetail(id) {
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
  let transcriptBadge = "";
  if (data.has_transcript) transcriptBadge = " · Transcript ready";
  else if (trBusy || w.needs_transcription) transcriptBadge = " · Transcribing…";
  else if (trFailed) transcriptBadge = " · Transcription failed";

  document.getElementById("detail").innerHTML = `
    <div class="detail-covers">
      <img src="/api/works/${w.id}/cover?t=${Date.now()}" alt="" />
      ${data.has_cover_candidate ? `
        <div class="cover-candidate">
          <p class="muted">AI candidate</p>
          <img src="/api/works/${w.id}/cover/candidate?t=${Date.now()}" alt="Candidate cover" />
          <div class="row">
            <button type="button" id="btn-approve-cover">Approve cover</button>
            <button type="button" id="btn-discard-cover">Discard</button>
          </div>
        </div>` : ""}
    </div>
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
      <p class="muted">${hasEpub ? "EPUB ready" : "No EPUB yet"} · ${hasMd ? "Markdown ready" : "No Markdown"} · ${hasAudio ? "M4B ready" : (w.needs_audio ? "Needs audio" : "No audio")} · Direction: ${escapeHtml(w.reading_direction)}${w.needs_review ? " · Needs review" : ""}${w.needs_cover ? " · Needs cover" : ""}${transcriptBadge}${w.sg_matched ? " · On StoryGraph" : ""}${w.sg_review_dirty ? " · Update SG review" : ""}${w.sg_needs_add ? " · Add to StoryGraph" : ""}${w.sg_audio_only_remote ? " · SG audio, missing local" : ""}</p>
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
        <button type="button" id="btn-fetch-cover"${w.isbn ? "" : " disabled title=\"Add an ISBN first\""}>Fetch cover</button>
        <button type="button" id="btn-generate-cover" title="Generate via LocalAI (staged — approve before it replaces the cover)">Generate cover</button>
        <label class="btn-file">Upload cover <input type="file" id="cover-file" accept="image/*" hidden /></label>
        ${w.needs_cover ? `<button type="button" id="btn-clear-needs-cover">Dismiss needs cover</button>` : ""}
        ${w.sg_review_dirty ? `<button type="button" id="btn-clear-sg-review" title="Clear after you update StoryGraph">Clear SG review flag</button>` : ""}
        ${w.sg_needs_add ? `<button type="button" id="btn-clear-sg-add" title="Clear after you add this book on StoryGraph">Clear add-to-SG flag</button>` : ""}
        ${w.sg_audio_only_remote ? `<button type="button" id="btn-clear-sg-audio">Dismiss SG audio gap</button>` : ""}
        <label class="btn-file">Upload audiobook (.m4b / .aax) <input type="file" id="audio-file" accept=".m4b,.aax,audio/mp4" hidden /></label>
        <button type="button" id="btn-transcribe"${hasAudio && !trBusy ? "" : ` disabled title="${!hasAudio ? "Upload an audiobook first" : "Transcription already in progress"}"`}>${trFailed ? "Retry transcription" : "Transcribe"}</button>
        ${data.has_transcript ? `<a href="/api/works/${w.id}/transcript"><button type="button">Download transcript</button></a>` : ""}
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
        ${(state.user.is_admin || w.created_by === state.user.id)
          ? `<button type="button" id="btn-delete-work" class="danger">Delete book</button>`
          : ""}
      </div>
      ${w.needs_review ? `
      <section class="review-workspace" aria-label="Import review">
        <div class="review-pane review-pdf">
          <h3>Source PDF</h3>
          ${hasImportPdf
            ? `<iframe title="Quarantined PDF" src="/api/works/${w.id}/content/pdf"></iframe>`
            : `<p class="muted">No quarantine PDF (markdown-only import).</p>`}
        </div>
        <div class="review-pane review-md">
          <h3>Markdown</h3>
          <div class="review-md-toolbar" role="toolbar" aria-label="Markdown formatting">
            <button type="button" data-md-cmd="bold" title="Bold (Ctrl+B)"><strong>B</strong></button>
            <button type="button" data-md-cmd="italic" title="Italic (Ctrl+I)"><em>I</em></button>
            <button type="button" data-md-cmd="heading" title="Heading">H</button>
            <button type="button" data-md-cmd="link" title="Link">Link</button>
            <button type="button" data-md-cmd="quote" title="Quote">“</button>
            <button type="button" data-md-cmd="ul" title="Bullet list">• List</button>
            <button type="button" data-md-cmd="ol" title="Numbered list">1. List</button>
            <button type="button" data-md-cmd="code" title="Inline code">Code</button>
            <button type="button" data-md-cmd="fence" title="Code block">Block</button>
            <span class="tb-sep" aria-hidden="true"></span>
            <button type="button" data-md-cmd="find" title="Find / Replace">Find</button>
          </div>
          <div id="review-find" class="review-find" hidden>
            <input type="text" id="review-find-q" placeholder="Find" />
            <input type="text" id="review-find-r" placeholder="Replace" />
            <button type="button" id="review-find-next">Next</button>
            <button type="button" id="review-find-replace">Replace</button>
            <button type="button" id="review-find-all">Replace all</button>
            <button type="button" id="review-find-close">Close</button>
          </div>
          <textarea id="review-md-editor" spellcheck="false" placeholder="Loading…"></textarea>
          <div class="review-actions">
            <button type="button" id="btn-save-md">Save markdown</button>
            <span id="review-dirty" class="review-dirty" hidden>Unsaved changes</span>
            <button type="button" id="btn-preview-md">Preview</button>
            <a href="/api/works/${w.id}/download/markdown"><button type="button">Download MD</button></a>
            <button type="button" id="btn-confirm">Confirm → EPUB</button>
          </div>
        </div>
      </section>
      ` : ""}
      <pre id="detail-msg"></pre>
    </div>`;

  const msg = (t, isError = false) => {
    const el = document.getElementById("detail-msg");
    el.textContent = t || "";
    el.classList.toggle("error", !!isError);
  };

  if (trBusy) {
    if (state._transcribePoll) clearTimeout(state._transcribePoll);
    state._transcribePoll = setTimeout(() => {
      if (state.currentWork?.work?.id === w.id) openDetail(w.id);
    }, 4000);
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
    const form = document.getElementById("detail-form");
    if (!form || !hit) return;
    form.title.value = hit.title || form.title.value;
    if (hit.authors) form.authors.value = hit.authors;
    if (hit.isbn) form.isbn.value = hit.isbn;
    if (hit.description && form.description) form.description.value = hit.description;
    await api(`/api/works/${w.id}`, {
      method: "PUT",
      json: {
        title: form.title.value,
        authors: form.authors.value,
        isbn: form.isbn.value || null,
        description: form.description?.value || hit.description || null,
      },
    });
    msg(`Applied metadata from ${hit.source || "lookup"}`);
    await openDetail(w.id);
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
    box.innerHTML = `<p class="muted">Pick a match:</p>` + hits.map((h, i) => `
      <button type="button" class="meta-hit" data-hit="${i}">
        <strong>${escapeHtml(h.title || "Untitled")}</strong>
        <span>${escapeHtml(h.authors || "Unknown author")}</span>
        <span class="muted">${escapeHtml(h.source || "")}${h.isbn ? " · " + escapeHtml(h.isbn) : ""}</span>
      </button>`).join("");
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
    msg(isbn ? `Looking up ISBN ${isbn}…` : "Searching Open Library / Google Books / LoC…");
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
      const hits = await api(`/api/metadata/search?${q}`);
      if (!Array.isArray(hits) || !hits.length) {
        const base = providerNote
          ? `${providerNote}; title search also empty`
          : "No metadata match found";
        const hint = /rate limited|DIARCH_GOOGLE_BOOKS_KEY/i.test(providerNote)
          ? " — set DIARCH_GOOGLE_BOOKS_KEY for Google Books quota"
          : "";
        msg(base + hint, true);
        return;
      }
      if (hits.length === 1) {
        await applyMetaHit(hits[0]);
        return;
      }
      msg(`Found ${hits.length} matches — pick one`);
      showMetaHits(hits);
    } catch (e) {
      msg(e.message || String(e), true);
    }
  });

  document.getElementById("btn-fetch-cover")?.addEventListener("click", async () => {
    msg("Fetching cover from Open Library / Google Books…");
    try {
      const r = await api(`/api/works/${w.id}/cover/fetch`, { method: "POST" });
      if (!r?.ok) {
        msg(r?.message || "No remote cover found", true);
        return;
      }
      msg("Cover fetched");
      await openDetail(w.id);
    } catch (e) {
      msg(e.message || String(e), true);
    }
  });

  document.getElementById("btn-generate-cover")?.addEventListener("click", async () => {
    const promptEl = document.getElementById("cover-prompt");
    msg("Generating cover via LocalAI (may take a minute)…");
    try {
      const r = await api(`/api/works/${w.id}/cover/generate`, { method: "POST" });
      if (r?.prompt && promptEl) {
        promptEl.hidden = false;
        promptEl.textContent = `Prompt: ${r.prompt}`;
      }
      if (!r?.ok) {
        msg(r?.message || "Cover generation failed", true);
        return;
      }
      msg("Candidate ready — approve or discard");
      await openDetail(w.id);
    } catch (e) {
      msg(e.message || String(e), true);
    }
  });

  document.getElementById("btn-approve-cover")?.addEventListener("click", async () => {
    msg("Approving candidate cover…");
    try {
      const r = await api(`/api/works/${w.id}/cover/approve`, { method: "POST" });
      if (!r?.ok) {
        msg(r?.message || "Approve failed", true);
        return;
      }
      msg("Cover approved");
      await openDetail(w.id);
    } catch (e) {
      msg(e.message || String(e), true);
    }
  });

  document.getElementById("btn-discard-cover")?.addEventListener("click", async () => {
    await api(`/api/works/${w.id}/cover/discard`, { method: "POST" });
    msg("Candidate discarded");
    await openDetail(w.id);
  });

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

  document.getElementById("btn-clear-needs-cover")?.addEventListener("click", async () => {
    await api(`/api/works/${w.id}/flags/clear`, {
      method: "POST",
      json: { flags: ["needs_cover"] },
    });
    msg("Cleared needs_cover");
    await openDetail(w.id);
  });
  async function clearSgFlag(flag, label) {
    await api(`/api/works/${w.id}/flags/clear`, {
      method: "POST",
      json: { flags: [flag] },
    });
    msg(label);
    await openDetail(w.id);
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
    const editor = document.getElementById("review-md-editor");
    if (editor) {
      const res = await fetch(`/api/works/${w.id}/content/markdown`, {
        method: "PUT",
        credentials: "include",
        headers: { "Content-Type": "text/markdown; charset=utf-8" },
        body: editor.value,
      });
      if (!res.ok) {
        msg(await res.text() || "Could not save markdown before confirm", true);
        return;
      }
      setReviewDirty(false, editor.value);
    }
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
  });

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
  show("library");
  loadWorks();
});


/* —— Review markdown editor —— */
let reviewSavedText = "";
let reviewPreviewWin = null;

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

function openMdPreviewFloat(editor) {
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
  fillMdPreviewBody(editor?.value || "");

  document.getElementById("md-preview-close")?.addEventListener("click", closeMdPreviewFloat);
  document.getElementById("md-preview-refresh")?.addEventListener("click", () => {
    fillMdPreviewBody(document.getElementById("review-md-editor")?.value || "");
  });
  document.getElementById("md-preview-tab")?.addEventListener("click", () => {
    openMdPreviewTab(document.getElementById("review-md-editor")?.value || "");
  });

  // Drag by header
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

function mdWrapSelection(editor, before, after = before) {
  const start = editor.selectionStart;
  const end = editor.selectionEnd;
  const val = editor.value;
  const selected = val.slice(start, end) || "text";
  const next = val.slice(0, start) + before + selected + after + val.slice(end);
  editor.value = next;
  editor.focus();
  editor.setSelectionRange(start + before.length, start + before.length + selected.length);
  editor.dispatchEvent(new Event("input"));
}

function mdPrefixLines(editor, prefixFn) {
  const start = editor.selectionStart;
  const end = editor.selectionEnd;
  const val = editor.value;
  const lineStart = val.lastIndexOf("\n", start - 1) + 1;
  const lineEnd = (() => {
    const i = val.indexOf("\n", end);
    return i === -1 ? val.length : i;
  })();
  const block = val.slice(lineStart, lineEnd);
  const lines = block.split("\n");
  const replaced = lines.map((line, i) => prefixFn(line, i)).join("\n");
  editor.value = val.slice(0, lineStart) + replaced + val.slice(lineEnd);
  editor.focus();
  editor.setSelectionRange(lineStart, lineStart + replaced.length);
  editor.dispatchEvent(new Event("input"));
}

function runMdCommand(cmd, editor) {
  if (!editor) return;
  switch (cmd) {
    case "bold":
      mdWrapSelection(editor, "**");
      break;
    case "italic":
      mdWrapSelection(editor, "*");
      break;
    case "heading":
      mdPrefixLines(editor, (line) => {
        if (/^#{1,6}\s/.test(line)) {
          const m = line.match(/^(#{1,6})\s/);
          const n = Math.min(6, (m[1].length % 6) + 1);
          return "#".repeat(n) + " " + line.replace(/^#{1,6}\s*/, "");
        }
        return "## " + line;
      });
      break;
    case "link": {
      const start = editor.selectionStart;
      const end = editor.selectionEnd;
      const selected = editor.value.slice(start, end) || "label";
      const insert = `[${selected}](https://)`;
      editor.value = editor.value.slice(0, start) + insert + editor.value.slice(end);
      const urlStart = start + selected.length + 3;
      editor.focus();
      editor.setSelectionRange(urlStart, urlStart + "https://".length);
      editor.dispatchEvent(new Event("input"));
      break;
    }
    case "quote":
      mdPrefixLines(editor, (line) => (line.startsWith("> ") ? line : "> " + line));
      break;
    case "ul":
      mdPrefixLines(editor, (line) => (line.startsWith("- ") ? line : "- " + line));
      break;
    case "ol":
      mdPrefixLines(editor, (line, i) => (/^\d+\.\s/.test(line) ? line : `${i + 1}. ${line}`));
      break;
    case "code":
      mdWrapSelection(editor, "`");
      break;
    case "fence": {
      const start = editor.selectionStart;
      const end = editor.selectionEnd;
      const selected = editor.value.slice(start, end) || "code";
      const insert = "```\n" + selected + "\n```";
      editor.value = editor.value.slice(0, start) + insert + editor.value.slice(end);
      editor.focus();
      editor.setSelectionRange(start + 4, start + 4 + selected.length);
      editor.dispatchEvent(new Event("input"));
      break;
    }
    case "find": {
      const box = document.getElementById("review-find");
      if (box) {
        box.hidden = !box.hidden;
        if (!box.hidden) document.getElementById("review-find-q")?.focus();
      }
      break;
    }
    default:
      break;
  }
}

function findInEditor(editor, query, from) {
  if (!query) return -1;
  return editor.value.indexOf(query, from);
}

function wireReviewMarkdownEditor(workId, msg) {
  const editor = document.getElementById("review-md-editor");
  if (!editor) return;
  const confirmBtn = document.getElementById("btn-confirm");
  if (confirmBtn) confirmBtn.disabled = false;

  closeMdPreviewFloat();

  fetch(`/api/works/${workId}/content/markdown`, { credentials: "include" })
    .then(async (res) => {
      editor.value = res.ok ? await res.text() : "";
      if (!res.ok) msg(`Could not load markdown (${res.status})`, true);
      setReviewDirty(false, editor.value);
    })
    .catch((e) => msg(e.message || String(e), true));

  editor.addEventListener("input", () => {
    setReviewDirty(editor.value !== reviewSavedText);
  });

  document.querySelectorAll("[data-md-cmd]").forEach((btn) => {
    btn.addEventListener("click", () => runMdCommand(btn.dataset.mdCmd, editor));
  });

  editor.addEventListener("keydown", (e) => {
    const mod = e.ctrlKey || e.metaKey;
    if (!mod) return;
    if (e.key === "s") {
      e.preventDefault();
      document.getElementById("btn-save-md")?.click();
    } else if (e.key === "b") {
      e.preventDefault();
      runMdCommand("bold", editor);
    } else if (e.key === "i") {
      e.preventDefault();
      runMdCommand("italic", editor);
    } else if (e.key === "f") {
      e.preventDefault();
      runMdCommand("find", editor);
    }
  });

  document.getElementById("btn-save-md")?.addEventListener("click", async () => {
    msg("Saving markdown…");
    const res = await fetch(`/api/works/${workId}/content/markdown`, {
      method: "PUT",
      credentials: "include",
      headers: { "Content-Type": "text/markdown; charset=utf-8" },
      body: editor.value,
    });
    if (!res.ok) {
      msg((await res.text()) || `Save failed (${res.status})`, true);
      return;
    }
    setReviewDirty(false, editor.value);
    msg("Markdown saved (still needs confirm)");
  });

  document.getElementById("btn-preview-md")?.addEventListener("click", () => {
    openMdPreviewFloat(editor);
  });

  const findNext = () => {
    const q = document.getElementById("review-find-q")?.value || "";
    if (!q) return;
    let idx = findInEditor(editor, q, editor.selectionEnd);
    if (idx < 0) idx = findInEditor(editor, q, 0);
    if (idx < 0) {
      msg("No matches", true);
      return;
    }
    editor.focus();
    editor.setSelectionRange(idx, idx + q.length);
  };
  document.getElementById("review-find-next")?.addEventListener("click", findNext);
  document.getElementById("review-find-replace")?.addEventListener("click", () => {
    const q = document.getElementById("review-find-q")?.value || "";
    const r = document.getElementById("review-find-r")?.value ?? "";
    if (!q) return;
    const start = editor.selectionStart;
    const end = editor.selectionEnd;
    if (editor.value.slice(start, end) === q) {
      editor.value = editor.value.slice(0, start) + r + editor.value.slice(end);
      editor.setSelectionRange(start, start + r.length);
      editor.dispatchEvent(new Event("input"));
    }
    findNext();
  });
  document.getElementById("review-find-all")?.addEventListener("click", () => {
    const q = document.getElementById("review-find-q")?.value || "";
    const r = document.getElementById("review-find-r")?.value ?? "";
    if (!q) return;
    if (!editor.value.includes(q)) {
      msg("No matches", true);
      return;
    }
    editor.value = editor.value.split(q).join(r);
    editor.dispatchEvent(new Event("input"));
    msg("Replaced all");
  });
  document.getElementById("review-find-close")?.addEventListener("click", () => {
    const box = document.getElementById("review-find");
    if (box) box.hidden = true;
  });
}

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
};

const PALETTES = {
  dark: { ink: "#f3e6d4", pageBg: "#0c1210", link: "#e8b87a" },
  sepia: { ink: "#5b4636", pageBg: "#f4ecd8", link: "#8a5a2b" },
  light: { ink: "#1a1a1a", pageBg: "#fafafa", link: "#8a5a2b" },
};

const FONTS = {
  serif: '"Iowan Old Style", "Palatino Linotype", Palatino, Georgia, serif',
  sans: '"Source Sans 3", "Segoe UI", system-ui, sans-serif',
  mono: 'ui-monospace, "Cascadia Code", "Source Code Pro", monospace',
};

const LINE_HEIGHTS = { tight: 1.35, normal: 1.7, loose: 2.0 };
const MEASURES = {
  narrow: { maxCh: 52, padX: "max(2.5rem, calc(50vw - 14rem))", epubMargin: "12%" },
  medium: { maxCh: 68, padX: "max(1rem, calc(50vw - 18rem))", epubMargin: "6%" },
  wide: { maxCh: 88, padX: "1rem", epubMargin: "2%" },
};

function loadTypo() {
  try {
    const raw = localStorage.getItem(TYPO_KEY);
    if (!raw) return { ...TYPO_DEFAULTS };
    return { ...TYPO_DEFAULTS, ...JSON.parse(raw) };
  } catch {
    return { ...TYPO_DEFAULTS };
  }
}

function saveTypo(typo) {
  try {
    localStorage.setItem(TYPO_KEY, JSON.stringify(typo));
  } catch {}
}

function fillTypoForm(typo) {
  const form = document.getElementById("reader-typo-form");
  if (!form) return;
  form.palette.value = typo.palette;
  form.font.value = typo.font;
  form.size.value = typo.size;
  form.lineHeight.value = typo.lineHeight;
  form.measure.value = typo.measure;
  form.justify.checked = !!typo.justify;
  const sv = document.getElementById("typo-size-val");
  if (sv) sv.textContent = `${typo.size}%`;
}

function readTypoForm() {
  const form = document.getElementById("reader-typo-form");
  if (!form) return loadTypo();
  return {
    palette: form.palette.value || "dark",
    font: form.font.value || "serif",
    size: Number(form.size.value) || 100,
    lineHeight: form.lineHeight.value || "normal",
    measure: form.measure.value || "medium",
    justify: !!form.justify.checked,
  };
}

function applyReaderTypography(typo) {
  const t = typo || loadTypo();
  state.readerTypo = t;
  const pal = PALETTES[t.palette] || PALETTES.dark;
  const ff = FONTS[t.font] || FONTS.serif;
  const lh = LINE_HEIGHTS[t.lineHeight] || LINE_HEIGHTS.normal;
  const measure = MEASURES[t.measure] || MEASURES.medium;
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
    md.style.background = pal.pageBg;
    md.style.color = pal.ink;
  }
  if (state.rendition) {
    applyEpubTypography(t, pal, ff, lh, measure);
  }
}

function applyEpubTypography(t, pal, ff, lh, measure) {
  if (!state.rendition) return;
  pal = pal || PALETTES[(t || state.readerTypo || {}).palette] || PALETTES.dark;
  ff = ff || FONTS[(t || state.readerTypo || {}).font] || FONTS.serif;
  lh = lh || LINE_HEIGHTS[(t || state.readerTypo || {}).lineHeight] || LINE_HEIGHTS.normal;
  measure = measure || MEASURES[(t || state.readerTypo || {}).measure] || MEASURES.medium;
  t = t || state.readerTypo || TYPO_DEFAULTS;
  const align = t.justify ? "justify" : "start";
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
    "text-align": `${align} !important`,
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
      },
      p: { "text-align": `${align} !important` },
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

  const audioWrap = document.getElementById("audio-wrap");
  const audioEl = document.getElementById("audio-player");
  if (audio) {
    audioWrap.hidden = false;
    const name = (audio.relative_path || "book.m4b").split("/").pop() || "book.m4b";
    audioEl.src = `/api/works/${w.id}/audio/${name}`;
    wireAudioSpeed(audioEl, "audio-speed", "audio-speed-val");
  } else {
    audioWrap.hidden = true;
    audioEl.removeAttribute("src");
  }

  if (audioOnly) return;
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
    const width = Math.max(area.clientWidth || 0, window.innerWidth || 320);
    const height = Math.max(area.clientHeight || 0, (window.innerHeight || 480) - 56);
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
      applyEpubTypography(state.readerTypo || loadTypo());
      const percent = updateEpubProgressUi(loc);
      api(`/api/works/${w.id}/progress`, {
        method: "PUT",
        json: { mode: "epub", position: loc.start.cfi, percent },
      }).catch(() => {});
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
  closeReaderMenu();
  closeReaderPanels();
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

async function loadIntegrations() {
  await loadRemarkableStatus();
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
