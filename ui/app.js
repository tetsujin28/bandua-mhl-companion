// Bandua MHL Companion : interface.
"use strict";

const TAURI = window.__TAURI__;
const DEMO = !TAURI;
const invoke = DEMO ? null : TAURI.core.invoke;
const $ = (id) => document.getElementById(id);
const esc = (s) => String(s ?? "").replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));
const VERSION = "0.1.0";
const SITE = "https://tetsujin28.github.io";

function store(k, v) {
  try {
    if (v === undefined) return localStorage.getItem(k);
    localStorage.setItem(k, v);
  } catch (_) { return null; }
}

// ------------------------------------------------------------------ langue
let lang = store("bmc_lang") || ((navigator.language || "fr").slice(0, 2));
if (!I18N[lang]) lang = "en";
const t = (k, vars) => {
  let s = (I18N[lang] && I18N[lang][k]) ?? I18N.en[k] ?? k;
  if (vars) for (const [a, b] of Object.entries(vars)) s = s.split("{" + a + "}").join(b);
  return s;
};

function applyLang() {
  document.documentElement.lang = lang;
  document.querySelectorAll("[data-i18n]").forEach((el) => (el.textContent = t(el.dataset.i18n)));
  document.querySelectorAll("[data-i18n-title]").forEach((el) => (el.title = t(el.dataset.i18nTitle)));
  document.querySelectorAll("#lang button").forEach((b) => b.classList.toggle("on", b.dataset.l === lang));
  render();
}

// ------------------------------------------------------------------ format
function bytes(b) {
  const u = lang === "fr" ? ["o", "Ko", "Mo", "Go", "To"] : ["B", "KB", "MB", "GB", "TB"];
  let v = b, i = 0;
  while (v >= 1000 && i < u.length - 1) { v /= 1000; i++; }
  const s = i === 0 ? `${b} ${u[0]}` : `${v.toFixed(v >= 100 ? 0 : v >= 10 ? 1 : 2)} ${u[i]}`;
  return lang === "en" ? s : s.replace(".", ",");
}
function clock(s) {
  if (!isFinite(s) || s < 0) return "-";
  s = Math.round(s);
  const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60), x = s % 60;
  return h ? `${h}:${String(m).padStart(2, "0")}:${String(x).padStart(2, "0")}` : `${m}:${String(x).padStart(2, "0")}`;
}
const fileName = (p) => String(p).split("/").pop();

// ------------------------------------------------------------------ état
const S = {
  phase: "empty", // empty | scanning | ready | running | done
  zoneMsg: null, // {err, sub}
  source: "",
  packages: [],
  pk: {}, // id → {state, base, done, problems}
  issues: [], // {pkg, issue}
  results: {}, // id → PackageResult
  report: null,
  tab: "problems",
  done: 0, total: 0, filesDone: 0, totalFiles: 0, current: "",
  t0: 0, samples: [], speed: 0,
  cancelArm: false, cancelling: false,
};

function totals() {
  return {
    files: S.packages.reduce((a, p) => a + p.total_files, 0),
    bytes: S.packages.reduce((a, p) => a + p.total_bytes, 0),
  };
}

// ------------------------------------------------------------------ rendu
function render() {
  $("empty").classList.toggle("hide", !(S.phase === "empty" || S.phase === "scanning"));
  $("work").classList.toggle("hide", S.phase === "empty" || S.phase === "scanning");
  $("demoTag").classList.toggle("hide", !DEMO);
  renderChip();
  renderZone();
  if (S.phase !== "empty" && S.phase !== "scanning") {
    renderSource();
    renderPackages();
    renderDash();
    renderVerdict();
    renderTabs();
  }
  renderFooter();
}

