/* Diarch Android reader bridge — render-only, no editing. */
(function () {
  const PALETTES = {
    dark: { ink: "#f3e6d4", pageBg: "#0c1210", link: "#e8b87a" },
    sepia: { ink: "#5b4636", pageBg: "#f4ecd8", link: "#8a5a2b" },
    light: { ink: "#1a1a1a", pageBg: "#fafafa", link: "#8a5a2b" },
    paper: { ink: "#2a241c", pageBg: "#f7f1e5", link: "#7a4e28" },
    night: { ink: "#d7e0f0", pageBg: "#0b1220", link: "#8fb4ff" },
    contrast: { ink: "#ffffff", pageBg: "#000000", link: "#ffe566" },
  };
  const FONTS = {
    serif: 'Georgia, "Iowan Old Style", Palatino, serif',
    literata: 'Literata, "Source Serif 4", Georgia, serif',
    sans: '"Source Sans 3", "Segoe UI", system-ui, sans-serif',
    dyslexia: '"Atkinson Hyperlegible", Verdana, sans-serif',
    mono: 'ui-monospace, "Cascadia Code", monospace',
  };
  const LINE_HEIGHTS = { tight: 1.35, normal: 1.7, loose: 2.0 };
  const LETTER_SPACING = { tight: "-0.01em", normal: "0", wide: "0.04em" };
  const PARA_SPACING = { compact: "0.35em", normal: "0.85em", roomy: "1.35em" };
  const MEASURES = {
    narrow: { maxCh: "52ch", padX: "1.5rem" },
    medium: { maxCh: "68ch", padX: "1.25rem" },
    wide: { maxCh: "88ch", padX: "1rem" },
  };

  const state = {
    book: null,
    rendition: null,
    typo: null,
    mode: "epub",
    workId: null,
    baseUrl: null,
    token: null,
    rtl: false,
    mdScrollTimer: null,
    epubLocationsReady: false,
  };

  function post(type, payload) {
    if (window.DiarchBridge && window.DiarchBridge.postMessage) {
      window.DiarchBridge.postMessage(JSON.stringify({ type, ...payload }));
    }
  }

  function setStatus(text) {
    const el = document.getElementById("status");
    if (!el) return;
    el.textContent = text || "";
    el.style.display = text ? "flex" : "none";
  }

  function applyTypography(t) {
    state.typo = t || state.typo || {};
    const typo = state.typo;
    const pal = PALETTES[typo.palette] || PALETTES.dark;
    const root = document.documentElement;
    root.style.setProperty("--ink", pal.ink);
    root.style.setProperty("--page-bg", pal.pageBg);
    root.style.setProperty("--link", pal.link);
    document.body.style.background = pal.pageBg;
    document.body.style.color = pal.ink;
    document.body.style.fontFamily = FONTS[typo.font] || FONTS.serif;

    const size = Math.min(200, Math.max(80, Number(typo.size) || 100));
    const lh = LINE_HEIGHTS[typo.lineHeight] || LINE_HEIGHTS.normal;
    const track = LETTER_SPACING[typo.letterSpacing] || LETTER_SPACING.normal;
    const para = PARA_SPACING[typo.paragraphSpacing] || PARA_SPACING.normal;
    const measure = MEASURES[typo.measure] || MEASURES.medium;
    const justify = !!typo.justify;
    // Infinite-scroll markdown defaults to justified when typo.justify is set;
    // caller may force justify for scroll mode.
    const align = justify ? "justify" : "start";

    root.style.setProperty("--font-size", size + "%");
    root.style.setProperty("--line-height", String(lh));
    root.style.setProperty("--letter-spacing", track);
    root.style.setProperty("--para-spacing", para);
    root.style.setProperty("--max-ch", measure.maxCh);
    root.style.setProperty("--pad-x", measure.padX);
    root.style.setProperty("--reader-align", align);
    root.style.setProperty("--indent", typo.indent ? "1.25em" : "0");
    root.style.setProperty("--hyphens", typo.hyphenate ? "auto" : "manual");

    if (state.rendition) {
      try {
        state.rendition.themes.default({
          body: {
            color: pal.ink + " !important",
            background: pal.pageBg + " !important",
            "font-family": (FONTS[typo.font] || FONTS.serif) + " !important",
            "font-size": size + "% !important",
            "line-height": lh + " !important",
            "letter-spacing": track + " !important",
            "text-align": align + " !important",
            "hyphens": (typo.hyphenate ? "auto" : "manual") + " !important",
          },
          a: { color: pal.link + " !important" },
          p: {
            "margin-bottom": para + " !important",
            "text-indent": (typo.indent ? "1.25em" : "0") + " !important",
          },
        });
      } catch (e) {}
    }
  }

  async function fetchBinary(path) {
    const res = await fetch(state.baseUrl + path, {
      headers: state.token ? { Authorization: "Bearer " + state.token } : {},
    });
    if (!res.ok) throw new Error("HTTP " + res.status);
    return res.arrayBuffer();
  }

  async function fetchText(path) {
    const res = await fetch(state.baseUrl + path, {
      headers: state.token ? { Authorization: "Bearer " + state.token } : {},
    });
    if (!res.ok) throw new Error("HTTP " + res.status);
    return res.text();
  }

  function destroyEpub() {
    if (state.book) {
      try { state.book.destroy(); } catch (e) {}
    }
    state.book = null;
    state.rendition = null;
    state.epubLocationsReady = false;
    const area = document.getElementById("epub-area");
    if (area) area.innerHTML = "";
  }

  async function loadEpub(progress) {
    destroyEpub();
    const epubArea = document.getElementById("epub-area");
    const mdArea = document.getElementById("md-area");
    epubArea.hidden = false;
    mdArea.hidden = true;
    setStatus("Loading EPUB…");
    state.mode = "epub";

    if (typeof ePub === "undefined") {
      setStatus("epub.js failed to load");
      post("error", { message: "epub.js failed to load" });
      return;
    }

    const buf = await fetchBinary("/api/works/" + state.workId + "/content/epub");
    state.book = ePub(buf);
    const width = Math.max(epubArea.clientWidth || 0, window.innerWidth || 320);
    const height = Math.max(epubArea.clientHeight || 0, window.innerHeight || 480);
    state.rendition = state.book.renderTo(epubArea, {
      width,
      height,
      flow: "paginated",
      allowScriptedContent: false,
    });
    applyTypography(state.typo);
    await state.book.ready;

    const toc = (state.book.navigation && state.book.navigation.toc) || [];
    post("toc", {
      items: toc.map(function (t) {
        return { label: t.label || "", href: t.href || "" };
      }),
    });

    try {
      await state.book.locations.generate(1024);
      state.epubLocationsReady = true;
    } catch (e) {
      state.epubLocationsReady = false;
    }

    if (progress && progress.position) {
      await state.rendition.display(progress.position);
    } else {
      await state.rendition.display();
    }

    state.rendition.on("relocated", function (loc) {
      applyTypography(state.typo);
      let pct = 0;
      try {
        if (state.book.locations && state.epubLocationsReady) {
          const frac = state.book.locations.percentageFromCfi(loc.start.cfi);
          if (Number.isFinite(frac)) pct = Math.min(1, Math.max(0, frac)) * 100;
        } else if (Number.isFinite(loc.start.percentage)) {
          pct = Math.min(1, Math.max(0, loc.start.percentage)) * 100;
        }
      } catch (e) {}
      post("progress", {
        mode: "epub",
        position: loc.start.cfi || "",
        percent: pct,
      });
    });

    setStatus("");
    post("ready", { mode: "epub" });
  }

  function mdPercent(area) {
    if (area.scrollHeight <= area.clientHeight) return 100;
    return (area.scrollTop / (area.scrollHeight - area.clientHeight)) * 100;
  }

  async function loadMarkdown(progress, forceJustify) {
    destroyEpub();
    const epubArea = document.getElementById("epub-area");
    const mdArea = document.getElementById("md-area");
    epubArea.hidden = true;
    mdArea.hidden = false;
    setStatus("Loading markdown…");
    state.mode = "markdown";

    const raw = await fetchText("/api/works/" + state.workId + "/content/markdown");
    const typo = Object.assign({}, state.typo || {});
    if (forceJustify) typo.justify = true;
    applyTypography(typo);

    let html = raw;
    if (typeof marked !== "undefined") {
      html = marked.parse(raw);
    }
    if (typeof DOMPurify !== "undefined") {
      html = DOMPurify.sanitize(html);
    }
    mdArea.innerHTML = '<div class="md-inner">' + html + "</div>";

    // Heading TOC from markdown
    const headings = [];
    mdArea.querySelectorAll("h1,h2,h3").forEach(function (h, i) {
      const id = "md-h-" + i;
      h.id = id;
      headings.push({ label: h.textContent || "", href: "#" + id });
    });
    post("toc", { items: headings });

    requestAnimationFrame(function () {
      if (progress && Number.isFinite(progress.percent) && progress.percent > 0) {
        mdArea.scrollTop =
          (progress.percent / 100) * (mdArea.scrollHeight - mdArea.clientHeight);
      } else if (progress && progress.position) {
        const top = Number(progress.position);
        if (Number.isFinite(top)) mdArea.scrollTop = top;
      }
      post("progress", {
        mode: "markdown",
        position: String(mdArea.scrollTop),
        percent: mdPercent(mdArea),
      });
      setStatus("");
      post("ready", { mode: "markdown" });
    });

    mdArea.onscroll = function () {
      clearTimeout(state.mdScrollTimer);
      state.mdScrollTimer = setTimeout(function () {
        post("progress", {
          mode: "markdown",
          position: String(mdArea.scrollTop),
          percent: mdPercent(mdArea),
        });
      }, 250);
    };
  }

  window.DiarchReader = {
    init: function (cfg) {
      state.workId = cfg.workId;
      state.baseUrl = (cfg.baseUrl || "").replace(/\/$/, "");
      state.token = cfg.token || "";
      state.rtl = !!cfg.rtl;
      state.typo = cfg.typography || {};
      applyTypography(state.typo);
    },
    open: async function (opts) {
      try {
        const mode = opts.mode || "epub";
        const progress = opts.progress || null;
        if (mode === "markdown") {
          await loadMarkdown(progress, opts.forceJustify !== false);
        } else {
          await loadEpub(progress);
        }
      } catch (e) {
        setStatus(e.message || String(e));
        post("error", { message: e.message || String(e) });
      }
    },
    setTypography: function (t) {
      applyTypography(t);
    },
    next: function () {
      if (!state.rendition || state.mode !== "epub") return;
      if (state.rtl) state.rendition.prev();
      else state.rendition.next();
    },
    prev: function () {
      if (!state.rendition || state.mode !== "epub") return;
      if (state.rtl) state.rendition.next();
      else state.rendition.prev();
    },
    goToc: function (href) {
      if (!href) return;
      if (state.mode === "markdown") {
        if (href.charAt(0) === "#") {
          const el = document.querySelector(href);
          if (el) el.scrollIntoView({ behavior: "smooth", block: "start" });
        }
        return;
      }
      if (state.rendition) state.rendition.display(href);
    },
    resize: function () {
      if (!state.rendition || state.mode !== "epub") return;
      const area = document.getElementById("epub-area");
      const width = Math.max(area.clientWidth || 0, window.innerWidth || 320);
      const height = Math.max(area.clientHeight || 0, window.innerHeight || 480);
      try {
        state.rendition.resize(width, height);
      } catch (e) {}
    },
  };

  setStatus("Ready");
  post("bridgeReady", {});
})();
