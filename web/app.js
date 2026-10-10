import { loadParser } from "./ncz.js";

const PAGE_SIZE = 500;

const fileInput = document.getElementById("file-input");
const dropZone = document.getElementById("drop-zone");
const dropText = document.getElementById("drop-text");
const status = document.getElementById("status");
const results = document.getElementById("results");
const fileInfo = document.getElementById("file-info");
const kindFilters = document.getElementById("kind-filters");
const search = document.getElementById("search");
const rows = document.getElementById("rows");
const showMore = document.getElementById("show-more");

const parserReady = loadParser(new URL("nczrs.wasm", import.meta.url));

let elements = [];
let activeKind = null;
let visibleCount = PAGE_SIZE;

fileInput.addEventListener("change", () => {
  if (fileInput.files.length > 0) openFile(fileInput.files[0]);
});

dropZone.addEventListener("dragover", (event) => {
  event.preventDefault();
  dropZone.classList.add("dragging");
});
dropZone.addEventListener("dragleave", () => dropZone.classList.remove("dragging"));
dropZone.addEventListener("drop", (event) => {
  event.preventDefault();
  dropZone.classList.remove("dragging");
  const file = event.dataTransfer.files[0];
  if (file) openFile(file);
});

search.addEventListener("input", () => {
  visibleCount = PAGE_SIZE;
  renderRows();
});

showMore.addEventListener("click", () => {
  visibleCount += PAGE_SIZE;
  renderRows();
});

async function openFile(file) {
  setStatus(`Parsing ${file.name}…`);
  try {
    const parse = await parserReady;
    const bytes = new Uint8Array(await file.arrayBuffer());

    const started = performance.now();
    const objects = parse(bytes);
    const elapsed = performance.now() - started;

    showObjects(objects);
    dropText.textContent = file.name;
    setStatus(`${file.name}: ${formatBytes(bytes.length)}, ${objects.length} records, ${elements.length} elements, parsed in ${elapsed.toFixed(0)} ms.`);
  } catch (error) {
    console.error(error);
    results.hidden = true;
    setStatus(`Could not parse ${file.name}: ${error.message}`, true);
  }
}

function showObjects(objects) {
  const info = { Version: [], UserInformation: [], Metadata: [], Layers: [], LayerInformation: [], LineTypes: [], Fonts: [], Libraries: [], FileHeader: [] };
  elements = [];

  objects.forEach((object, recordIndex) => {
    // Records without fields, such as block ends, serialize as plain strings.
    if (typeof object === "string") return;
    const [type, value] = Object.entries(object)[0];

    if (type === "Geometry") {
      const [kind, geometry] = Object.entries(value.geometry)[0];
      const details = geometry.code ? `[${geometry.code}] ${describe(kind, geometry)}` : describe(kind, geometry);
      elements.push({ recordIndex, kind: kindLabel(kind, geometry), layer: geometry.layer_id, details, raw: object });
    } else if (type === "Unknown") {
      elements.push({ recordIndex, kind: "Unknown record", layer: null, details: `type ${value.id}, ${value.bytes.length} bytes`, raw: object });
    } else {
      info[type]?.push(value);
    }
  });

  renderFileInfo(info);
  activeKind = null;
  visibleCount = PAGE_SIZE;
  search.value = "";
  renderKindFilters();
  renderRows();
  results.hidden = false;
}

function kindLabel(kind, geometry) {
  switch (kind) {
    case "PolyLine": return "Polyline";
    case "BlockInsert": return "Block insert";
    case "Other": return `Other geometry (${geometry.id})`;
    default: return kind;
  }
}

function describe(kind, g) {
  switch (kind) {
    case "Point": return g.name ? `${g.name} at ${point(g.position)}` : point(g.position);
    case "Line": return `${point(g.begin)} → ${point(g.end)}`;
    case "Circle": return `center ${point(g.center)}, r ${num(g.radius)}`;
    case "Arc": return `center ${point(g.center)}, r ${num(g.radius)}, ${num(g.start_angle)} rad → ${num(g.end_angle)} rad`;
    case "Text": {
      const styles = ["italic", "underline", "highlight"].filter((style) => g[style]);
      return `"${g.string}" at ${point(g.position)}, size ${num(g.size)}${styles.length ? `, ${styles.join(", ")}` : ""}`;
    }
    case "PolyLine":
    case "Contour": {
      const ends = g.points.length > 0 ? `: ${point(g.points[0])} … ${point(g.points[g.points.length - 1])}` : "";
      const closed = kind === "PolyLine" && (g.flags & 0x01) ? "closed, " : "";
      return `${g.name ? `${g.name}, ` : ""}${closed}${g.points.length} points${ends}`;
    }
    case "Symbol": return `symbol ${g.symbol} at ${point(g.position)}, scale ${num(g.scale)}`;
    case "BlockInsert": return `"${g.name}" at ${point(g.position)}, rotation ${num(g.rotation)} rad, scale ${num(g.scale_x)} × ${num(g.scale_y)}`;
    case "Image": return `image at ${point(g.position)}, ${g.properties.map((p) => p.key === "imageContent" ? `${p.key}: ${p.value.length} chars` : `${p.key}: ${p.value}`).join(", ")}`;
    default: return `${g.bytes.length} bytes`;
  }
}