function renderChip() {
  const c = $("chip");
  let cls = "", txt = t("chip_ready");
  if (S.phase === "running") { cls = "run"; txt = t("chip_run"); }
  if (S.phase === "done" && S.report) {
    if (S.report.cancelled) { cls = "err"; txt = t("chip_cancel"); }
    else if (S.report.ok) { cls = "ok"; txt = t("chip_ok"); }
    else { cls = "err"; txt = t("chip_bad"); }
  }
  c.className = "chip " + cls;
  c.textContent = txt;
}

function renderZone() {
  const z = $("zoneState");
  if (S.phase === "scanning") z.innerHTML = `<div class="spin"></div><div style="color:var(--muted);margin-top:8px">${esc(t("scanning"))}</div>`;
  else if (S.zoneMsg) z.innerHTML = `<div class="err">${esc(S.zoneMsg.err)}${S.zoneMsg.sub ? `<small>${esc(S.zoneMsg.sub)}</small>` : ""}</div>`;
  else z.innerHTML = "";
}

function renderSource() {
  const tt = totals();
  $("srcPath").textContent = S.source;
  $("sFiles").textContent = tt.files.toLocaleString(lang);
  $("sSize").textContent = bytes(tt.bytes);
  $("pkCount").textContent = S.packages.length;
  $("changeBtn").disabled = S.phase === "running";
  $("changeBtn").style.visibility = S.phase === "running" ? "hidden" : "";
}

function pkPill(id) {
  const st = S.pk[id] || { state: "wait" };
  switch (st.state) {
    case "run": return `<span class="pill run">${esc(t("pk_run"))}</span>`;
    case "ok": return `<span class="pill ok">${esc(t("pk_ok"))}</span>`;
    case "bad": return `<span class="pill corrupt">${esc(t("pk_bad", { n: st.problems }))}</span>`;
    case "cancel": return `<span class="pill no_hash">${esc(t("pk_cancel"))}</span>`;
    default: return S.phase === "ready" ? "" : `<span class="pill wait">${esc(t("pk_wait"))}</span>`;
  }
}

function renderPackages() {
  $("pkgs").innerHTML = S.packages.map((p) => {
    const st = S.pk[p.id] || { state: "wait" };
    const pct = st.state === "run" && p.total_bytes ? Math.min(100, ((st.done - st.base) / p.total_bytes) * 100) : st.state === "wait" ? 0 : 100;
    const kind = p.kind === "ascmhl" ? "ASC MHL" : "MHL 1.1";
    return `<div class="pk ${st.state === "bad" ? "bad" : st.state}" id="pk${p.id}">
      <div class="top"><span class="kind ${p.kind}">${kind}</span><span class="nm" title="${esc(p.label)}">${esc(p.label)}</span>${pkPill(p.id)}</div>
      ${p.location && p.location !== p.label ? `<div class="loc" title="${esc(p.location)}">&lrm;${esc(p.location)}</div>` : ""}
      <div class="meta"><span><b>${p.total_files.toLocaleString(lang)}</b> ${esc(t("files_word"))}</span><span><b>${bytes(p.total_bytes)}</b></span>
        <span>${esc(p.algos.join(", "))}</span>${p.generations > 1 ? `<span>${esc(t("gens", { n: p.generations }))}</span>` : ""}
        ${p.tool ? `<span>${esc(t("by", { t: p.tool }))}</span>` : ""}</div>
      ${p.chain_broken ? `<div class="warn">⚠ ${esc(t("chain_warn"))}</div>` : ""}
      ${st.state === "run" ? `<div class="pbar" style="width:${pct}%"></div>` : ""}
    </div>`;
  }).join("");
}

