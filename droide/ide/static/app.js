"use strict";

const S = {
  catalog: null,
  examples: [],
  root: "",
  project: null,
  files: [],
  current: null,
  text: "",
  form: null,
  mode: "design",
  selected: null,
  tool: null,
  scale: 1,
  drag: null,
  db: null,
  errors: [],
  listing: "",
  apk: null,
  verify: "",
  manifest: "",
  log: "",
  dirty: false,
  formDirty: false,
  explore: null,
  lens: "calls",
  query: "",
  refKind: "all",
  refScope: "here",
  sortKey: "complexity",
  sortDir: -1,
  pinInspector: false,
  timer: 0,
  findOn: false,
  findQuery: "",
  findAt: -1,
  findSpans: [],
};

const LENSES = [
  ["calls", "Call Graph"],
  ["flow", "Control Flow"],
  ["refs", "References"],
  ["arch", "Architecture"],
];

let graphEdges = [];

const bar = document.getElementById("bar");
const palette = document.getElementById("palette");
const stage = document.getElementById("stage");
const caption = document.getElementById("stage-caption");
const stageWrap = document.getElementById("stage-wrap");
const editor = document.getElementById("editor");
const editorCaption = document.getElementById("editor-caption");
const findbar = document.getElementById("findbar");
const findq = document.getElementById("findq");
const findn = document.getElementById("findn");
const gutter = document.getElementById("gutter");
const hl = document.getElementById("hl");
const code = document.getElementById("code");
const structure = document.getElementById("structure");
const inspector = document.getElementById("inspector");
const status = document.getElementById("status");
const modal = document.getElementById("modal");
const side = document.getElementById("side");
const explore = document.getElementById("explore");
const drawer = document.getElementById("drawer");
const sheet = document.getElementById("sheet");

function esc(value) {
  return String(value).replace(/[&<>"']/g, (ch) => ({
    "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;",
  }[ch]));
}

async function api(method, url, body) {
  const opts = { method, headers: {} };
  if (body !== undefined) {
    opts.headers["Content-Type"] = "application/json";
    opts.body = JSON.stringify(body);
  }
  const res = await fetch(url, opts);
  const data = await res.json();
  if (!res.ok) throw new Error(data.error || res.statusText);
  return data;
}

function buffers() {
  const out = {};
  if (!S.current) return out;
  if (S.dirty) out[S.current.source] = S.text;
  if (S.form && S.formDirty && S.current.form) {
    out[S.current.form] = JSON.stringify(S.form, null, 2) + "\n";
  }
  return out;
}

function snap(value) {
  return Math.max(0, Math.round(value / 4) * 4);
}

function localPoint(ev) {
  const form = document.getElementById("form");
  const rect = form.getBoundingClientRect();
  return {
    x: (ev.clientX - rect.left) / S.scale,
    y: (ev.clientY - rect.top) / S.scale,
  };
}

function typingTarget(el) {
  return el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.tagName === "SELECT");
}

function widgetSpec(type) {
  return S.catalog.widgets[type];
}

function nextName(type) {
  const used = new Set((S.form.components || []).map((comp) => comp.name.toLowerCase()));
  let n = 1;
  while (used.has((type + n).toLowerCase())) n += 1;
  return type + n;
}

function findComp(name) {
  return (S.form && S.form.components || []).find((comp) => comp.name === name) || null;
}

function showProject(view, current, source, form) {
  S.root = view.root;
  S.project = view.project;
  S.files = view.files;
  S.examples = view.examples || S.examples;
  document.title = "Droide — " + (view.project.title || view.project.name);
  S.current = current || null;
  S.text = source || "";
  S.form = form || null;
  S.dirty = false;
  S.formDirty = false;
  S.selected = null;
  S.mode = form ? "design" : "code";
  paintAll();
  analyze();
}

async function boot() {
  const baked = window.DROIDE_BOOT;
  if (baked && baked.catalog) {
    S.catalog = baked.catalog;
    S.examples = (baked.view && baked.view.examples) || [];
    S.db = baked.db || null;
    S.errors = baked.errors || [];
    if (S.db) S.explore = pickFocus();
    paintChrome();
    if (baked.view && baked.view.open) showProject(baked.view, baked.current, baked.source, baked.form);
    else openDialog("open");
    applyHash();
  } else {
    S.catalog = await api("GET", "/api/catalog");
    const view = await api("GET", "/api/project");
    S.examples = view.examples || [];
    paintChrome();
    if (view.open) await adopt(view);
    else openDialog("open");
    applyHash();
  }
  window.addEventListener("resize", () => { fit(); drawGraph(); });
  window.addEventListener("load", fit);
  window.addEventListener("keydown", onKey);
  window.addEventListener("pointermove", onPointerMove);
  window.addEventListener("pointerup", onPointerUp);
  code.addEventListener("input", onCode);
  code.addEventListener("scroll", syncEditorScroll);
  code.addEventListener("keyup", (ev) => {
    paintCaretStatus();
    if (!ev.ctrlKey && !ev.metaKey) code.style.cursor = "";
  });
  code.addEventListener("mousemove", (ev) => {
    code.style.cursor = (ev.ctrlKey || ev.metaKey) ? "pointer" : "";
  });
  code.addEventListener("click", (ev) => {
    paintCaretStatus();
    if (ev.ctrlKey || ev.metaKey) goToDeclaration();
  });
  document.addEventListener("selectionchange", () => {
    if (document.activeElement === code) paintCaretStatus();
  });
  findq.addEventListener("input", () => {
    S.findQuery = findq.value;
    S.findAt = -1;
    activateFind(code.selectionStart || 0, true);
  });
  document.getElementById("find-prev").onclick = () => moveFind(-1);
  document.getElementById("find-next").onclick = () => moveFind(1);
  requestAnimationFrame(fit);
}

function applyHash() {
  const hash = (location.hash || "").replace("#", "");
  const atLine = hash.match(/^L(\d+)$/);
  if (hash === "explore" || hash === "flow" || hash === "refs" || hash === "arch") {
    if (hash !== "explore") S.lens = hash;
    setMode("explore");
  } else if (hash === "code" || atLine) {
    setMode("code");
    if (atLine) revealOffset(offsetOfLine(Number(atLine[1])));
  } else if (hash === "find" || hash.startsWith("find=")) {
    openQuick(hash.startsWith("find=") ? decodeURIComponent(hash.slice(5)) : "");
  } else if (hash === "search" || hash.startsWith("search=")) {
    openFind(hash.startsWith("search=") ? decodeURIComponent(hash.slice(7)) : "");
  } else if (hash === "inspect" && S.form && S.form.components && S.form.components[0]) {
    S.mode = "design";
    S.selected = S.form.components[0].name;
    S.pinInspector = true;
    paintAll();
  }
}

function paintChrome() {
  bar.innerHTML = `
    <span class="brand">Droide</span>
    <button type="button" id="b-project">Open ▾</button>
    <span class="spacer"></span>
    <div class="seg">
      <button type="button" id="b-design">Design</button>
      <button type="button" id="b-code">Code</button>
      <button type="button" id="b-explore">Explore</button>
    </div>
    <button type="button" id="b-build">Compile</button>
    <button type="button" id="b-run">Run</button>`;
  document.getElementById("b-project").onclick = toggleDrawer;
  document.getElementById("b-design").onclick = () => setMode("design");
  document.getElementById("b-code").onclick = () => setMode("code");
  document.getElementById("b-explore").onclick = () => setMode("explore");
  document.getElementById("b-build").onclick = () => compile();
  document.getElementById("b-run").onclick = () => launch();
  paintPalette();
}

function paintProjectButton() {
  const button = document.getElementById("b-project");
  if (button && S.project) button.textContent = (S.project.title || S.project.name) + " ▾";
}

function syncSeg() {
  const design = document.getElementById("b-design");
  const codeBtn = document.getElementById("b-code");
  const exp = document.getElementById("b-explore");
  if (!design) return;
  design.classList.toggle("on", S.mode === "design");
  design.disabled = !(S.current && S.current.form);
  codeBtn.classList.toggle("on", S.mode === "code");
  exp.classList.toggle("on", S.mode === "explore");
}

function paintPalette() {
  if (!S.catalog) return;
  const groups = {};
  Object.entries(S.catalog.widgets).forEach(([name, spec]) => {
    (groups[spec.group] = groups[spec.group] || []).push(name);
  });
  palette.innerHTML = Object.entries(groups).map(([group, names]) => `
    <h3>${esc(group)}</h3>
    ${names.map((name) => `<button type="button" class="tool${S.tool === name ? " on" : ""}" data-tool="${esc(name)}">${esc(name)}</button>`).join("")}
  `).join("");
  palette.querySelectorAll(".tool").forEach((button) => {
    button.onclick = () => {
      S.tool = S.tool === button.dataset.tool ? null : button.dataset.tool;
      if (S.mode !== "design") setMode("design");
      paintPalette();
      fit();
      setStatus(S.tool ? "Click the form to place " + S.tool : "");
    };
  });
}

async function adopt(view) {
  S.root = view.root;
  S.project = view.project;
  S.files = view.files;
  S.examples = view.examples || S.examples;
  document.title = "Droide — " + (view.project.title || view.project.name);
  const main = S.files.find((file) => file.kind === "form" && file.name === view.project.mainForm) || S.files[0];
  if (main) await openFile(main, false);
  else paintAll();
  analyze();
}

async function openFile(info, persist) {
  if (persist !== false && S.current && (S.dirty || S.formDirty)) await save();
  S.current = info;
  S.selected = null;
  S.tool = null;
  const got = await api("GET", "/api/file?path=" + encodeURIComponent(info.source));
  S.text = got.text;
  S.dirty = false;
  S.form = null;
  S.formDirty = false;
  if (info.form) {
    const raw = await api("GET", "/api/file?path=" + encodeURIComponent(info.form));
    S.form = JSON.parse(raw.text || "{}");
    S.mode = "design";
  } else {
    S.mode = "code";
  }
  paintAll();
}

function paintAll() {
  document.body.dataset.mode = S.mode;
  paintProjectButton();
  syncInspector();
  paintCenter();
  paintInspector();
  paintPalette();
  paintStatus();
}

function setMode(mode) {
  if (mode === "design" && !(S.current && S.current.form)) mode = "code";
  if (mode !== "code" && S.findOn) closeFind(false);
  S.mode = mode;
  document.body.dataset.mode = mode;
  paintCenter();
  syncInspector();
  paintStatus();
}

