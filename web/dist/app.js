const state = {
  user: null,
  settings: null,
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
  } catch (e) {
    console.warn("settings load failed", e);
    state.settings = { show_audio_gaps: true, reader_infinite_scroll: false };
  }
  try {
    await loadWorks();
  } catch (e) {
    console.error("library load failed", e);
    const el = document.getElementById("work-list");
    el.innerHTML = `<p class="empty error">Could not load library: ${escapeHtml(e.message || String(e))}</p>`;
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
  } catch (ex) {
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
    emptyHint: "Use New work to add a book, or open Wishlist to add by ISBN / title.",
  });
}

document.getElementById("filter-status").addEventListener("change", loadWorks);

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
    emptyHint: "Flags appear here when reviews need StoryGraph updates, covers are missing, etc.",
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
    if (w.status === "wishlist") badges.push("wishlist");
    if (w.primary_code) badges.push(`#${w.primary_code}`);
    if (w.needs_review) badges.push("review");
    if (state.settings?.show_audio_gaps && w.needs_audio) badges.push("no audio");
    if (w.sg_review_dirty) badges.push("SG review");
    if (w.is_manga) badges.push("manga");
    card.innerHTML = `
      <img src="/api/works/${w.id}/cover" alt="" onerror="this.style.opacity=0.3" />
      <div class="meta">
        <strong>${escapeHtml(w.title)}</strong>
        <span>${escapeHtml(w.authors || "")}</span>
        <div>${badges.map((b) => `<span class="badge">${b}</span>`).join("")}</div>
      </div>`;
    card.addEventListener("click", () => openDetail(w.id));
    el.appendChild(card);
  }
}