function renderDash() {
  const pct = S.total ? Math.min(100, (S.done / S.total) * 100) : S.phase === "done" ? 100 : 0;
  const shown = S.phase === "done" && S.report && !S.report.cancelled ? 100 : pct;
  $("pct").textContent = Math.floor(shown);
  $("ringArc").style.strokeDashoffset = 376.99 * (1 - shown / 100);
  let color = "var(--accent)", ph = t("ph_ready");
  if (S.phase === "running") ph = t("ph_run");
  if (S.phase === "done" && S.report) {
    ph = S.report.cancelled ? t("ph_cancel") : t("ph_done");
    color = S.report.cancelled ? "var(--warn)" : S.report.ok ? "var(--ok)" : "var(--err)";
  }
  $("ringArc").style.stroke = color;
  $("bar").style.width = shown + "%";
  $("bar").style.background = color;
  $("phase").textContent = ph;
  const unit = lang === "fr" ? "Mo/s" : "MB/s";
  $("speed").textContent = S.phase === "running" && S.speed > 0 ? `${(S.speed / 1e6).toFixed(0)} ${unit}` : S.phase === "done" && S.report ? `${(S.report.total_bytes / S.report.duration_s / 1e6).toFixed(0)} ${unit}` : "-";
  $("eta").textContent = S.phase === "running" && S.speed > 0 ? clock((S.total - S.done) / S.speed) : S.phase === "done" ? clock(0) : "-";
  $("elapsed").textContent = S.phase === "running" ? clock((Date.now() - S.t0) / 1000) : S.phase === "done" && S.report ? clock(S.report.duration_s) : "-";
  const tf = S.totalFiles || totals().files;
  $("filesDone").textContent = S.phase === "ready" ? `0/${tf}` : `${S.filesDone}/${tf}`;
  $("cur").innerHTML = S.phase === "running" && S.current ? `<b>${esc(fileName(S.current))}</b>  ${esc(S.current)}` : esc(S.phase === "done" && S.report && !S.report.cancelled ? t("cur_done") : t("cur_idle"));
}

function renderVerdict() {
  const v = $("verdict");
  if (S.phase !== "done" || !S.report) { v.innerHTML = ""; v.className = ""; return; }
  const r = S.report;
  if (r.cancelled) { v.className = "verdict can"; v.innerHTML = `<div class="big">${esc(t("v_cancel"))}</div><span>${esc(t("v_cancel_sub"))}</span>`; }
  else if (r.ok) { v.className = "verdict ok"; v.innerHTML = `<div class="big">${esc(t("v_ok"))}</div><span>${esc(t("v_ok_sub"))}</span>`; }
  else { v.className = "verdict err"; v.innerHTML = `<div class="big">${esc(t("v_bad", { n: r.problems }))}</div><span>${esc(t("v_bad_sub"))}</span>`; }
}

function problemIssues() {
  // problèmes d'abord, puis fichiers sans hash
  const rank = { missing: 0, corrupt: 1, unreadable: 2, size: 3, no_hash: 9 };
  return [...S.issues].sort((a, b) => (rank[a.issue.status] ?? 5) - (rank[b.issue.status] ?? 5));
}

function renderTabs() {
  const real = S.issues.filter((x) => x.issue.status !== "no_hash").length;
  const nh = S.issues.length - real;
  $("nProblems").textContent = real + (nh ? ` + ${nh}` : "");
  $("nProblems").className = "n" + (real ? " err" : nh ? " warn" : "");
  const clips = Object.values(S.results).reduce((a, r) => a + r.clips.length, 0);
  $("nClips").textContent = S.phase === "done" ? clips : "-";
  const extra = Object.values(S.results).reduce((a, r) => a + r.extra_total, 0);
  $("nExtra").textContent = S.phase === "done" ? extra : "-";
  $("nExtra").className = "n" + (extra ? " warn" : "");
  document.querySelectorAll("#tabs button").forEach((b) => b.classList.toggle("on", b.dataset.tab === S.tab));
  renderPane();
}

function why(i) {
  switch (i.status) {
    case "corrupt": return t("d_corrupt");
    case "missing": return t("d_missing");
    case "size": return t("d_size", { a: bytes(i.actual_size ?? 0), e: bytes(i.expected_size ?? 0) });
    case "unreadable": return t("d_unreadable", { m: i.detail || "?" });
    case "no_hash": return t("d_no_hash");
  }
  return "";
}