function paintCenter() {
  const design = S.mode === "design" && !!S.form;
  const exploring = S.mode === "explore";
  stageWrap.hidden = !design;
  editor.hidden = design || exploring;
  explore.hidden = !exploring;
  syncSeg();
  if (design) {
    paintForm();
    fit();
    requestAnimationFrame(fit);
  } else if (exploring) {
    paintExplore();
  } else {
    editorCaption.textContent = S.current ? S.current.source : "";
    code.value = S.text;
    hl.innerHTML = highlight(S.text, S.current ? S.current.kind : "unit");
    paintGutter();
    syncEditorScroll();
  }
}

function fit() {
  const form = document.getElementById("form");
  if (!form || stageWrap.hidden) return;
  const width = Math.max(200, stageWrap.clientWidth - 48);
  const height = Math.max(200, stageWrap.clientHeight - 48);
  const w = (S.form && S.form.width) || 360;
  const h = (S.form && S.form.height) || 640;
  S.scale = Math.min(width / w, height / h, 1.35);
  form.style.transform = `scale(${S.scale})`;
  stage.style.width = (w * S.scale) + "px";
  stage.style.height = (h * S.scale) + "px";
  caption.textContent = S.tool ? "Click the form to place " + S.tool : (w + " × " + h + " dp");
}

function paintForm() {
  const bg = colorOf(S.form.background, "#F7F8FA");
  const w = S.form.width || 360;
  const h = S.form.height || 640;
  stage.innerHTML = `<div id="form" style="width:${w}px;height:${h}px;background-color:${esc(bg)}"></div>`;
  const form = document.getElementById("form");
  (S.form.components || []).forEach((comp) => form.appendChild(widgetNode(comp)));
  form.addEventListener("pointerdown", (ev) => {
    if (ev.target !== form) return;
    if (S.tool) {
      const point = localPoint(ev);
      placeWidget(point.x, point.y);
      return;
    }
    S.selected = null;
    paintForm();
    fit();
    paintInspector();
  });
}

function widgetNode(comp) {
  const spec = widgetSpec(comp.type) || { props: [] };
  const el = document.createElement("div");
  el.className = "widget" + (S.selected === comp.name ? " selected" : "");
  el.dataset.name = comp.name;
  el.style.left = comp.left + "px";
  el.style.top = comp.top + "px";
  el.style.width = comp.width + "px";
  el.style.height = comp.height + "px";
  el.innerHTML = widgetFace(comp, spec);
  if (S.selected === comp.name) {
    const grip = document.createElement("i");
    grip.className = "grip";
    grip.addEventListener("pointerdown", (ev) => beginDrag(ev, comp, "resize"));
    el.appendChild(grip);
  }
  el.addEventListener("pointerdown", (ev) => {
    if (ev.target.classList.contains("grip")) return;
    ev.stopPropagation();
    S.selected = comp.name;
    S.tool = null;
    beginDrag(ev, comp, "move");
    paintInspector();
    paintPalette();
  });
  el.addEventListener("dblclick", (ev) => {
    ev.stopPropagation();
    ev.preventDefault();
    ensureHandler(comp);
  });
  return el;
}

function widgetFace(comp, spec) {
  const props = comp.props || {};
  const text = esc(props.Text || "");
  const hint = esc(props.Hint || "");
  const fg = colorOf(props.TextColor, "#1A1A1A");
  const bg = props.Color ? colorOf(props.Color, "") : "";
  const size = Math.max(10, Number(props.TextSize) || 15);
  if (comp.type === "Button") {
    return `<div class="w-button" style="width:100%;height:100%;color:${fg};font-size:${size}px;${bg ? "background:" + bg : ""}">${text}</div>`;
  }
  if (comp.type === "Edit" || comp.type === "Number" || comp.type === "Password" || comp.type === "Email" || comp.type === "Phone") {
    let shown = props.Text ? text : `<span class="hint">${hint}</span>`;
    if (comp.type === "Password" && props.Text) shown = "•".repeat(Math.min(String(props.Text).length, 16));
    return `<div class="w-edit" style="width:100%;height:100%;color:${fg};font-size:${size}px">${shown}</div>`;
  }
  if (comp.type === "Toggle") {
    const on = !!props.Checked;
    const label = esc(on ? (props.TextOn || "On") : (props.TextOff || "Off"));
    return `<div class="w-button w-toggle${on ? " on" : ""}" style="width:100%;height:100%;font-size:${size}px">${label}</div>`;
  }
  if (comp.type === "Memo") {
    const shown = props.Text ? text : `<span class="hint">${hint}</span>`;
    return `<div class="w-memo" style="width:100%;height:100%;color:${fg};font-size:${size}px">${shown}</div>`;
  }
  if (comp.type === "Check" || comp.type === "Radio") {
    const mark = comp.type === "Radio" ? (props.Checked ? "●" : "") : (props.Checked ? "✓" : "");
    const shape = comp.type === "Radio" ? " box round" : " box";
    return `<div class="w-check" style="color:${fg};font-size:${size}px"><span class="${shape.trim()}">${mark}</span>${text}</div>`;
  }
  if (comp.type === "Switch") {
    const on = !!props.Checked;
    return `<div class="w-switch" style="color:${fg};font-size:${size}px"><span class="track" style="${on ? "background:#1d4e89" : ""}"><i style="${on ? "left:18px" : "left:1px"}"></i></span>${text}</div>`;
  }
  if (comp.type === "Seek" || comp.type === "Progress") {
    const max = Math.max(1, Number(props.Max) || 100);
    const value = Math.max(0, Math.min(max, Number(props.Progress) || 0));
    const pct = Math.round((100 * value) / max);
    const face = comp.type === "Seek" ? "w-seek" : "w-progress";
    return `<div class="${face}"><i style="width:${pct}%"></i></div>`;
  }
  if (comp.type === "Rating") {
    const stars = Math.max(1, Number(props.Stars) || 5);
    const rating = Math.max(0, Math.min(stars, Number(props.Rating) || 0));
    return `<div class="w-rating">${"★".repeat(rating)}${"☆".repeat(stars - rating)}</div>`;
  }
  if (comp.type === "Clock") {
    const format = esc(props.Format || "HH:mm");
    return `<div class="w-label" style="width:100%;height:100%;color:${fg};font-size:${size}px;${bg ? "background:" + bg : ""}">${format}</div>`;
  }
  if (comp.type === "Picker") {
    const value = Number(props.Value ?? 0);
    return `<div class="w-picker"><span>▲</span><b>${value}</b><span>▼</span></div>`;
  }
  if (comp.type === "Time") {
    const hour = Math.max(0, Math.min(23, Number(props.Hour) || 0));
    const minute = Math.max(0, Math.min(59, Number(props.Minute) || 0));
    const h24 = props.Is24Hour !== false;
    const pad = (n) => String(n).padStart(2, "0");
    const shown = h24 ? `${pad(hour)}:${pad(minute)}` : `${hour % 12 || 12}:${pad(minute)} ${hour < 12 ? "AM" : "PM"}`;
    return `<div class="w-pick">${shown}</div>`;
  }
  if (comp.type === "Date") {
    const year = Number(props.Year) || 2026;
    const month = Math.max(1, Math.min(12, Number(props.Month) || 1));
    const day = Math.max(1, Math.min(31, Number(props.Day) || 1));
    const pad = (n) => String(n).padStart(2, "0");
    return `<div class="w-pick">${year}-${pad(month)}-${pad(day)}</div>`;
  }
  if (comp.type === "Busy") {
    return `<div class="w-busy"><i></i></div>`;
  }
  if (comp.type === "Panel") {
    return `<div style="width:100%;height:100%;background:${bg || "#E4E8EE"}"></div>`;
  }
  return `<div class="w-label" style="width:100%;height:100%;color:${fg};font-size:${size}px;${bg ? "background:" + bg : ""}">${text}</div>`;
}