function escapeHtml(s) {
  return String(s)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

document.getElementById("btn-new-work").addEventListener("click", async () => {
  const title = prompt("Title?");
  if (!title) return;
  const authors = prompt("Author(s)?") || "";
  const code = prompt("Primary taxonomy code? (e.g. 8201 for Epic Fantasy)") || "";
  const codes = code ? [Number(code)] : [];
  await api("/api/works", {
    method: "POST",
    json: {
      title,
      authors,
      codes,
      primary_code: codes[0] || null,
    },
  });
  await loadWorks();
});

async function openDetail(id) {
  const data = await api(`/api/works/${id}`);
  state.currentWork = data;
  show("detail");
  const w = data.work;
  const assets = data.assets || [];
  const hasEpub = assets.some((a) => a.kind === "epub");
  const hasMd = assets.some((a) => a.kind === "markdown");
  const audio = assets.find((a) => a.kind === "audio");
  document.getElementById("detail").innerHTML = `
    <img src="/api/works/${w.id}/cover" alt="" />
    <div>
      <h2>${escapeHtml(w.title)}</h2>
      <p>${escapeHtml(w.authors || "")}</p>
      <p class="muted">ISBN: ${escapeHtml(w.isbn || "—")} · Status: ${w.status}
        · Primary: ${w.primary_code ?? "—"} · Codes: ${(data.codes || []).join(", ")}</p>
      <p>${escapeHtml(w.description || "")}</p>
      <div class="actions">
        ${hasEpub || hasMd ? `<button id="btn-read">Read</button>` : ""}
        <label class="btn-file">Import <input type="file" id="import-file" hidden /></label>
        ${w.needs_review ? `<button id="btn-confirm">Confirm → EPUB</button>` : ""}
        ${hasEpub ? `<a href="/api/works/${w.id}/download/epub"><button type="button">Download EPUB</button></a>` : ""}
        ${hasMd ? `<a href="/api/works/${w.id}/download/markdown"><button type="button">Download MD</button></a>` : ""}
        <button id="btn-cover-gen">Generate cover</button>
        <button id="btn-cover-prompt">Copy cover prompt</button>
        <label class="btn-file">Upload cover <input type="file" id="cover-file" accept="image/*" hidden /></label>
        <label class="btn-file">Upload audio <input type="file" id="audio-file" accept="audio/*,.m4a" hidden /></label>
        ${audio ? `<button id="btn-play-audio">Play audio</button>` : ""}
        ${hasEpub ? `<button id="btn-remarkable">Send to reMarkable</button>` : ""}
        ${state.user.is_admin ? `<button id="btn-grant">Grant to user…</button>` : ""}
        <button id="btn-flag-tts">Flag needs TTS</button>
        <button id="btn-year">Add to year list…</button>
      </div>
      <pre id="detail-msg"></pre>
    </div>`;

  const msg = (t) => (document.getElementById("detail-msg").textContent = t || "");

  document.getElementById("btn-read")?.addEventListener("click", () => openReader(w, hasEpub, hasMd, audio));
  document.getElementById("import-file")?.addEventListener("change", async (e) => {
    const file = e.target.files[0];
    if (!file) return;
    const fd = new FormData();
    fd.append("file", file);
    const res = await fetch(`/api/works/${w.id}/import`, { method: "POST", body: fd, credentials: "include" });
    const j = await res.json();
    msg(`Import job ${j.job_id}`);
    pollJob(j.job_id, () => openDetail(w.id));
  });
  document.getElementById("btn-confirm")?.addEventListener("click", async () => {
    const j = await api(`/api/works/${w.id}/confirm`, { method: "POST" });
    msg(`Confirm job ${j.job_id}`);
    pollJob(j.job_id, () => openDetail(w.id));
  });
  document.getElementById("btn-cover-gen")?.addEventListener("click", async () => {
    const j = await api(`/api/works/${w.id}/cover/generate`, { method: "POST" });
    msg(j.ok ? "Cover generated" : `Prompt: ${j.prompt}`);
    if (j.ok) openDetail(w.id);
  });
  document.getElementById("btn-cover-prompt")?.addEventListener("click", async () => {
    const j = await api(`/api/works/${w.id}/cover/prompt`);
    await navigator.clipboard.writeText(j.prompt);
    msg("Prompt copied");
  });
  document.getElementById("cover-file")?.addEventListener("change", async (e) => {
    const file = e.target.files[0];
    if (!file) return;
    const fd = new FormData();
    fd.append("file", file);
    await fetch(`/api/works/${w.id}/cover`, { method: "POST", body: fd, credentials: "include" });
    openDetail(w.id);
  });
  document.getElementById("audio-file")?.addEventListener("change", async (e) => {
    const file = e.target.files[0];
    if (!file) return;
    const fd = new FormData();
    fd.append("file", file);
    await fetch(`/api/works/${w.id}/audio`, { method: "POST", body: fd, credentials: "include" });
    openDetail(w.id);
  });
  document.getElementById("btn-play-audio")?.addEventListener("click", () => {
    openReader(w, hasEpub, hasMd, audio);
  });
  document.getElementById("btn-remarkable")?.addEventListener("click", async () => {
    const j = await api(`/api/works/${w.id}/remarkable`, { method: "POST" });
    msg(j.ok ? "Sent" : j.error);
  });
  document.getElementById("btn-grant")?.addEventListener("click", async () => {
    const uid = prompt("User UUID to grant?");
    if (!uid) return;
    await api(`/api/works/${w.id}/grants`, { method: "POST", json: { user_id: uid } });
    msg("Granted");
  });
  document.getElementById("btn-flag-tts")?.addEventListener("click", async () => {
    await api(`/api/works/${w.id}`, { method: "PUT", json: { needs_tts: true } });
    msg("Flagged needs_tts");
  });
  document.getElementById("btn-year")?.addEventListener("click", async () => {
    const y = Number(prompt("Year?", String(new Date().getFullYear())));
    await api(`/api/works/${w.id}`, { method: "PUT", json: { year_list: y } });
    msg(`Year list ${y}`);
  });
}

document.getElementById("back-library").addEventListener("click", () => {
  show("library");
  loadWorks();
});

async function pollJob(id, done) {
  for (let i = 0; i < 60; i++) {
    await new Promise((r) => setTimeout(r, 1000));
    const j = await api(`/api/jobs/${id}`);
    if (j.status === "done" || j.status === "failed") {
      done();
      return;
    }
  }
}

async function openReader(w, hasEpub, hasMd, audio) {
  state.readerWorkId = w.id;
  show("reader");
  document.getElementById("reader-title").textContent = w.title;
  const scrollPref = state.settings?.reader_infinite_scroll;
  const scrollBox = document.getElementById("reader-scroll");
  scrollBox.checked = !!scrollPref && hasMd;
  const useMd = scrollBox.checked && hasMd;
  document.getElementById("epub-area").hidden = useMd;
  document.getElementById("md-area").hidden = !useMd;

  const audioEl = document.getElementById("audio-player");
  if (audio) {
    audioEl.hidden = false;
    audioEl.src = `/api/works/${w.id}/audio/${audio.relative_path.split("/").pop()}`;
  } else {
    audioEl.hidden = true;
  }

  if (useMd) {
    await loadMarkdown(w.id);
  } else if (hasEpub) {
    await loadEpub(w);
  } else if (hasMd) {
    scrollBox.checked = true;
    document.getElementById("epub-area").hidden = true;
    document.getElementById("md-area").hidden = false;
    await loadMarkdown(w.id);
  }

  scrollBox.onchange = async () => {
    if (scrollBox.checked && hasMd) {
      if (state.rendition) {
        state.book?.destroy?.();
        state.rendition = null;
      }
      document.getElementById("epub-area").hidden = true;
      document.getElementById("md-area").hidden = false;
      await loadMarkdown(w.id);
    } else if (hasEpub) {
      document.getElementById("md-area").hidden = true;
      document.getElementById("epub-area").hidden = false;
      await loadEpub(w);
    }
  };
}

async function loadMarkdown(id) {
  const res = await fetch(`/api/works/${id}/content/markdown`, { credentials: "include" });
  const text = await res.text();
  document.getElementById("md-area").textContent = text;
  const prog = await api(`/api/works/${id}/progress?mode=markdown`).catch(() => null);
  if (prog?.percent) {
    const area = document.getElementById("md-area");
    area.scrollTop = (prog.percent / 100) * (area.scrollHeight - area.clientHeight);
  }
  document.getElementById("md-area").onscroll = () => {
    const area = document.getElementById("md-area");
    const percent = area.scrollHeight <= area.clientHeight
      ? 100
      : (area.scrollTop / (area.scrollHeight - area.clientHeight)) * 100;
    api(`/api/works/${id}/progress`, {
      method: "PUT",
      json: { mode: "markdown", position: String(area.scrollTop), percent },
    }).catch(() => {});
  };
}

async function loadEpub(w) {
  const area = document.getElementById("epub-area");
  area.innerHTML = "";
  if (state.book) {
    try { state.book.destroy(); } catch {}
  }
  state.book = ePub(`/api/works/${w.id}/content/epub`);
  const rtl = w.reading_direction === "rtl" || w.is_manga;
  state.rendition = state.book.renderTo("epub-area", {
    width: "100%",
    height: "100%",
    flow: "paginated",
    allowScriptedContent: false,
  });
  if (rtl) {
    state.rendition.display();
    state.book.ready.then(() => {
      try { state.rendition.themes.default({ body: { direction: "rtl" } }); } catch {}
    });
  }
  const prog = await api(`/api/works/${w.id}/progress?mode=epub`).catch(() => null);
  if (prog?.position) {
    await state.rendition.display(prog.position);
  } else {
    await state.rendition.display();
  }
  state.rendition.on("relocated", (loc) => {
    const percent = loc.start.percentage * 100;
    api(`/api/works/${w.id}/progress`, {
      method: "PUT",
      json: { mode: "epub", position: loc.start.cfi, percent },
    }).catch(() => {});
  });
}

document.getElementById("reader-close").addEventListener("click", () => {
  if (state.book) {
    try { state.book.destroy(); } catch {}
    state.book = null;
    state.rendition = null;
  }
  show("detail");
  if (state.readerWorkId) openDetail(state.readerWorkId);
});

document.getElementById("reader-prev").addEventListener("click", () => {
  const w = state.currentWork?.work;
  const rtl = w && (w.reading_direction === "rtl" || w.is_manga);
  if (!state.rendition) return;
  if (rtl) state.rendition.next();
  else state.rendition.prev();
});

document.getElementById("reader-next").addEventListener("click", () => {
  const w = state.currentWork?.work;
  const rtl = w && (w.reading_direction === "rtl" || w.is_manga);
  if (!state.rendition) return;
  if (rtl) state.rendition.prev();
  else state.rendition.next();
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
  } catch (e) {
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
  const users = await api("/api/users");
  const ul = document.getElementById("user-list");
  ul.innerHTML = users.map((u) => `<li>${escapeHtml(u.username)} (${u.id})${u.is_admin ? " [admin]" : ""}</li>`).join("");
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