function renderPane() {
  const p = $("pane");
  const multi = S.packages.length > 1;
  const label = (id) => (S.packages.find((x) => x.id === id) || {}).label || "";
  if (S.tab === "problems") {
    if (!S.issues.length) {
      const k = S.phase === "done" ? (S.report && S.report.cancelled ? "none_yet" : "none_done") : S.phase === "running" ? "none_yet" : "none_ready";
      p.innerHTML = `<div class="placeholder ${k === "none_done" ? "good" : ""}">${k === "none_done" ? "✓ " : ""}${esc(t(k))}</div>`;
      return;
    }
    p.innerHTML = problemIssues().map(({ pkg, issue: i }) => `
      <div class="issue">
        <span class="pill ${i.status}">${esc(t("s_" + i.status))}</span>
        <div>
          <div class="nm">${esc(i.clip || fileName(i.rel))}${multi ? `<small>${esc(label(pkg))}</small>` : ""}</div>
          <div class="rel">${esc(i.rel)}</div>
          <div class="why">${esc(why(i))}</div>
          ${i.status === "corrupt" && i.expected_hash ? `<div class="hash">${esc(i.algo)} · ${esc(i.expected_hash)} → ${esc(i.actual_hash || "")}</div>` : ""}
        </div>
        <button class="sbtn" data-reveal="${esc(i.path)}" title="${esc(t("reveal_t"))}">${esc(t("reveal"))}</button>
      </div>`).join("");
    return;
  }
  if (S.phase !== "done") { p.innerHTML = `<div class="placeholder">${esc(t("clips_wait"))}</div>`; return; }
  const res = S.packages.map((x) => S.results[x.id]).filter(Boolean);
  if (S.tab === "clips") {
    const rows = [];
    for (const r of res) for (const c of r.clips) rows.push({ r, c });
    if (!rows.length) { p.innerHTML = `<div class="placeholder">${esc(t("no_clips"))}</div>`; return; }
    rows.sort((a, b) => (b.c.status !== "ok") - (a.c.status !== "ok"));
    const brand = (c) => (c.brand_label === "Générique" ? { fr: "Générique", en: "Generic", es: "Genérico" }[lang] : c.brand_label);
    p.innerHTML = `<table><thead><tr><th>${esc(t("th_clip"))}</th>${multi ? `<th>${esc(t("th_card"))}</th>` : ""}<th></th><th>${esc(t("th_files"))}</th><th>${esc(t("th_size"))}</th><th>${esc(t("th_status"))}</th></tr></thead><tbody>
      ${rows.map(({ r, c }) => `<tr class="${c.status !== "ok" && c.status !== "no_hash" ? "bad" : ""}">
        <td class="name">${esc(c.name)}<small>${esc(c.rel_path)}</small></td>
        ${multi ? `<td class="mono">${esc(r.label)}</td>` : ""}
        <td><span class="brand b-${esc(c.brand_label)}">${esc(brand(c))}</span></td>
        <td class="mono">${c.bad ? `<span style="color:var(--err)">${c.bad}</span>/` : ""}${c.files}</td>
        <td class="mono">${bytes(c.bytes)}</td>
        <td><span class="pill ${c.status}">${esc(t("s_" + c.status))}</span></td></tr>`).join("")}
      </tbody></table>`;
    return;
  }
  // hors MHL
  const total = res.reduce((a, r) => a + r.extra_total, 0);
  if (!total) { p.innerHTML = `<div class="placeholder">${esc(t("extra_none"))}</div>`; return; }
  let h = `<div class="info">${esc(t("extra_info"))}</div>`;
  for (const r of res) {
    for (const e of r.extra) h += `<div class="extra">${multi ? esc(r.label) + " · " : ""}${esc(e)}</div>`;
    if (r.extra_total > r.extra.length) h += `<div class="extra">${esc(t("extra_more", { n: r.extra_total - r.extra.length }))}</div>`;
  }
  p.innerHTML = h;
}