function colorOf(value, fallback) {
  const text = String(value || "").trim();
  if (/^#([0-9a-fA-F]{3}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})$/.test(text)) return text;
  return fallback;
}

function placeWidget(x, y) {
  const spec = widgetSpec(S.tool);
  const comp = {
    type: S.tool,
    name: nextName(S.tool),
    left: snap(x),
    top: snap(y),
    width: spec.defaults.width,
    height: spec.defaults.height,
    props: Object.assign({}, spec.defaults.props),
    events: {},
  };
  S.form.components.push(comp);
  S.selected = comp.name;
  S.tool = null;
  S.formDirty = true;
  paintForm();
  fit();
  paintInspector();
  paintPalette();
  paintStatus();
}

function beginDrag(ev, comp, mode) {
  const point = localPoint(ev);
  S.drag = {
    name: comp.name,
    mode,
    dx: point.x - comp.left,
    dy: point.y - comp.top,
    sx: point.x,
    sy: point.y,
    w: comp.width,
    h: comp.height,
    left: comp.left,
    top: comp.top,
  };
  S.selected = comp.name;
  markSelected();
}

function markSelected() {
  stage.querySelectorAll(".widget").forEach((el) => {
    const on = el.dataset.name === S.selected;
    el.classList.toggle("selected", on);
    let grip = el.querySelector(".grip");
    if (on && !grip) {
      grip = document.createElement("i");
      grip.className = "grip";
      grip.addEventListener("pointerdown", (event) => {
        event.stopPropagation();
        const comp = findComp(el.dataset.name);
        if (comp) beginDrag(event, comp, "resize");
      });
      el.appendChild(grip);
    } else if (!on && grip) {
      grip.remove();
    }
  });
}

function onPointerMove(ev) {
  if (!S.drag || !S.form) return;
  const comp = findComp(S.drag.name);
  if (!comp) return;
  const point = localPoint(ev);
  if (S.drag.mode === "move") {
    comp.left = snap(point.x - S.drag.dx);
    comp.top = snap(point.y - S.drag.dy);
  } else {
    comp.width = Math.max(16, snap(S.drag.w + (point.x - S.drag.sx)));
    comp.height = Math.max(16, snap(S.drag.h + (point.y - S.drag.sy)));
  }
  const el = stage.querySelector(`[data-name="${CSS.escape(comp.name)}"]`);
  if (el) {
    el.style.left = comp.left + "px";
    el.style.top = comp.top + "px";
    el.style.width = comp.width + "px";
    el.style.height = comp.height + "px";
  }
  const left = inspector.querySelector("[data-field='left']");
  const top = inspector.querySelector("[data-field='top']");
  const width = inspector.querySelector("[data-field='width']");
  const height = inspector.querySelector("[data-field='height']");
  if (left) left.value = comp.left;
  if (top) top.value = comp.top;
  if (width) width.value = comp.width;
  if (height) height.value = comp.height;
}

function onPointerUp() {
  if (!S.drag) return;
  const comp = findComp(S.drag.name);
  if (comp && (comp.left !== S.drag.left || comp.top !== S.drag.top || comp.width !== S.drag.w || comp.height !== S.drag.h)) {
    S.formDirty = true;
  }
  S.drag = null;
  paintStatus();
}

function ensureHandler(comp) {
  const handler = comp.name + "Click";
  comp.events = comp.events || {};
  comp.events.OnClick = handler;
  S.formDirty = true;
  const pattern = new RegExp("procedure\\s+" + handler + "\\b", "i");
  if (!pattern.test(S.text)) {
    if (S.text && !S.text.endsWith("\n")) S.text += "\n";
    S.text += `\nprocedure ${handler}(Sender: View);\nbegin\n\nend;\n`;
    S.dirty = true;
  }
  S.mode = "code";
  document.body.dataset.mode = "code";
  paintCenter();
  paintInspector();
  const at = S.text.toLowerCase().lastIndexOf("procedure " + handler.toLowerCase());
  if (at >= 0) revealOffset(at);
  schedule();
}

function syncInspector() {
  const show = S.mode === "design" && !!S.form && (S.pinInspector || !!S.selected);
  side.classList.toggle("hidden", !show);
  document.body.classList.toggle("inspect", show);
  if (show) requestAnimationFrame(fit);
}

function paintInspector() {
  structure.innerHTML = "";
  inspector.innerHTML = "";
  const head = document.getElementById("side-head");
  if (!S.form) {
    if (head) head.textContent = "Inspector";
    inspector.innerHTML = "<p class=\"hint\">This file has no form.</p>";
    syncInspector();
    return;
  }
  const items = [`<button type="button" class="struct-item${S.selected ? "" : " on"}" data-name="">${esc(S.form.name)} : Form</button>`];
  (S.form.components || []).forEach((comp) => {
    items.push(`<button type="button" class="struct-item${S.selected === comp.name ? " on" : ""}" data-name="${esc(comp.name)}">${esc(comp.name)} : ${esc(comp.type)}</button>`);
  });
  structure.innerHTML = items.join("");
  structure.querySelectorAll(".struct-item").forEach((button) => {
    button.onclick = () => {
      S.selected = button.dataset.name || null;
      if (S.mode === "design") { paintForm(); fit(); }
      paintInspector();
    };
  });
  const comp = S.selected ? findComp(S.selected) : null;
  if (head) head.textContent = comp ? comp.name + " · " + comp.type : S.form.name;
  if (!comp) paintFormFields();
  else paintCompFields(comp);
  syncInspector();
}

function field(name, value, kind) {
  const type = kind === "number" ? "number" : "text";
  return `<label>${esc(name)}<input data-field="${esc(name)}" type="${type}" value="${esc(value)}"></label>`;
}

function paintFormFields() {
  inspector.innerHTML = field("title", S.form.title || "", "text") +
    field("background", S.form.background || "", "text") +
    field("width", S.form.width || 360, "number") +
    field("height", S.form.height || 640, "number");
  bindFields((name, value) => {
    if (name === "width" || name === "height") S.form[name] = Number(value) || 0;
    else S.form[name] = value;
    S.formDirty = true;
    if (S.mode === "design") { paintForm(); fit(); }
  });
}

function paintCompFields(comp) {
  if (!comp) return;
  const spec = widgetSpec(comp.type);
  const props = spec.props.map((prop) => {
    const kind = (S.catalog.propTypes[prop] || "String") === "Boolean" ? "bool" : "text";
    if (kind === "bool") {
      const present = Object.prototype.hasOwnProperty.call(comp.props, prop);
      const on = present ? !!comp.props[prop] : (prop === "Enabled" || prop === "Visible");
      return `<label>${esc(prop)}<input data-prop="${esc(prop)}" type="checkbox" ${on ? "checked" : ""}></label>`;
    }
    return `<label>${esc(prop)}<input data-prop="${esc(prop)}" type="text" value="${esc(comp.props[prop] ?? "")}"></label>`;
  }).join("");
  const events = (spec.events || []).map((event) =>
    `<label>${esc(event)}<input data-event="${esc(event)}" type="text" value="${esc((comp.events || {})[event] || "")}"></label>`).join("");
  inspector.innerHTML = field("name", comp.name, "text") +
    field("left", comp.left, "number") +
    field("top", comp.top, "number") +
    field("width", comp.width, "number") +
    field("height", comp.height, "number") +
    props + events +
    `<div class="row"><button type="button" id="del-widget">Delete</button></div>`;
  bindFields((name, value) => {
    if (name === "name") return;
    comp[name] = name === "left" || name === "top" || name === "width" || name === "height" ? Number(value) || 0 : value;
    S.formDirty = true;
    if (S.mode === "design") { paintForm(); fit(); }
  });
  inspector.querySelector("[data-field='name']").addEventListener("change", (ev) => {
    const next = ev.target.value.trim();
    if (!next || next === comp.name) return;
    const old = comp.name;
    comp.name = next;
    if (comp.events && comp.events.OnClick === old + "Click") comp.events.OnClick = next + "Click";
    S.selected = next;
    S.formDirty = true;
    paintForm();
    fit();
    paintInspector();
  });
  inspector.querySelectorAll("[data-prop]").forEach((input) => {
    const apply = () => {
      const prop = input.dataset.prop;
      if (input.type === "checkbox") comp.props[prop] = input.checked;
      else if ((S.catalog.propTypes[prop] || "") === "Integer" && prop !== "TextColor" && prop !== "Color") {
        comp.props[prop] = Number(input.value || 0);
      } else comp.props[prop] = input.value;
      S.formDirty = true;
      if (S.mode === "design") { paintForm(); fit(); }
    };
    input.addEventListener("input", apply);
    input.addEventListener("change", apply);
  });
  inspector.querySelectorAll("[data-event]").forEach((input) => {
    input.addEventListener("change", () => {
      comp.events = comp.events || {};
      comp.events[input.dataset.event] = input.value.trim();
      S.formDirty = true;
    });
  });
  inspector.querySelector("#del-widget").onclick = () => {
    S.form.components = S.form.components.filter((item) => item !== comp);
    S.selected = null;
    S.formDirty = true;
    paintForm();
    fit();
    paintInspector();
  };
}

function bindFields(apply) {
  inspector.querySelectorAll("[data-field]").forEach((input) => {
    if (input.dataset.field === "name") return;
    input.addEventListener("input", () => apply(input.dataset.field, input.value));
    input.addEventListener("change", () => apply(input.dataset.field, input.value));
  });
}

function onCode() {
  S.text = code.value;
  S.dirty = true;
  hl.innerHTML = highlight(S.text, S.current ? S.current.kind : "unit");
  if (S.findOn && S.findAt >= 0 && !(S.findSpans || []).some((span) => span[0] === S.findAt)) S.findAt = -1;
  if (S.findOn) writeFindCount();
  paintGutter();
  syncEditorScroll();
  paintStatus();
  schedule();
}

function syncEditorScroll() {
  hl.scrollTop = code.scrollTop;
  hl.scrollLeft = code.scrollLeft;
  gutter.scrollTop = code.scrollTop;
}

function caretPos() {
  const at = code.selectionStart || 0;
  const upto = code.value.slice(0, at);
  const nl = upto.lastIndexOf("\n");
  return { line: upto.split("\n").length, col: at - nl, at };
}

function offsetOfLine(line) {
  const lines = code.value.split("\n");
  let at = 0;
  const n = Math.max(1, Math.min(line || 1, lines.length || 1));
  for (let i = 0; i < n - 1; i += 1) at += lines[i].length + 1;
  return at;
}

function editorLineHeight() {
  const cs = getComputedStyle(code);
  const lh = parseFloat(cs.lineHeight);
  if (Number.isFinite(lh) && lh > 0) return lh;
  const size = parseFloat(cs.fontSize);
  return (Number.isFinite(size) ? size : 13) * 1.55;
}

// Pascal scopes are begin/end. `{...}` and `(*...*)` are comments.
// Native scopes are braces. Strings and comments do not open a scope.
function blockScopes(src, native) {
  const scopes = [];
  const stack = [];
  let i = 0;
  let line = 0;
  const n = src.length;
  while (i < n) {
    const ch = src[i];
    const two = src.slice(i, i + 2);
    if (ch === "\n") { line += 1; i += 1; continue; }
    if (two === "//") {
      while (i < n && src[i] !== "\n") i += 1;
      continue;
    }
    if (!native && ch === "{") {
      i += 1;
      while (i < n && src[i] !== "}") {
        if (src[i] === "\n") line += 1;
        i += 1;
      }
      if (i < n) i += 1;
      continue;
    }
    if (!native && two === "(*") {
      i += 2;
      while (i + 1 < n && src.slice(i, i + 2) !== "*)") {
        if (src[i] === "\n") line += 1;
        i += 1;
      }
      i = i + 1 < n ? i + 2 : n;
      continue;
    }
    if (ch === "'" || ch === '"') {
      const quote = ch;
      i += 1;
      while (i < n && src[i] !== "\n") {
        if (src[i] === quote) {
          if (quote === "'" && src[i + 1] === "'") { i += 2; continue; }
          i += 1;
          break;
        }
        i += 1;
      }
      continue;
    }
    if (native && (ch === "{" || ch === "}")) {
      if (ch === "{") stack.push([line, stack.length]);
      else if (stack.length) {
        const open = stack.pop();
        scopes.push({ open: open[0], close: line, depth: open[1] });
      }
      i += 1;
      continue;
    }
    if (/[A-Za-z_]/.test(ch)) {
      const start = i;
      i += 1;
      while (i < n && /[A-Za-z0-9_]/.test(src[i])) i += 1;
      const word = src.slice(start, i).toLowerCase();
      if (!native && word === "begin") stack.push([line, stack.length]);
      else if (!native && word === "end" && stack.length) {
        const open = stack.pop();
        scopes.push({ open: open[0], close: line, depth: open[1] });
      }
      continue;
    }
    i += 1;
  }
  while (stack.length) {
    const open = stack.pop();
    scopes.push({ open: open[0], close: line, depth: open[1] });
  }
  return scopes;
}

function paintGutter() {
  const src = code.value;
  const lines = src.split("\n");
  const n = Math.max(1, lines.length);
  const here = caretPos().line;
  const native = !!(S.current && S.current.kind === "native");
  const scopes = blockScopes(src, native);
  const caret0 = here - 1;
  let hot = -1;
  scopes.forEach((scope) => {
    if (scope.open <= caret0 && caret0 <= scope.close && scope.depth > hot) hot = scope.depth;
  });
  const lh = editorLineHeight();
  gutter.style.setProperty("--lh", lh + "px");
  const rails = [];
  scopes.forEach((scope) => {
    if (scope.depth >= 6) return;
    const x = 8.5 + scope.depth * 8;
    const yOpen = (scope.open + 0.5) * lh;
    const yClose = (scope.close + 0.5) * lh;
    const color = scope.depth === hot && scope.open <= caret0 && caret0 <= scope.close ? "#1d4e89" : "#c4beb4";
    if (scope.open === scope.close) {
      rails.push(`<line x1="${x}" y1="${yOpen}" x2="${x + 5}" y2="${yOpen}" stroke="${color}" stroke-width="1"/>`);
    } else {
      rails.push(`<line x1="${x}" y1="${yOpen}" x2="${x}" y2="${yClose}" stroke="${color}" stroke-width="1"/>`);
      rails.push(`<line x1="${x}" y1="${yOpen}" x2="${x + 5}" y2="${yOpen}" stroke="${color}" stroke-width="1"/>`);
      rails.push(`<line x1="${x}" y1="${yClose}" x2="${x + 5}" y2="${yClose}" stroke="${color}" stroke-width="1"/>`);
    }
  });
  let nums = "";
  for (let i = 1; i <= n; i += 1) {
    nums += i === here ? `<span class="here">${i}</span>` : `<span>${i}</span>`;
  }
  gutter.innerHTML = `<span class="gutter-in"><svg class="rails" width="56" height="${n * lh}">${rails.join("")}</svg><span class="gnums">${nums}</span></span>`;
  gutter.scrollTop = code.scrollTop;
}

function revealOffset(at) {
  const full = code.value;
  const clamped = Math.max(0, Math.min(at, full.length));
  code.focus();
  code.value = full.slice(0, clamped);
  code.scrollTop = code.scrollHeight;
  const top = code.scrollTop;
  code.value = full;
  code.scrollTop = Math.max(0, top - 48);
  code.setSelectionRange(clamped, clamped);
  paintGutter();
  syncEditorScroll();
  paintCaretStatus();
}

function wordAtCaret() {
  const src = code.value;
  if (!src) return "";
  let a = Math.min(code.selectionStart || 0, src.length);
  const isWord = (ch) => /[A-Za-z0-9_]/.test(ch || "");
  if (a > 0 && !isWord(src[a]) && isWord(src[a - 1])) a -= 1;
  let b = a;
  while (a > 0 && isWord(src[a - 1])) a -= 1;
  while (b < src.length && isWord(src[b])) b += 1;
  return a === b ? "" : src.slice(a, b);
}

function entityAtCaret() {
  if (!S.db || !S.current) return null;
  const file = S.current.source;
  const { line } = caretPos();
  const word = wordAtCaret();
  if (word) {
    const low = word.toLowerCase();
    const named = S.db.entities.filter((ent) => ent.name.toLowerCase() === low || (ent.qname || "").toLowerCase() === low);
    const onLine = named.filter((ent) => (S.db.refs || []).some((ref) =>
      ref.file === file && ref.line === line && (ref.dst === ent.id || ref.src === ent.id)));
    if (onLine.length) return onLine[0];
    const declared = named.find((ent) => ent.file === file && ent.line === line);
    if (declared) return declared;
    const routine = named.find((ent) => ["procedure", "function", "native", "builtin", "widget"].includes(ent.kind));
    if (routine && named.filter((ent) => ent.kind === routine.kind && ent.name.toLowerCase() === low).length === 1) return routine;
  }
  const covers = S.db.entities.filter((ent) =>
    ent.file === file && ent.line <= line && (ent.endLine || ent.line) >= line &&
    ["procedure", "function", "native"].includes(ent.kind));
  covers.sort((a, b) => ((a.endLine || a.line) - a.line) - ((b.endLine || b.line) - b.line));
  return covers[0] || null;
}

function contextEntity() {
  if (S.mode === "code") return entityAtCaret();
  if (S.mode === "design" && S.form && S.selected && S.db) {
    const q = (S.form.name + "." + S.selected).toLowerCase();
    return S.db.entities.find((ent) => ent.kind === "widget" && (ent.qname || "").toLowerCase() === q) || null;
  }
  return null;
}

function declarationFor(word) {
  if (!S.db || !word) return null;
  const low = word.toLowerCase();
  const named = S.db.entities.filter((ent) => ent.name.toLowerCase() === low);
  const order = ["native", "function", "procedure", "form", "unit", "widget", "builtin"];
  for (let i = 0; i < order.length; i += 1) {
    const hits = named.filter((ent) => ent.kind === order[i]);
    if (hits.length === 1) return hits[0];
  }
  const at = entityAtCaret();
  if (at && at.name.toLowerCase() === low) return at;
  return null;
}

async function openEntity(ent) {
  if (!ent) return;
  if (ent.kind === "builtin" || !ent.file) {
    setStatus(ent.name + " is built in");
    return;
  }
  if (ent.kind === "widget" || ent.kind === "form") {
    const info = S.files.find((file) => file.source === ent.file || file.form === ent.file
      || (ent.kind === "form" && file.kind === "form" && file.name.toLowerCase() === ent.name.toLowerCase()));
    if (info) await openFile(info);
    if (ent.kind === "widget" && S.form) {
      S.selected = ent.name;
      S.mode = "design";
      paintAll();
    }
    return;
  }
  await jump(ent.file, ent.line || 1);
}

function goToDeclaration() {
  const word = wordAtCaret();
  const ent = declarationFor(word);
  if (!ent) {
    if (word) setStatus("No declaration for " + word);
    return;
  }
  openEntity(ent);
}

function quickRank(kind) {
  return { native: 0, function: 0, procedure: 0, form: 1, unit: 1, widget: 2, file: 3 }[kind] ?? 4;
}

function quickScore(query, name, file, qname) {
  if (!query) return 1;
  const label = String(name || "").toLowerCase();
  const path = String(file || "").toLowerCase();
  const qualified = String(qname || "").toLowerCase();
  if (label === query || qualified === query) return 100;
  if (label.startsWith(query)) return 80;
  if (query.includes(".") && qualified.includes(query)) return 70;
  if (label.includes(query)) return 55;
  if (path === query || path.endsWith("/" + query)) return 50;
  if (path.includes(query)) return 35;
  return 0;
}

function quickItems(query) {
  const q = query.trim().toLowerCase();
  const items = [];
  (S.files || []).forEach((file) => {
    items.push({
      kind: "file",
      label: file.name,
      detail: file.source,
      file,
      score: quickScore(q, file.name, file.source, file.name),
    });
  });
  (S.db ? S.db.entities : []).forEach((ent) => {
    if (ent.kind === "native-file") return;
    if (!q && (ent.kind === "param" || ent.kind === "local" || ent.kind === "builtin")) return;
    items.push({
      kind: ent.kind,
      label: ent.qname || ent.name,
      detail: ent.file ? ent.file + ":" + (ent.line || 1) : "",
      ent,
      score: quickScore(q, ent.name, "", ent.qname),
    });
  });
  if (!q) {
    const files = items.filter((item) => item.kind === "file");
    const routines = items.filter((item) => ["procedure", "function", "native", "form", "unit"].includes(item.kind));
    routines.sort((a, b) => ((b.ent.fanIn || 0) - (a.ent.fanIn || 0)) || a.label.localeCompare(b.label));
    return files.concat(routines).slice(0, 40);
  }
  return items.filter((item) => item.score > 0)
    .sort((a, b) => b.score - a.score || quickRank(a.kind) - quickRank(b.kind) || a.label.localeCompare(b.label))
    .slice(0, 40);
}

function openQuick(seed) {
  S.quickQuery = seed || "";
  S.quickIndex = 0;
  modal.hidden = false;
  paintQuick();
}

function paintQuick() {
  const items = quickItems(S.quickQuery);
  S.quickItems = items;
  if (S.quickIndex >= items.length) S.quickIndex = Math.max(0, items.length - 1);
  const rows = items.length ? items.map((item, index) => `
    <button type="button" class="quick-item${index === S.quickIndex ? " on" : ""}" data-i="${index}">
      <span class="idx-k">${esc(item.kind === "file" ? "file" : kindLabel(item.kind))}</span>
      <span>${esc(item.label)}</span>
      <span class="hint">${esc(item.detail || "")}</span>
    </button>`).join("") : `<p class="hint">Nothing matches.</p>`;
  modal.innerHTML = `<div class="dialog quick">
    <input id="quick" placeholder="File, routine, or widget" value="${esc(S.quickQuery)}" autocomplete="off" spellcheck="false">
    <div id="quick-list">${rows}</div>
    <p class="hint">Enter opens it. Ctrl+Enter explores it.</p>
  </div>`;
  const input = document.getElementById("quick");
  input.oninput = () => {
    S.quickQuery = input.value;
    S.quickIndex = 0;
    paintQuick();
  };
  modal.onclick = (ev) => { if (ev.target === modal) modal.hidden = true; };
  modal.querySelectorAll(".quick-item").forEach((button) => {
    button.onmousemove = () => { S.quickIndex = Number(button.dataset.i); };
    button.onclick = (ev) => activateQuick(Number(button.dataset.i), ev.ctrlKey || ev.metaKey);
  });
  input.focus();
  input.setSelectionRange(input.value.length, input.value.length);
  const current = modal.querySelector(".quick-item.on");
  if (current) current.scrollIntoView({ block: "nearest" });
}

function moveQuick(ev) {
  const count = (S.quickItems || []).length;
  if (ev.key === "ArrowDown") S.quickIndex = Math.min(count - 1, S.quickIndex + 1);
  else if (ev.key === "ArrowUp") S.quickIndex = Math.max(0, S.quickIndex - 1);
  else if (ev.key === "Enter") {
    activateQuick(S.quickIndex, ev.ctrlKey || ev.metaKey);
    return;
  }
  paintQuick();
}

async function activateQuick(index, explore) {
  const item = (S.quickItems || [])[index];
  if (!item) return;
  modal.hidden = true;
  if (explore && item.ent) {
    S.explore = item.ent.id;
    S.lens = item.ent.kind === "widget" || item.ent.kind === "form" || item.ent.kind === "unit" ? "refs" : "calls";
    setMode("explore");
    return;
  }
  if (item.file && !item.ent) {
    await openFile(item.file);
    return;
  }
  await openEntity(item.ent);
}

function caretPhrase() {
  const ent = contextEntity();
  if (!ent) return "";
  if (["procedure", "function", "native", "builtin"].includes(ent.kind)) {
    return ent.qname + " · " + (ent.fanIn || 0) + " in · " + (ent.fanOut || 0) + " out";
  }
  return ent.qname + (ent.type ? " · " + ent.type : "");
}

function paintCaretStatus() {
  const slot = document.getElementById("st-caret");
  if (slot) slot.textContent = caretPhrase();
  if (S.mode === "code" && gutter) paintGutter();
}

function findSpans(src, query) {
  const needle = String(query || "");
  if (!needle) return [];
  const hay = src.toLowerCase();
  const low = needle.toLowerCase();
  const spans = [];
  let i = 0;
  while (i <= hay.length - low.length) {
    const at = hay.indexOf(low, i);
    if (at < 0) break;
    spans.push([at, at + low.length]);
    i = at + low.length;
  }
  return spans;
}

function closeFind(focus) {
  if (!S.findOn && findbar.hidden) return;
  S.findOn = false;
  S.findAt = -1;
  S.findSpans = [];
  editor.classList.remove("find-on");
  findbar.hidden = true;
  if (!editor.hidden) {
    hl.innerHTML = highlight(code.value, S.current ? S.current.kind : "unit");
    syncEditorScroll();
  }
  if (focus) code.focus();
}

function revealFind() {
  const hit = (S.findSpans || []).find((span) => span[0] === S.findAt);
  if (!hit) return;
  code.setSelectionRange(hit[0], hit[1]);
  const line = code.value.slice(0, hit[0]).split("\n").length;
  code.scrollTop = Math.max(0, (line - 3) * 13 * 1.55);
  syncEditorScroll();
  paintGutter();
  paintCaretStatus();
}

function paintFind(jump) {
  editor.classList.toggle("find-on", !!S.findOn);
  findbar.hidden = !S.findOn;
  if (findq.value !== S.findQuery) findq.value = S.findQuery;
  if (!editor.hidden) {
    hl.innerHTML = highlight(code.value, S.current ? S.current.kind : "unit");
    syncEditorScroll();
  }
  writeFindCount();
  if (jump && S.findOn && S.findAt >= 0) revealFind();
}

function writeFindCount() {
  const spans = S.findSpans || [];
  if (!S.findQuery) findn.textContent = "";
  else if (!spans.length) findn.textContent = "none";
  else if (S.findAt < 0) findn.textContent = String(spans.length);
  else {
    const idx = Math.max(0, spans.findIndex((span) => span[0] === S.findAt));
    findn.textContent = (idx + 1) + " of " + spans.length;
  }
}

function activateFind(origin, jump) {
  const spans = findSpans(code.value, S.findQuery);
  if (!spans.length) S.findAt = -1;
  else if (!spans.some((span) => span[0] === S.findAt)) {
    const from = origin == null ? 0 : origin;
    const next = spans.find((span) => span[0] >= from) || spans[0];
    S.findAt = next[0];
  }
  paintFind(jump);
}

function moveFind(dir) {
  if (!S.findOn) return;
  const spans = findSpans(code.value, S.findQuery);
  if (!spans.length) {
    S.findAt = -1;
    paintFind(false);
    return;
  }
  let idx = spans.findIndex((span) => span[0] === S.findAt);
  if (idx < 0) idx = dir < 0 ? 0 : -1;
  idx = (idx + dir + spans.length) % spans.length;
  S.findAt = spans[idx][0];
  paintFind(true);
}

function openFind(query) {
  if (S.mode !== "code") setMode("code");
  if (query == null) {
    const selected = code.value.slice(code.selectionStart || 0, code.selectionEnd || 0);
    const seed = selected && !selected.includes("\n") ? selected : "";
    if (seed) S.findQuery = seed;
    else if (!S.findOn) {
      const word = wordAtCaret();
      if (word) S.findQuery = word;
    }
  } else {
    S.findQuery = query;
  }
  S.findOn = true;
  S.findAt = -1;
  activateFind(code.selectionStart || 0, true);
  findq.focus();
  findq.select();
}

function highlight(src, kind) {
  const native = kind === "native";
  const words = native
    ? new Set(["fun", "return", "if", "else", "while", "var", "i32"])
    : new Set(["unit", "uses", "procedure", "function", "var", "begin", "end", "if", "then", "else", "while", "do", "exit", "and", "or", "not", "div", "mod", "true", "false", "integer", "string", "boolean", "view"]);
  const spans = (S.findOn && S.findQuery) ? findSpans(src, S.findQuery) : [];
  S.findSpans = spans;
  const piece = (from, to) => {
    if (!spans.length || from >= to) return esc(src.slice(from, to));
    let html = "";
    let i = from;
    for (let k = 0; k < spans.length; k += 1) {
      const a = spans[k][0];
      const b = spans[k][1];
      if (b <= i || a >= to) continue;
      const s = Math.max(a, i);
      const e = Math.min(b, to);
      if (s > i) html += esc(src.slice(i, s));
      html += `<mark class="hit${a === S.findAt ? " on" : ""}">${esc(src.slice(s, e))}</mark>`;
      i = e;
    }
    if (i < to) html += esc(src.slice(i, to));
    return html;
  };
  let i = 0;
  let out = "";
  const len = src.length;
  while (i < len) {
    const ch = src[i];
    const two = src.slice(i, i + 2);
    if (two === "//") {
      let j = i;
      while (j < len && src[j] !== "\n") j += 1;
      out += `<span class="cmt">${piece(i, j)}</span>`;
      i = j;
      continue;
    }
    if (!native && ch === "{") {
      let j = src.indexOf("}", i + 1);
      j = j < 0 ? len : j + 1;
      out += `<span class="cmt">${piece(i, j)}</span>`;
      i = j;
      continue;
    }
    if (!native && two === "(*") {
      let j = src.indexOf("*)", i + 2);
      j = j < 0 ? len : j + 2;
      out += `<span class="cmt">${piece(i, j)}</span>`;
      i = j;
      continue;
    }
    if (ch === "'" || ch === '"') {
      let j = i + 1;
      while (j < len) {
        if (src[j] === ch) {
          if (ch === "'" && src[j + 1] === "'") { j += 2; continue; }
          j += 1;
          break;
        }
        if (src[j] === "\n") break;
        j += 1;
      }
      out += `<span class="str">${piece(i, j)}</span>`;
      i = j;
      continue;
    }
    if (/[A-Za-z_]/.test(ch)) {
      let j = i + 1;
      while (j < len && /[A-Za-z0-9_]/.test(src[j])) j += 1;
      const word = src.slice(i, j);
      if (words.has(word.toLowerCase())) out += `<span class="kw">${piece(i, j)}</span>`;
      else out += piece(i, j);
      i = j;
      continue;
    }
    if (/[0-9$]/.test(ch)) {
      let j = i + 1;
      while (j < len && /[0-9A-Fa-fx]/.test(src[j])) j += 1;
      out += `<span class="num">${piece(i, j)}</span>`;
      i = j;
      continue;
    }
    out += piece(i, i + 1);
    i += 1;
  }
  return out;
}

function schedule() {
  clearTimeout(S.timer);
  S.timer = setTimeout(analyze, 450);
}

async function save() {
  if (!S.current) return;
  if (S.dirty) {
    await api("PUT", "/api/file", { path: S.current.source, text: S.text });
    S.dirty = false;
    if (S.current.source === "project.json") {
      try { S.project = JSON.parse(S.text); } catch (err) { /* the next analyze reports it */ }
    }
  }
  if (S.form && S.formDirty && S.current.form) {
    await api("PUT", "/api/file", { path: S.current.form, text: JSON.stringify(S.form, null, 2) + "\n" });
    S.formDirty = false;
  }
  paintStatus();
  paintProjectButton();
  setStatus("Saved");
}

async function analyze() {
  if (!S.root) return;
  try {
    const result = await api("POST", "/api/analyze", { buffers: buffers() });
    S.errors = result.errors || [];
    if (result.db) S.db = result.db;
    if (S.explore == null || !entity(S.explore)) S.explore = pickFocus();
    if (S.mode === "explore") paintExplore();
    paintStatus();
  } catch (err) {
    setStatus(err.message);
  }
}

async function compile() {
  setStatus("Compiling…");
  try {
    const result = await api("POST", "/api/build", { buffers: buffers() });
    takeBuild(result);
  } catch (err) {
    setStatus(err.message);
  }
}

async function launch() {
  setStatus("Launching…");
  try {
    const result = await api("POST", "/api/run", { buffers: buffers() });
    S.log = result.log || "";
    takeBuild(result);
    if (result.launched) setStatus("Started on " + (result.device || "device"));
    else if (result.ok) {
      setStatus("Not launched");
      openSheet("build");
    }
  } catch (err) {
    setStatus(err.message);
  }
}

function takeBuild(result) {
  S.errors = result.errors || [];
  if (result.db) S.db = result.db;
  S.listing = result.listing || "";
  S.apk = result.apk || null;
  S.verify = result.verify || "";
  S.manifest = result.manifest || "";
  if (S.mode === "explore") paintExplore();
  paintStatus();
  if (!result.ok) openSheet("messages");
  else setStatus(result.apk ? "APK ready" : "Build finished");
}

function messagesHtml() {
  const rows = [];
  if (!S.errors.length) rows.push(`<div class="msg ok">No errors.</div>`);
  S.errors.forEach((err) => {
    rows.push(`<button type="button" class="msg err" data-jump-file="${esc(err.file || "")}" data-jump-line="${err.line || 1}">${esc(err.file || "")}:${err.line || 1}: ${esc(err.message)}</button>`);
  });
  if (S.log) rows.push(`<pre class="dump">${esc(S.log)}</pre>`);
  return `<div class="sheet-bar"><b>Messages</b><button type="button" data-close>Close</button></div>` + rows.join("");
}

function buildHtml() {
  const listing = S.listing ? `<details><summary>Assembly</summary><pre class="dump">${esc(S.listing)}</pre></details>` : "";
  const body = [S.verify || "", S.manifest ? S.manifest.slice(0, 4000) : "", S.log || ""].filter(Boolean).join("\n");
  return `<div class="sheet-bar"><b>Build</b><button type="button" data-close>Close</button></div>
    <div class="msg ${S.apk ? "ok" : ""}">${esc(S.apk || "No APK yet.")}</div>
    <pre class="dump">${esc(body)}</pre>${listing}`;
}

function openSheet(kind) {
  sheet.hidden = false;
  sheet.innerHTML = `<div class="sheet">${kind === "messages" ? messagesHtml() : buildHtml()}</div>`;
  sheet.onclick = (ev) => { if (ev.target === sheet) closeSheet(); };
  const closer = sheet.querySelector("[data-close]");
  if (closer) closer.onclick = closeSheet;
  sheet.querySelectorAll("[data-jump-file]").forEach((button) => {
    button.onclick = () => {
      closeSheet();
      jump(button.dataset.jumpFile, Number(button.dataset.jumpLine || 1));
    };
  });
}

function closeSheet() {
  sheet.hidden = true;
  sheet.innerHTML = "";
}

async function jump(file, line) {
  if (!file) return;
  const info = S.files.find((item) => item.source === file || item.form === file);
  if (info && (!S.current || S.current.source !== info.source)) await openFile(info);
  if (!info || info.source !== file) return;
  setMode("code");
  revealOffset(offsetOfLine(line));
}

function entity(id) {
  return S.db && S.db.entities.find((ent) => ent.id === id);
}

function kindLabel(kind) {
  return {
    "native-file": "native file",
    native: "native",
    procedure: "procedure",
    function: "function",
    form: "form",
    unit: "unit",
    widget: "widget",
    builtin: "builtin",
    param: "parameter",
    local: "local",
  }[kind] || kind;
}

function pickFocus() {
  if (!S.db || !S.db.entities.length) return null;
  const list = S.db.entities.filter((ent) => ["procedure", "function", "native"].includes(ent.kind));
  const pool = (list.length ? list : S.db.entities).slice();
  pool.sort((a, b) => (b.fanIn - a.fanIn) || (b.fanOut - a.fanOut) || (a.id - b.id));
  return pool[0].id;
}

function nameOf(id) {
  const ent = entity(id);
  return ent ? ent.qname : "#" + id;
}

function entButton(ent, depth) {
  const on = ent.id === S.explore ? " on" : "";
  return `<button type="button" class="idx${on}" data-focus="${ent.id}" style="padding-left:${8 + depth * 14}px"><span class="idx-k">${esc(kindLabel(ent.kind))}</span><span>${esc(ent.name)}</span></button>`;
}

function indexList() {
  if (!S.db) return "";
  const q = S.query.trim().toLowerCase();
  if (q) {
    const hits = S.db.entities.filter((ent) =>
      ent.name.toLowerCase().includes(q) ||
      (ent.qname || "").toLowerCase().includes(q) ||
      ent.kind.toLowerCase().includes(q) ||
      (ent.file || "").toLowerCase().includes(q) ||
      (ent.type || "").toLowerCase().includes(q));
    if (!hits.length) return `<p class="hint">Nothing matches.</p>`;
    return hits.slice(0, 200).map((ent) => entButton(ent, 0)).join("");
  }
  const roots = S.db.entities.filter((ent) => ent.kind === "form" || ent.kind === "unit" || ent.kind === "native-file");
  const placed = new Set();
  let html = roots.map((root) => {
    placed.add(root.id);
    const kids = S.db.entities.filter((ent) => ent.parent === root.id && ent.kind !== "param" && ent.kind !== "local");
    kids.forEach((kid) => placed.add(kid.id));
    return entButton(root, 0) + kids.map((kid) => entButton(kid, 1)).join("");
  }).join("");
  const builtins = S.db.entities.filter((ent) => ent.kind === "builtin");
  if (builtins.length) {
    builtins.forEach((ent) => placed.add(ent.id));
    html += `<h3 class="idx-h">Builtins</h3>` + builtins.map((ent) => entButton(ent, 0)).join("");
  }
  const orphans = S.db.entities.filter((ent) => !placed.has(ent.id) && ent.kind !== "param" && ent.kind !== "local");
  if (orphans.length) html += orphans.map((ent) => entButton(ent, 0)).join("");
  return html;
}

function signature(ent) {
  const params = S.db.entities.filter((item) => item.parent === ent.id && item.kind === "param");
  if (["procedure", "function", "native", "builtin"].includes(ent.kind)) {
    const args = params.map((item) => item.name + ": " + (item.type || "")).join(", ");
    return ent.name + "(" + args + ")" + (ent.type ? ": " + ent.type : "");
  }
  if (ent.kind === "widget") return ent.name + ": " + (ent.type || "widget");
  return ent.qname;
}

function headHtml(ent) {
  const members = S.db.entities.filter((item) => item.parent === ent.id && (item.kind === "param" || item.kind === "local"));
  const where = ent.file
    ? `<button type="button" data-jump-file="${esc(ent.file)}" data-jump-line="${ent.line || 1}">${esc(ent.file)}:${ent.line || 1}</button>`
    : "";
  const metric = ["procedure", "function", "native", "form", "unit", "native-file"].includes(ent.kind)
    ? `<span>${ent.loc || 0} lines</span><span>complexity ${ent.complexity || 0}</span><span>depth ${ent.depth || 0}</span><span>in ${ent.fanIn || 0}</span><span>out ${ent.fanOut || 0}</span>`
    : "";
  const nrefs = (S.db.refs || []).filter((ref) => ref.src === ent.id || ref.dst === ent.id).length;
  const memberHtml = members.map((item) =>
    `<button type="button" data-jump-file="${esc(item.file || "")}" data-jump-line="${item.line || 1}">${esc(item.name)}: ${esc(item.type || "")}</button>`).join("");
  return `<p class="ex-kicker">${esc(kindLabel(ent.kind))}</p>
    <h2>${esc(ent.qname)}</h2>
    <p class="ex-sig">${esc(signature(ent))}</p>
    <div class="chips">${metric}<button type="button" data-lens="refs">${nrefs} refs</button>${where}</div>
    ${memberHtml ? `<div class="members">${memberHtml}</div>` : ""}`;
}

function callRefs(id, dir) {
  return (S.db.refs || []).filter((ref) => ref.kind === "call" && (dir === "in" ? ref.dst === id : ref.src === id));
}

function callColumns(rootId, dir) {
  const columns = [];
  let frontier = [{ id: rootId, key: "f-" + rootId }];
  const seen = new Set([rootId]);
  for (let depth = 1; depth <= 4; depth += 1) {
    const grouped = new Map();
    frontier.forEach((parent) => {
      callRefs(parent.id, dir).forEach((ref) => {
        const other = dir === "in" ? ref.src : ref.dst;
        if (!grouped.has(other)) {
          grouped.set(other, {
            id: other,
            key: (dir === "in" ? "i-" : "o-") + depth + "-" + other,
            cycle: seen.has(other),
            parents: [],
            sites: [],
          });
        }
        const node = grouped.get(other);
        if (!node.parents.includes(parent.key)) node.parents.push(parent.key);
        node.sites.push(ref);
      });
    });
    if (!grouped.size) break;
    const col = [...grouped.values()].sort((a, b) => nameOf(a.id).localeCompare(nameOf(b.id)));
    columns.push(col);
    const next = [];
    col.forEach((node) => {
      if (!node.cycle && !seen.has(node.id)) {
        seen.add(node.id);
        next.push({ id: node.id, key: node.key });
      }
    });
    frontier = next;
    if (!frontier.length) break;
  }
  return columns;
}

function cardHtml(node) {
  const ent = entity(node.id);
  const n = node.sites.length;
  const site = node.sites[0];
  const jump = site && site.file
    ? `<button type="button" class="jump" data-jump-file="${esc(site.file)}" data-jump-line="${site.line}">${esc(site.file)}:${site.line}${n > 1 ? " · " + n : ""}</button>`
    : "";
  return `<div class="bf-card" data-key="${node.key}">
    <button type="button" class="bf-focus" data-focus="${node.id}"><b>${esc(ent ? ent.qname : node.id)}</b></button>
    <div class="meta">${esc(node.cycle ? "cycle" : kindLabel(ent ? ent.kind : ""))}${n ? " · " + n + (n === 1 ? " call" : " calls") : ""}</div>
    ${jump}
  </div>`;
}

function focusCard(ent) {
  return `<div class="bf-card focus" data-key="f-${ent.id}"><b>${esc(ent.qname)}</b><div class="meta">${esc(kindLabel(ent.kind))}</div></div>`;
}

function callsHtml(ent) {
  graphEdges = [];
  const callers = callColumns(ent.id, "in");
  const callees = callColumns(ent.id, "out");
  callers.forEach((col) => col.forEach((node) => node.parents.forEach((key) => {
    graphEdges.push({ from: key, to: node.key, side: "in" });
  })));
  callees.forEach((col) => col.forEach((node) => node.parents.forEach((key) => {
    graphEdges.push({ from: key, to: node.key, side: "out" });
  })));
  const cols = [];
  for (let i = callers.length - 1; i >= 0; i -= 1) {
    cols.push(`<div class="g-col">${callers[i].map(cardHtml).join("")}</div>`);
  }
  cols.push(`<div class="g-col">${focusCard(ent)}</div>`);
  callees.forEach((col) => cols.push(`<div class="g-col">${col.map(cardHtml).join("")}</div>`));
  const empty = !callers.length && !callees.length
    ? `<p class="hint">Nothing calls this, and it calls nothing.</p>` : "";
  return `<p class="hint">Click a name to focus it. Click a line to open the source. A repeated routine is a cycle and stops there. The graph walks four levels.</p>
    ${empty}
    <div id="graph" class="graph"><div class="graph-body"><svg></svg><div class="g-cols">${cols.join("")}</div></div></div>`;
}

function flowHtml(ent) {
  const cfg = S.db.cfgs && S.db.cfgs[String(ent.id)];
  if (!cfg || !cfg.blocks || !cfg.blocks.length) {
    return `<p class="hint">${esc(ent.qname)} has no control-flow graph. Graphs are kept for routines and native functions.</p>`;
  }
  graphEdges = [];
  const byId = new Map(cfg.blocks.map((block) => [block.id, block]));
  const rank = new Map();
  const queue = [[cfg.entry, 0]];
  rank.set(cfg.entry, 0);
  while (queue.length) {
    const pair = queue.shift();
    const block = byId.get(pair[0]);
    if (!block) continue;
    (block.succs || []).forEach((succ) => {
      if (rank.has(succ)) return;
      rank.set(succ, pair[1] + 1);
      queue.push([succ, pair[1] + 1]);
    });
  }
  let extra = Math.max(0, ...[...rank.values(), 0]);
  cfg.blocks.forEach((block) => {
    if (!rank.has(block.id)) {
      extra += 1;
      rank.set(block.id, extra);
    }
  });
  const rows = [];
  cfg.blocks.forEach((block) => {
    const at = rank.get(block.id);
    (rows[at] = rows[at] || []).push(block);
  });
  cfg.blocks.forEach((block) => {
    (block.succs || []).forEach((succ) => {
      const back = (rank.get(succ) || 0) <= (rank.get(block.id) || 0);
      graphEdges.push({ from: "b-" + block.id, to: "b-" + succ, side: back ? "back" : "down" });
    });
  });
  const html = rows.filter(Boolean).map((row) => `<div class="cfg-row">${row.map((block) => {
    const text = (block.text || []).join("\n");
    const entry = block.id === cfg.entry ? " entry" : "";
    return `<button type="button" class="cfg-node${entry}" data-key="b-${block.id}" data-jump-file="${esc(ent.file || "")}" data-jump-line="${block.line || 1}"><span class="lab">${esc(block.label)}</span><span class="txt">${esc(text)}</span></button>`;
  }).join("")}</div>`).join("");
  return `<p class="hint">Click a block to open its line. A dashed edge loops back.</p>
    <div id="graph" class="graph"><div class="graph-body"><svg></svg><div class="cfg-rows">${html}</div></div></div>`;
}

function family(id) {
  const ids = new Set([id]);
  let grew = true;
  while (grew) {
    grew = false;
    S.db.entities.forEach((ent) => {
      if (ent.parent != null && ids.has(ent.parent) && !ids.has(ent.id)) {
        ids.add(ent.id);
        grew = true;
      }
    });
  }
  return ids;
}

function refsHtml(ent) {
  const kinds = ["all", "call", "read", "write", "uses"];
  const scopes = [["here", "This entity"], ["inside", "Inside"], ["project", "Project"]];
  const ids = S.refScope === "project" ? null : (S.refScope === "inside" ? family(ent.id) : new Set([ent.id]));
  const rows = (S.db.refs || []).filter((ref) => {
    if (S.refKind !== "all" && ref.kind !== S.refKind) return false;
    if (!ids) return true;
    return ids.has(ref.src) || ids.has(ref.dst);
  });
  const hereCount = (S.db.refs || []).filter((ref) => ref.src === ent.id || ref.dst === ent.id).length;
  const note = S.refScope === "here" && !hereCount
    ? `<p class="hint">Nothing refers to this entity directly. Inside includes its routines and widgets.</p>` : "";
  const kindBar = `<div class="scope">${kinds.map((kind) => `<button type="button" data-refkind="${kind}" class="${S.refKind === kind ? "on" : ""}">${kind}</button>`).join("")}</div>`;
  const scopeBar = `<div class="scope">${scopes.map(([id, label]) => `<button type="button" data-scope="${id}" class="${S.refScope === id ? "on" : ""}">${label}</button>`).join("")}</div>`;
  const list = rows.length ? rows.map((ref) => {
    const src = entity(ref.src);
    const dst = entity(ref.dst);
    const where = ref.file
      ? `<button type="button" class="where" data-jump-file="${esc(ref.file)}" data-jump-line="${ref.line}">${esc(ref.file)}:${ref.line}</button>`
      : "";
    return `<div class="ref-row"><span class="kind">${esc(ref.kind)}</span><button type="button" data-focus="${ref.src}">${esc(src ? src.qname : ref.src)}</button><span>→</span><button type="button" data-focus="${ref.dst}">${esc(dst ? dst.qname : ref.dst)}</button>${where}</div>`;
  }).join("") : `<p class="hint">No references in this scope.</p>`;
  return kindBar + scopeBar + note + `<div class="ref-list">${list}</div>`;
}

function archHtml(ent) {
  const uses = (S.db.refs || []).filter((ref) => ref.kind === "uses");
  const usesHtml = uses.length ? uses.map((ref) => {
    const src = entity(ref.src);
    const dst = entity(ref.dst);
    return `<div class="ref-row"><span class="kind">uses</span><button type="button" data-focus="${ref.src}">${esc(src ? src.qname : ref.src)}</button><span>→</span><button type="button" data-focus="${ref.dst}">${esc(dst ? dst.qname : ref.dst)}</button></div>`;
  }).join("") : `<p class="hint">No unit dependencies.</p>`;
  const seen = new Map();
  (S.db.refs || []).filter((ref) => ref.kind === "call").forEach((ref) => {
    const src = entity(ref.src);
    const dst = entity(ref.dst);
    if (!src || !dst || !src.file || !dst.file || src.file === dst.file) return;
    const key = src.id + ">" + dst.id;
    if (!seen.has(key)) seen.set(key, { src, dst, n: 0, ref });
    seen.get(key).n += 1;
  });
  const crossHtml = seen.size ? [...seen.values()].map((item) =>
    `<div class="ref-row"><span class="kind">${item.n}</span><button type="button" data-focus="${item.src.id}" data-open-lens="calls">${esc(item.src.qname)}</button><span>→</span><button type="button" data-focus="${item.dst.id}" data-open-lens="calls">${esc(item.dst.qname)}</button><button type="button" class="where" data-jump-file="${esc(item.ref.file)}" data-jump-line="${item.ref.line}">${esc(item.ref.file)}:${item.ref.line}</button></div>`
  ).join("") : `<p class="hint">No calls leave their file.</p>`;
  const rows = S.db.entities.filter((item) => ["procedure", "function", "native"].includes(item.kind));
  const dir = S.sortDir;
  const key = S.sortKey;
  rows.sort((a, b) => {
    const av = a[key];
    const bv = b[key];
    if (typeof av === "string" || typeof bv === "string") return String(av || "").localeCompare(String(bv || "")) * dir;
    return ((av || 0) - (bv || 0)) * dir;
  });
  const maxIn = Math.max(1, ...rows.map((item) => item.fanIn || 0));
  const th = (id, label) => `<th><button type="button" data-sort="${id}">${label}${S.sortKey === id ? (S.sortDir < 0 ? " ↓" : " ↑") : ""}</button></th>`;
  const table = `<table class="grid"><tr>${th("kind", "Kind")}${th("qname", "Name")}${th("loc", "Lines")}${th("complexity", "Complexity")}${th("depth", "Depth")}${th("fanIn", "In")}${th("fanOut", "Out")}</tr>` +
    rows.map((item) => `<tr class="${item.id === ent.id ? "on" : ""}"><td>${esc(kindLabel(item.kind))}</td><td><button type="button" data-focus="${item.id}" data-open-lens="calls">${esc(item.qname)}</button></td><td>${item.loc || 0}</td><td>${item.complexity || 0}</td><td>${item.depth || 0}</td><td><i class="bar-n" style="width:${Math.max(4, ((item.fanIn || 0) / maxIn) * 48)}px"></i>${item.fanIn || 0}</td><td>${item.fanOut || 0}</td></tr>`).join("") +
    `</table>`;
  return `<div class="arch-block"><h3>Dependencies</h3>${usesHtml}</div><div class="arch-block"><h3>Calls across files</h3>${crossHtml}</div><div class="arch-block"><h3>Metrics</h3>${table}</div>`;
}

function paintExplore() {
  if (!S.db) {
    explore.innerHTML = `<div class="ex-wait">Reading the project.</div>`;
    explore.onclick = null;
    return;
  }
  if (S.explore == null || !entity(S.explore)) S.explore = pickFocus();
  const ent = entity(S.explore);
  if (!ent) {
    explore.innerHTML = `<div class="ex-wait">This project has no entities yet.</div>`;
    return;
  }
  const body = S.lens === "flow" ? flowHtml(ent) : S.lens === "refs" ? refsHtml(ent) : S.lens === "arch" ? archHtml(ent) : callsHtml(ent);
  explore.innerHTML = `
    <aside id="ex-index">
      <input id="ex-search" placeholder="Find a routine, widget, or file" value="${esc(S.query)}" autocomplete="off">
      <div id="ex-list">${indexList()}</div>
      <p class="hint">${S.db.entities.length} entities · ${(S.db.refs || []).length} references</p>
    </aside>
    <section id="ex-main">
      ${headHtml(ent)}
      <div class="lenses">${LENSES.map(([id, label]) => `<button type="button" data-lens="${id}" class="${S.lens === id ? "on" : ""}">${label}</button>`).join("")}</div>
      ${body}
    </section>`;
  const input = document.getElementById("ex-search");
  if (input) {
    input.oninput = () => {
      S.query = input.value;
      const list = document.getElementById("ex-list");
      if (list) list.innerHTML = indexList();
    };
  }
  explore.onclick = onExploreClick;
  if (S.lens === "calls" || S.lens === "flow") requestAnimationFrame(drawGraph);
}

function onExploreClick(ev) {
  const sort = ev.target.closest("[data-sort]");
  if (sort) {
    const key = sort.dataset.sort;
    if (S.sortKey === key) S.sortDir *= -1;
    else {
      S.sortKey = key;
      S.sortDir = key === "qname" || key === "kind" ? 1 : -1;
    }
    paintExplore();
    return;
  }
  const lens = ev.target.closest("[data-lens]");
  if (lens) { S.lens = lens.dataset.lens; paintExplore(); return; }
  const scope = ev.target.closest("[data-scope]");
  if (scope) { S.refScope = scope.dataset.scope; paintExplore(); return; }
  const kind = ev.target.closest("[data-refkind]");
  if (kind) { S.refKind = kind.dataset.refkind; paintExplore(); return; }
  const jumpBtn = ev.target.closest("[data-jump-file]");
  if (jumpBtn) {
    jump(jumpBtn.dataset.jumpFile, Number(jumpBtn.dataset.jumpLine || 1));
    return;
  }
  const focus = ev.target.closest("[data-focus]");
  if (focus) {
    S.explore = Number(focus.dataset.focus);
    if (focus.dataset.openLens) S.lens = focus.dataset.openLens;
    paintExplore();
  }
}

function drawGraph() {
  const host = document.getElementById("graph");
  if (!host) return;
  const body = host.querySelector(".graph-body");
  const svg = host.querySelector("svg");
  if (!body || !svg) return;
  const origin = body.getBoundingClientRect();
  if (!origin.width) return;
  const paths = graphEdges.map((edge) => {
    const from = body.querySelector(`[data-key="${CSS.escape(edge.from)}"]`);
    const to = body.querySelector(`[data-key="${CSS.escape(edge.to)}"]`);
    if (!from || !to) return "";
    const ar = from.getBoundingClientRect();
    const br = to.getBoundingClientRect();
    let d = "";
    if (edge.side === "in" || edge.side === "out") {
      const x1 = (edge.side === "in" ? ar.left : ar.right) - origin.left;
      const y1 = ar.top - origin.top + ar.height / 2;
      const x2 = (edge.side === "in" ? br.right : br.left) - origin.left;
      const y2 = br.top - origin.top + br.height / 2;
      const mx = (x1 + x2) / 2;
      d = `M ${x1} ${y1} C ${mx} ${y1}, ${mx} ${y2}, ${x2} ${y2}`;
    } else if (edge.side === "back") {
      const x1 = ar.left - origin.left + ar.width / 2;
      const y1 = ar.bottom - origin.top;
      const x2 = br.left - origin.left + br.width / 2;
      const y2 = br.top - origin.top;
      const x = Math.max(ar.right, br.right) - origin.left + 28;
      d = `M ${x1} ${y1} C ${x} ${y1}, ${x} ${y2}, ${x2} ${y2}`;
    } else {
      const x1 = ar.left - origin.left + ar.width / 2;
      const y1 = ar.bottom - origin.top;
      const x2 = br.left - origin.left + br.width / 2;
      const y2 = br.top - origin.top;
      const my = (y1 + y2) / 2;
      d = `M ${x1} ${y1} C ${x1} ${my}, ${x2} ${my}, ${x2} ${y2}`;
    }
    return `<path class="${edge.side === "back" ? "back" : ""}" d="${d}"/>`;
  }).join("");
  svg.setAttribute("width", String(body.scrollWidth));
  svg.setAttribute("height", String(body.scrollHeight));
  svg.innerHTML = paths;
}

function paintStatus() {
  const name = S.current ? S.current.name : "Droide";
  const mark = (S.dirty || S.formDirty) ? " •" : "";
  const errs = S.errors.length ? `${S.errors.length} error${S.errors.length === 1 ? "" : "s"}` : "clean";
  status.innerHTML = `<span id="st-main">${esc(name)}${mark} · ${esc(S.mode)}</span><span id="st-caret"></span><span><button type="button" id="st-health" class="${S.errors.length ? "bad" : ""}">${esc(errs)}</button>${S.apk ? `<button type="button" id="st-apk">APK</button>` : ""}</span>`;
  const health = document.getElementById("st-health");
  const apk = document.getElementById("st-apk");
  if (health) health.onclick = () => openSheet("messages");
  if (apk) apk.onclick = () => openSheet("build");
  paintCaretStatus();
}

function setStatus(text) {
  paintStatus();
  const main = document.getElementById("st-main");
  if (main) main.textContent = text;
}

function showHelp() {
  modal.hidden = false;
  modal.innerHTML = `<div class="dialog"><h2>Shortcuts</h2><pre class="dump">F9            build and launch
Ctrl+F9       compile
F12           design and code
F4            explore
F3            explore the name under the caret
Ctrl+P        open a file or routine
Ctrl+F        find in this file
Ctrl+Click    go to the declaration
Ctrl+Enter    go to the declaration
F11           pin the inspector
Ctrl+S        save
Escape        close a panel
Arrows        nudge the selected widget
Shift+arrows  nudge by 8 dp
Delete        remove the selected widget
Double-click  create NameClick(Sender: View)

The form is the screen. Choose a widget, then click the form.
F9 builds the APK and launches it. With no device attached, it starts the first emulator.
The inspector opens for the selected widget.
The status line names the routine or widget under the caret.
F3 opens that entity in Explore.
Ctrl+P opens a file or routine. Ctrl+F finds in this file.
Enter and Shift+Enter step through those matches.
Ctrl+Click jumps to its declaration.
Explore holds the cross-reference database: Call Graph, Control Flow,
references, and architecture.</pre><div class="row"><button type="button" id="cancel">Close</button></div></div>`;
  modal.querySelector("#cancel").onclick = () => { modal.hidden = true; };
  modal.onclick = (ev) => { if (ev.target === modal) modal.hidden = true; };
}

function onKey(ev) {
  const quick = !modal.hidden && document.getElementById("quick");
  if (ev.key === "Escape") {
    if (!modal.hidden) { modal.hidden = true; ev.preventDefault(); return; }
    if (S.findOn) { closeFind(true); ev.preventDefault(); return; }
    if (!sheet.hidden) { closeSheet(); ev.preventDefault(); return; }
    if (!drawer.hidden) { closeDrawer(); ev.preventDefault(); return; }
  }
  if ((ev.ctrlKey || ev.metaKey) && !ev.altKey && (ev.key === "f" || ev.key === "F")) {
    ev.preventDefault();
    if (!modal.hidden) return;
    openFind();
    return;
  }
  if (ev.target && ev.target.id === "findq" && !ev.ctrlKey && !ev.metaKey && !ev.altKey) {
    if (ev.key === "Enter") {
      ev.preventDefault();
      moveFind(ev.shiftKey ? -1 : 1);
    }
    return;
  }
  if (quick && (ev.key === "ArrowDown" || ev.key === "ArrowUp" || ev.key === "Enter")) {
    ev.preventDefault();
    moveQuick(ev);
    return;
  }
  if ((ev.ctrlKey || ev.metaKey) && (ev.key === "p" || ev.key === "P")) {
    ev.preventDefault();
    if (!modal.hidden && !quick) return;
    const seed = S.mode === "code" ? wordAtCaret() : (S.selected || "");
    openQuick(seed);
    return;
  }
  if (ev.ctrlKey && ev.key === "Enter" && S.mode === "code" && !quick) {
    ev.preventDefault();
    goToDeclaration();
    return;
  }
  if (ev.key === "F9") {
    ev.preventDefault();
    if (ev.ctrlKey) compile();
    else launch();
    return;
  }
  if (ev.key === "F12") {
    ev.preventDefault();
    setMode(S.mode === "design" ? "code" : "design");
    return;
  }
  if (ev.key === "F4") { ev.preventDefault(); setMode("explore"); return; }
  if (ev.key === "F3") {
    ev.preventDefault();
    if (typingTarget(ev.target) && ev.target !== code) return;
    const ent = contextEntity();
    if (ent) {
      S.explore = ent.id;
      S.lens = ent.kind === "widget" || ent.kind === "form" || ent.kind === "unit" ? "refs" : "calls";
    }
    setMode("explore");
    return;
  }
  if (ev.key === "F11") {
    ev.preventDefault();
    S.pinInspector = !S.pinInspector;
    if (S.pinInspector && S.form) S.mode = "design";
    paintAll();
    return;
  }
  if (ev.ctrlKey && (ev.key === "s" || ev.key === "S")) { ev.preventDefault(); save(); return; }
  if (typingTarget(ev.target) || !S.form || !S.selected || S.mode !== "design") return;
  const comp = findComp(S.selected);
  if (!comp) return;
  const step = ev.shiftKey ? 8 : 4;
  if (ev.key === "ArrowLeft") comp.left = snap(comp.left - step);
  else if (ev.key === "ArrowRight") comp.left = snap(comp.left + step);
  else if (ev.key === "ArrowUp") comp.top = snap(comp.top - step);
  else if (ev.key === "ArrowDown") comp.top = snap(comp.top + step);
  else if (ev.key === "Delete" || ev.key === "Backspace") {
    S.form.components = S.form.components.filter((item) => item.name !== comp.name);
    S.selected = null;
  } else return;
  ev.preventDefault();
  S.formDirty = true;
  paintForm();
  fit();
  paintInspector();
}

function openDialog(kind) {
  if (kind === "new") {
    modal.hidden = false;
    modal.innerHTML = `<form class="dialog" id="dlg">
      <h2>New project</h2>
      <label>Directory<input name="path" value="examples/NewApp" required></label>
      <label>Name<input name="name" value="NewApp" required></label>
      <label>Package<input name="package" value="com.example.newapp" required></label>
      <label>Title<input name="title" value="NewApp"></label>
      <div class="row"><button type="button" id="cancel">Cancel</button><button type="submit">Create</button></div>
    </form>`;
    modal.querySelector("#cancel").onclick = () => { modal.hidden = true; };
    modal.onclick = (ev) => { if (ev.target === modal) modal.hidden = true; };
    modal.querySelector("#dlg").onsubmit = async (ev) => {
      ev.preventDefault();
      const data = Object.fromEntries(new FormData(ev.target).entries());
      const view = await api("POST", "/api/new", data);
      modal.hidden = true;
      await adopt(view);
    };
    return;
  }
  const buttons = S.examples.map((item) => `<button type="button" data-path="${esc(item.path)}">${esc(item.name)}</button>`).join("");
  modal.hidden = false;
  modal.innerHTML = `<form class="dialog" id="dlg">
    <h2>Open project</h2>
    <div class="examples">${buttons}</div>
    <label>Path<input name="path" placeholder="examples/Hello" required></label>
    <div class="row"><button type="button" id="cancel">Cancel</button><button type="submit">Open</button></div>
  </form>`;
  modal.querySelector("#cancel").onclick = () => { modal.hidden = true; };
  modal.onclick = (ev) => { if (ev.target === modal) modal.hidden = true; };
  modal.querySelectorAll(".examples button").forEach((button) => {
    button.onclick = async () => {
      const view = await api("POST", "/api/open", { path: button.dataset.path });
      modal.hidden = true;
      await adopt(view);
    };
  });
  modal.querySelector("#dlg").onsubmit = async (ev) => {
    ev.preventDefault();
    const data = Object.fromEntries(new FormData(ev.target).entries());
    try {
      const view = await api("POST", "/api/open", data);
      modal.hidden = true;
      await adopt(view);
    } catch (err) {
      setStatus(err.message);
    }
  };
}

function openDrawer() {
  if (!S.project) { openDialog("open"); return; }
  const groups = [
    ["Forms", S.files.filter((file) => file.kind === "form")],
    ["Units", S.files.filter((file) => file.kind === "unit")],
    ["Native", S.files.filter((file) => file.kind === "native")],
    ["Project", S.files.filter((file) => file.kind === "project")],
  ];
  const files = groups.map(([label, list]) => `<h3>${label}</h3>` + list.map((file) =>
    `<button type="button" class="tree-item${S.current && S.current.source === file.source ? " on" : ""}" data-src="${esc(file.source)}">${esc(file.name)}</button>`
  ).join("")).join("");
  drawer.hidden = false;
  drawer.innerHTML = `<div class="drawer-panel">${files}
    <h3>Project</h3>
    <button type="button" data-act="new">New project</button>
    <button type="button" data-act="open">Open</button>
    <button type="button" data-act="save">Save</button>
    <button type="button" data-act="build">Compile</button>
    <button type="button" data-act="form">New form</button>
    <button type="button" data-act="unit">New unit</button>
    <button type="button" data-act="native">New native</button>
    <button type="button" data-act="help">Shortcuts</button>
  </div>`;
  drawer.onclick = (ev) => { if (ev.target === drawer) closeDrawer(); };
  drawer.querySelectorAll("[data-src]").forEach((button) => {
    button.onclick = () => {
      const info = S.files.find((file) => file.source === button.dataset.src);
      closeDrawer();
      if (info) openFile(info);
    };
  });
  const acts = {
    new: () => openDialog("new"),
    open: () => openDialog("open"),
    save: () => save(),
    build: () => compile(),
    form: () => addFile("form"),
    unit: () => addFile("unit"),
    native: () => addFile("native"),
    help: () => showHelp(),
  };
  drawer.querySelectorAll("[data-act]").forEach((button) => {
    button.onclick = () => {
      const act = acts[button.dataset.act];
      closeDrawer();
      if (act) act();
    };
  });
}

function closeDrawer() {
  drawer.hidden = true;
  drawer.innerHTML = "";
}

function toggleDrawer() {
  if (drawer.hidden) openDrawer();
  else closeDrawer();
}

async function addFile(kind) {
  if (!S.root) return;
  const name = prompt(kind === "form" ? "Form name" : kind === "unit" ? "Unit name" : "Native file name");
  if (!name) return;
  try {
    const view = await api("POST", "/api/add", { kind, name });
    S.files = view.files;
    S.project = view.project;
    const created = S.files.find((file) => file.source === view.created || file.form === view.created);
    if (created) await openFile(created, true);
    else paintAll();
  } catch (err) {
    setStatus(err.message);
  }
}

boot().catch((err) => {
  status.textContent = err.message;
});