function renderFileInfo(info) {
  fileInfo.replaceChildren();

  const card = (title, items) => {
    if (items.length === 0) return;
    const section = el("div", { className: "info-card" }, el("h2", {}, title));
    section.append(el("ul", {}, ...items.map((item) => el("li", {}, ...item))));
    fileInfo.append(section);
  };
  const key = (text) => el("span", { className: "key" }, text);

  card("Version", info.Version.map((v) => [v.name]));
  card("User", info.UserInformation.map((u) => [u.name]));
  card("Metadata", info.Metadata.map((m) => [key(`${m.name}: `), m.bytes ? `${m.bytes.length} bytes of binary data` : m.content]));
  const layers = info.Layers.flatMap((l) => l.layers).map((l) => l.name);
  card("Layers", layers.length ? [[layers.join(", ")]] : []);
  card("Fonts", info.Fonts.map((f) => [f.names.join(", ")]));
  card("Libraries", info.Libraries.map((l) => [l.names.join(", ")]));
  card("Layers", info.LayerInformation.map((l) => [l.name, key(` (color 0x${l.color.toString(16).padStart(8, "0")})`)]));

  const lineTypes = info.LineTypes.flatMap((l) => l.types).map((t) => t.name).filter((name) => name.trim() !== "");
  card("Line types", lineTypes.length ? [[lineTypes.join(", ")]] : []);
}

function renderKindFilters() {
  const counts = new Map();
  for (const element of elements) counts.set(element.kind, (counts.get(element.kind) ?? 0) + 1);

  const chip = (label, kind, count) => {
    const button = el("button", { className: "chip", type: "button" }, label, el("span", { className: "count" }, String(count)));
    button.setAttribute("aria-pressed", String(activeKind === kind));
    button.addEventListener("click", () => {
      activeKind = kind;
      visibleCount = PAGE_SIZE;
      renderKindFilters();
      renderRows();
    });
    return button;
  };

  kindFilters.replaceChildren(
    chip("All", null, elements.length),
    ...[...counts].sort((a, b) => b[1] - a[1]).map(([kind, count]) => chip(kind, kind, count)),
  );
}

function renderRows() {
  const query = search.value.trim().toLocaleLowerCase("tr");
  const matching = elements.filter((element) =>
    (activeKind === null || element.kind === activeKind) &&
    (query === "" || `${element.kind} ${element.layer ?? ""} ${element.details}`.toLocaleLowerCase("tr").includes(query)));

  const fragment = document.createDocumentFragment();
  for (const element of matching.slice(0, visibleCount)) {
    const row = el("tr", { className: "element" },
      el("td", {}, String(element.recordIndex)),
      el("td", {}, element.kind),
      el("td", {}, element.layer ?? "–"),
      el("td", {}, element.details));
    row.addEventListener("click", () => toggleRaw(row, element));
    fragment.append(row);
  }
  rows.replaceChildren(fragment);

  if (matching.length === 0) {
    rows.append(el("tr", {}, el("td", { colSpan: 4 }, "No matching elements.")));
  }

  showMore.hidden = matching.length <= visibleCount;
  showMore.textContent = `Show more (${matching.length - visibleCount} remaining)`;
}

function toggleRaw(row, element) {
  if (row.nextElementSibling?.classList.contains("raw")) {
    row.nextElementSibling.remove();
    return;
  }
  row.after(el("tr", { className: "raw" }, el("td", { colSpan: 4 }, el("pre", {}, prettyJson(element.raw)))));
}

// Pretty-prints JSON but keeps arrays of numbers (raw byte fields) on one line.
function prettyJson(value) {
  return JSON.stringify(value, null, 2).replace(/\[[\d\s,.eE+-]*\]/g, (array) => array.replace(/\s+/g, ""));
}

function point(c) {
  return c.z === 0 ? `(${num(c.x)}, ${num(c.y)})` : `(${num(c.x)}, ${num(c.y)}, ${num(c.z)})`;
}

function num(value) {
  return Number.isInteger(value) ? String(value) : value.toFixed(3);
}

function formatBytes(bytes) {
  return bytes < 1024 * 1024 ? `${(bytes / 1024).toFixed(1)} KB` : `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

function setStatus(text, isError = false) {
  status.textContent = text;
  status.classList.toggle("error", isError);
}

function el(tag, props, ...children) {
  const node = Object.assign(document.createElement(tag), props);
  node.append(...children.map((child) => (typeof child === "number" ? String(child) : child)));
  return node;
}