function renderFooter() {
  const go = $("go"), msg = $("footMsg");
  const tt = totals();
  $("exportBtn").classList.toggle("hide", !(S.phase === "done" && S.report));
  $("newBtn").classList.toggle("hide", S.phase !== "done");
  go.classList.remove("stop");
  go.disabled = false;
  switch (S.phase) {
    case "empty":
    case "scanning":
      go.classList.add("hide");
      msg.textContent = "";
      return;
    case "ready":
      go.classList.remove("hide");
      go.textContent = t("start");
      msg.textContent = t("f_ready", { n: tt.files.toLocaleString(lang), size: bytes(tt.bytes) });
      return;
    case "running":
      go.classList.remove("hide");
      go.classList.add("stop");
      go.textContent = S.cancelling ? t("cancelling") : S.cancelArm ? t("confirm_cancel") : t("cancel");
      go.disabled = S.cancelling;
      msg.textContent = t("f_run");
      return;
    case "done": {
      go.classList.remove("hide");
      go.textContent = t("recheck");
      const r = S.report;
      if (!r) return;
      msg.textContent = r.cancelled ? t("f_cancel") : r.ok ? t("f_ok", { n: r.ok_files.toLocaleString(lang), t: clock(r.duration_s) }) : t("f_bad", { n: r.problems });
    }
  }
}

function toast(txt) {
  const el = $("toast");
  el.innerHTML = `<div class="toast">${esc(txt)}</div>`;
  clearTimeout(toast.h);
  toast.h = setTimeout(() => (el.innerHTML = ""), 4200);
}

// ------------------------------------------------------------------ actions
async function load(path) {
  if (S.phase === "running") return;
  S.phase = "scanning";
  S.zoneMsg = null;
  render();
  try {
    const pk = DEMO ? await demoScan(path) : await invoke("scan", { path });
    if (!pk.length) {
      S.phase = "empty";
      S.zoneMsg = { err: t("no_mhl"), sub: t("no_mhl_sub") };
    } else {
      S.source = path;
      S.packages = pk;
      resetRun();
      S.phase = "ready";
    }
  } catch (e) {
    S.phase = "empty";
    const m = String(e);
    S.zoneMsg = m === "not_mhl" ? { err: t("not_mhl") } : m === "not_found" ? { err: t("not_found", { p: path }) } : m === "busy" ? { err: t("busy") } : { err: m };
  }
  render();
}

function resetRun() {
  Object.assign(S, { pk: {}, issues: [], results: {}, report: null, done: 0, total: 0, filesDone: 0, totalFiles: 0, current: "", samples: [], speed: 0, cancelArm: false, cancelling: false });
  S.tab = "problems";
}

async function start() {
  resetRun();
  S.phase = "running";
  S.t0 = Date.now();
  const tt = totals();
  S.total = tt.bytes;
  S.totalFiles = tt.files;
  render();
  try {
    if (DEMO) demoRun();
    else await invoke("start_check", { ids: S.packages.map((p) => p.id) });
  } catch (e) {
    S.phase = "ready";
    toast(String(e) === "busy" ? t("busy") : String(e));
    render();
  }
}

function onEvent(e) {
  switch (e.type) {
    case "started":
      S.total = e.total_bytes;
      S.totalFiles = e.total_files;
      break;
    case "package_started":
      S.pk[e.pkg] = { state: "run", base: S.done, done: S.done, problems: 0 };
      break;
    case "progress": {
      S.done = e.done;
      S.filesDone = e.files_done;
      S.current = e.current;
      if (S.pk[e.pkg]) S.pk[e.pkg].done = e.done;
      const now = Date.now();
      S.samples.push([now, e.done]);
      while (S.samples.length > 2 && now - S.samples[0][0] > 4000) S.samples.shift();
      const [t0, d0] = S.samples[0];
      if (now - t0 > 400) S.speed = DEMO ? 1.12e9 : (e.done - d0) / ((now - t0) / 1000);
      renderLive();
      return;
    }
    case "issue":
      S.issues.push({ pkg: e.pkg, issue: e.issue });
      if (S.pk[e.pkg]) S.pk[e.pkg].problems++;
      break;
    case "package_done": {
      const r = e.result;
      S.results[r.id] = r;
      // les fichiers sans hash ne sont connus qu'à la fin du paquet
      for (const i of r.issues) if (i.status === "no_hash") S.issues.push({ pkg: r.id, issue: i });
      const bad = r.issues.filter((i) => i.status !== "no_hash").length;
      S.pk[r.id] = { ...(S.pk[r.id] || {}), state: r.cancelled ? "cancel" : bad ? "bad" : "ok", problems: bad };
      break;
    }
    case "finished":
      S.report = e.report;
      S.phase = "done";
      S.cancelArm = S.cancelling = false;
      break;
  }
  render();
}

// rendu léger pendant la lecture (évite de reconstruire les listes)
let liveQueued = false;
function renderLive() {
  if (liveQueued) return;
  liveQueued = true;
  requestAnimationFrame(() => {
    liveQueued = false;
    renderDash();
    for (const p of S.packages) {
      const st = S.pk[p.id];
      const el = document.querySelector(`#pk${p.id} .pbar`);
      if (st && st.state === "run" && el) el.style.width = Math.min(100, ((st.done - st.base) / Math.max(1, p.total_bytes)) * 100) + "%";
    }
  });
}

async function cancel() {
  if (S.cancelling) return;
  if (!S.cancelArm) {
    S.cancelArm = true;
    renderFooter();
    setTimeout(() => { S.cancelArm = false; if (S.phase === "running") renderFooter(); }, 3000);
    return;
  }
  S.cancelling = true;
  renderFooter();
  if (DEMO) demoCancel = true;
  else await invoke("cancel_check");
}

async function pickFolder() {
  const p = DEMO ? "/Volumes/RUSHES/FILM_J03" : await invoke("pick_folder");
  if (p) load(p);
}
async function pickMhl() {
  const p = DEMO ? "/Volumes/RUSHES/FILM_J03/A003_CARD.mhl" : await invoke("pick_mhl");
  if (p) load(p);
}

async function exportReport() {
  if (DEMO) { toast(t("exported", { p: "Controle_integrite_FILM_J03.txt" })); return; }
  try {
    const p = await invoke("export_report", { lang });
    if (p) toast(t("exported", { p }));
  } catch (e) { toast(String(e)); }
}

function about(first) {
  const m = $("modal");
  m.innerHTML = `<div class="modal"><div class="box">
    <h2><svg width="34" height="34" viewBox="0 0 64 64"><circle cx="32" cy="32" r="30" fill="url(#lg)"/><circle cx="32" cy="32" r="17" fill="none" stroke="#111" stroke-width="4.5" stroke-dasharray="7 4.4"/><path d="M24.5 32.5l5.5 5.5 9.5-11" fill="none" stroke="#111" stroke-width="5" stroke-linecap="round" stroke-linejoin="round"/></svg>
      Bandua MHL Companion <span class="vtag">v${VERSION}</span></h2>
    <div class="by">by Domingo Olmo Martin</div>
    <p style="margin-top:14px">${esc(t("about_desc"))}</p>
    <p>${esc(t("about_family"))}</p>
    <h4>${esc(t("formats_title"))}</h4><p>${esc(t("formats_text"))}</p>
    <h4>${esc(t("disclaimer_title"))}</h4><p class="warnb">${esc(t("disclaimer_text"))}<br><br><b>${esc(t("disclaimer_rule"))}</b></p>
    <h4>${esc(t("license_title"))}</h4><p>${esc(t("license_text"))}</p>
    <p style="color:var(--dim);font-size:11.5px">© 2026 Domingo Olmo Martin. MIT License.</p>
    <div class="acts"><a id="siteLink">${esc(t("site"))}</a><button class="go" id="closeAbout">${esc(first ? t("understood") : t("close"))}</button></div>
  </div></div>`;
  $("closeAbout").onclick = () => { m.innerHTML = ""; store("bmc_ack", "1"); };
  $("siteLink").onclick = () => (DEMO ? window.open(SITE) : invoke("open_url", { url: SITE }));
}

// ------------------------------------------------------------------ liaisons
$("lang").addEventListener("click", (e) => {
  const l = e.target.dataset && e.target.dataset.l;
  if (l) { lang = l; store("bmc_lang", l); applyLang(); }
});
$("aboutBtn").onclick = () => about(false);
$("pickFolder").onclick = pickFolder;
$("pickMhl").onclick = pickMhl;
$("changeBtn").onclick = () => { if (S.phase !== "running") { S.phase = "empty"; S.zoneMsg = null; render(); } };
$("newBtn").onclick = () => { S.phase = "empty"; S.zoneMsg = null; render(); };
$("exportBtn").onclick = exportReport;
$("go").onclick = () => (S.phase === "running" ? cancel() : start());
$("tabs").addEventListener("click", (e) => {
  const b = e.target.closest("button");
  if (b) { S.tab = b.dataset.tab; renderTabs(); }
});
$("pane").addEventListener("click", (e) => {
  const b = e.target.closest("[data-reveal]");
  if (b && !DEMO) invoke("reveal_path", { path: b.dataset.reveal }).catch((x) => toast(String(x)));
});
setInterval(() => { if (S.phase === "running") renderDash(); }, 1000);

function dragState(over) {
  const working = S.phase !== "empty" && S.phase !== "scanning";
  $("zone").classList.toggle("over", over && !working);
  $("overlay").classList.toggle("hide", !(over && working && S.phase !== "running"));
}

if (!DEMO) {
  TAURI.event.listen("check", (ev) => onEvent(ev.payload));
  TAURI.event.listen("check-fatal", (ev) => { S.phase = "ready"; toast(ev.payload.message); render(); });
  TAURI.webview.getCurrentWebview().onDragDropEvent((ev) => {
    const p = ev.payload;
    if (p.type === "enter" || p.type === "over") dragState(true);
    else if (p.type === "leave") dragState(false);
    else if (p.type === "drop") {
      dragState(false);
      if (p.paths && p.paths.length && S.phase !== "running") load(p.paths[0]);
    }
  });
}

// ------------------------------------------------------------------ démo (aperçu sans Tauri)
let demoCancel = false;
function demoScan(path) {
  return new Promise((res) => setTimeout(() => res([
    { id: 0, kind: "mhl1", label: "A003_CARD", location: "CAM_A/A003", total_files: 46, total_bytes: 412.6e9, algos: ["xxHash64"], tool: "Bandua 0.2.0", generations: 1, chain_broken: false, root: path + "/CAM_A/A003" },
    { id: 1, kind: "ascmhl", label: "B002", location: "CAM_B/B002", total_files: 31, total_bytes: 268.1e9, algos: ["XXH128"], tool: "Silverstack 8", generations: 2, chain_broken: false, root: path + "/CAM_B/B002" },
  ]), 700));
}
function demoRun() {
  demoCancel = false;
  const P = S.packages;
  const files = P.map((p) => p.total_files);
  onEvent({ type: "started", total_files: S.totalFiles, total_bytes: S.total, packages: P.length });
  let pk = 0, done = 0, fd = 0, base = 0;
  onEvent({ type: "package_started", pkg: 0 });
  const names = ["A003_C001_1106XY.RDC/A003_C001_1106XY_001.R3D", "A003_C004_1106KQ.RDC/A003_C004_1106KQ_002.R3D", "A003_C007_1106ZT.RDC/A003_C007_1106ZT_001.R3D"];
  const iv = setInterval(() => {
    if (demoCancel) { clearInterval(iv); demoFinish(true); return; }
    done += S.total / 130;
    const p = P[pk];
    const local = done - base;
    fd = Math.min(S.totalFiles, Math.round(files.slice(0, pk).reduce((a, b) => a + b, 0) + (local / p.total_bytes) * p.total_files));
    onEvent({ type: "progress", done: Math.min(done, S.total), total: S.total, files_done: fd, total_files: S.totalFiles, current: names[Math.floor(done / 5e10) % 3], pkg: pk });
    if (pk === 0 && local > p.total_bytes * 0.35 && !S.issues.length) {
      onEvent({ type: "issue", pkg: 0, issue: { rel: "A003_1106AB.RDM/A003_C004_1106KQ.RDC/A003_C004_1106KQ_002.R3D", clip: "A003_C004_1106KQ", status: "corrupt", path: "", expected_size: 4294705152, actual_size: 4294705152, algo: "xxHash64", expected_hash: "8f1c2b9e44a07d13", actual_hash: "a6d03e71bb5c92f0" } });
    }
    if (pk === 1 && local > p.total_bytes * 0.6 && S.issues.length === 1) {
      onEvent({ type: "issue", pkg: 1, issue: { rel: "Clips/B002C011_231106_R2CD.mxf", clip: "B002C011_231106_R2CD", status: "missing", path: "" } });
    }
    if (local >= p.total_bytes) {
      demoPkgDone(pk);
      base += p.total_bytes;
      pk++;
      if (pk >= P.length) { clearInterval(iv); demoFinish(false); return; }
      onEvent({ type: "package_started", pkg: pk });
    }
  }, 60);
}
function demoClips(pk) {
  const p = S.packages[pk];
  const out = [];
  for (let i = 1; i <= 9; i++) {
    const nm = pk === 0 ? `A003_C00${i}_1106${"XYZKQTRMB"[i - 1]}A` : `B002C0${String(i + 3).padStart(2, "0")}_231106_R2CD`;
    const bad = (pk === 0 && i === 4) || (pk === 1 && i === 8);
    out.push({ name: pk === 0 && i === 4 ? "A003_C004_1106KQ" : pk === 1 && i === 8 ? "B002C011_231106_R2CD" : nm, rel_path: pk === 0 ? `A003_1106AB.RDM/${nm}.RDC` : `Clips/${nm}.mxf`, brand: pk === 0 ? "Red" : "Arri", brand_label: pk === 0 ? "RED" : "ARRI", files: pk === 0 ? 5 : 1, bytes: p.total_bytes / 9, status: bad ? (pk === 0 ? "corrupt" : "missing") : "ok", bad: bad ? 1 : 0 });
  }
  return out;
}
function demoPkgDone(pk) {
  const p = S.packages[pk];
  const issues = S.issues.filter((x) => x.pkg === pk).map((x) => x.issue);
  onEvent({ type: "package_done", result: { ...p, checked: p.total_files, ok_files: p.total_files - issues.length, no_hash: 0, issues, extra: pk === 1 ? ["Clips/B002C006_231106_R2CD_proxy.mov"] : [], extra_total: pk === 1 ? 1 : 0, clips: demoClips(pk), cancelled: false, uncached: true } });
}
function demoFinish(cancelled) {
  const pr = S.issues.length;
  onEvent({ type: "finished", report: { cancelled, ok: !cancelled && !pr, problems: pr, ok_files: S.filesDone - pr, total_files: S.totalFiles, total_bytes: S.done, duration_s: S.done / 1.12e9, extra_total: 1 } });
}

// ------------------------------------------------------------------ démarrage
applyLang();
if (!store("bmc_ack")) about(true);
if (!DEMO) invoke("initial_path").then((p) => p && load(p)).catch(() => {});
if (DEMO && location.hash === "#run") { store("bmc_ack", "1"); $("modal").innerHTML = ""; pickFolder().then(() => setTimeout(start, 900)); }
