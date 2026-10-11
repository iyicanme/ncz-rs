import { loadConverter } from "./ncz.js";

const fileInput = document.getElementById("file-input");
const dropZone = document.getElementById("drop-zone");
const dropText = document.getElementById("drop-text");
const status = document.getElementById("status");
const results = document.getElementById("results");
const fileInfo = document.getElementById("file-info");
const contents = document.getElementById("contents");
const dxfInfo = document.getElementById("dxf-info");
const downloadDxf = document.getElementById("download-dxf");
const toggleDetails = document.getElementById("toggle-details");
const details = document.getElementById("details");
const detailInfo = document.getElementById("detail-info");
const kindFilters = document.getElementById("kind-filters");
const search = document.getElementById("search");
const rows = document.getElementById("rows");
const showMore = document.getElementById("show-more");

const PAGE_SIZE = 500;

const converterReady = loadConverter();

// The open file, kept so its details can be listed on demand.
let currentFile = null;
let detailsLoaded = false;
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

toggleDetails.addEventListener("click", async () => {
  const expand = details.hidden;
  if (expand && !detailsLoaded) {
    toggleDetails.disabled = true;
    const previousStatus = status.textContent;
    setStatus(`Listing the contents of ${currentFile.name}…`);
    await new Promise((resolve) => requestAnimationFrame(() => setTimeout(resolve)));
    try {
      const { parse } = await converterReady;
      showObjects(parse(currentFile.bytes));
      detailsLoaded = true;
      setStatus(previousStatus);
    } catch (error) {
      console.error(error);
      setStatus(`Could not list the contents of ${currentFile.name}: ${error.message}.`, true);
      return;
    } finally {
      toggleDetails.disabled = false;
    }
  }

  details.hidden = !expand;
  toggleDetails.setAttribute("aria-expanded", String(expand));
});

async function openFile(file) {
  results.hidden = true;
  currentFile = null;
  resetDetails();
  if (downloadDxf.href) URL.revokeObjectURL(downloadDxf.href);
  dropText.textContent = file.name;
  setStatus(`Converting ${file.name}…`);
  // Let the status paint before the conversion blocks the page.
  await new Promise((resolve) => requestAnimationFrame(() => setTimeout(resolve)));

  try {
    const { convert } = await converterReady;
    const bytes = new Uint8Array(await file.arrayBuffer());

    const started = performance.now();
    const { summary, dxf } = convert(bytes);
    const elapsed = performance.now() - started;
    if (!summary.file.version && Object.keys(summary.file.geometries).length === 0) {
      throw new Error("no NCZ data found");
    }

    const dxfName = `${file.name.replace(/\.[^.]*$/, "")}.dxf`;
    downloadDxf.href = URL.createObjectURL(new Blob([dxf], { type: "application/dxf" }));
    downloadDxf.download = dxfName;
    downloadDxf.textContent = `Download ${dxfName}`;

    currentFile = { name: file.name, bytes };
    showSummary(file, summary);
    results.hidden = false;
    setStatus(`Converted in ${(elapsed / 1000).toFixed(1)} s.`);
  } catch (error) {
    console.error(error);
    // A Rust panic traps the module, which then can't be used again.
    const hint = error instanceof WebAssembly.RuntimeError ? " Reload the page before trying again." : "";
    setStatus(`Could not convert ${file.name}: ${error.message}.${hint}`, true);
  }
}

function showSummary(file, { file: ncz, dxf }) {
  const geometryCount = sum(ncz.geometries);
  const skippedCount = sum(dxf.skipped);

  fill(fileInfo, [
    ["Name", file.name],
    ["Size", formatBytes(file.size)],
    ["NetCAD version", ncz.version],
    ["User", ncz.user],
    ["Scale", ncz.scale ? `1:${Math.round(ncz.scale * 1000).toLocaleString()}` : null],
    ["Coordinate system", ncz.coordinate_system],
    ["Extent", ncz.lower_bound && ncz.upper_bound ? extent(ncz.lower_bound, ncz.upper_bound) : null],
  ]);

  fill(contents, [
    ["Geometries", geometryCount.toLocaleString()],
    ...byCount(ncz.geometries),
    ["Layers", ncz.layers.toLocaleString()],
    ["Line types", ncz.line_types.toLocaleString()],
    ["Blocks", ncz.blocks.toLocaleString()],
    ["Unreadable objects", ncz.unreadable === 0 ? null : ncz.unreadable.toLocaleString()],
  ]);

  fill(dxfInfo, [
    ["Size", formatBytes(dxf.size)],
    ["Entities", sum(dxf.entities).toLocaleString()],
    ...byCount(dxf.entities),
    ["Layers", dxf.layers.toLocaleString()],
    ["Line types", dxf.line_types.toLocaleString()],
    ["Blocks", dxf.blocks.toLocaleString()],
    ["Not converted", skippedCount === 0 ? "nothing" :
      `${skippedCount.toLocaleString()} (${Object.entries(dxf.skipped).map(([kind, count]) => `${kind.toLowerCase()} ${count.toLocaleString()}`).join(", ")})`],
  ]);
}

// Fills a <dl> with [term, value] rows, skipping missing values. Rows from byCount are indented.
function fill(list, rows) {
  list.replaceChildren(...rows
    .filter(([, value]) => value !== null && value !== undefined && value !== "")
    .flatMap(([term, value, nested]) => [
      el("dt", { className: nested ? "nested" : "" }, term),
      el("dd", {}, value),
    ]));
}

function byCount(counts) {
  return Object.entries(counts)
    .sort((a, b) => b[1] - a[1])
    .map(([kind, count]) => [kind, count.toLocaleString(), true]);
}

function sum(counts) {
  return Object.values(counts).reduce((total, count) => total + count, 0);
}

function extent(lower, upper) {
  const size = `${num(upper.x - lower.x)} × ${num(upper.y - lower.y)}`;
  return `${size} (${num(lower.x)}, ${num(lower.y)} – ${num(upper.x)}, ${num(upper.y)})`;
}

function num(value) {
  return value.toLocaleString(undefined, { maximumFractionDigits: 2 });
}

function resetDetails() {
  detailsLoaded = false;
  details.hidden = true;
  toggleDetails.setAttribute("aria-expanded", "false");
  elements = [];
  detailInfo.replaceChildren();
  kindFilters.replaceChildren();
  rows.replaceChildren();
}

function showObjects(objects) {
  const info = { Version: [], Scale: [], RasterReferences: [], UserInformation: [], Metadata: [], Layers: [], LayerInformation: [], LineTypes: [], Fonts: [], Libraries: [] };
  elements = [];
  const layerNames = objects.find((object) => object.Layers)?.Layers.layers.map((layer) => layer.name) ?? [];

  objects.forEach((object, recordIndex) => {
    // Records without fields, such as block ends, serialize as plain strings.
    if (typeof object === "string") return;
    const [type, value] = Object.entries(object)[0];

    if (type === "Geometry") {
      const [kind, geometry] = Object.entries(value.geometry)[0];
      const details = geometry.code ? `[${geometry.code}] ${describe(kind, geometry)}` : describe(kind, geometry);
      elements.push({ recordIndex, kind: kindLabel(kind, geometry), layer: layerNames[geometry.layer_id] ?? geometry.layer_id, details, raw: object });
    } else if (type === "Unknown") {
      elements.push({ recordIndex, kind: "Unknown record", layer: null, details: `type ${value.id}, ${value.bytes.length} bytes`, raw: object });
    } else {
      info[type]?.push(value);
    }
  });

  renderDetailInfo(info);
  activeKind = null;
  visibleCount = PAGE_SIZE;
  search.value = "";
  renderKindFilters();
  renderRows();
}

function kindLabel(kind, geometry) {
  switch (kind) {
    case "PolyLine": return "Polyline";
    case "BlockInsert": return "Block insert";
    case "MapSheet": return "Map sheet";
    case "Other": return `Other geometry (${geometry.id})`;
    default: return kind;
  }
}

function describe(kind, g) {
  switch (kind) {
    case "Point": return g.name ? `${g.name} at ${point(g.position)}` : point(g.position);
    case "Line": return `${point(g.begin)} → ${point(g.end)}`;
    case "Circle": return `center ${point(g.center)}, r ${dec(g.radius)}`;
    case "Arc": return `center ${point(g.center)}, r ${dec(g.radius)}, ${dec(g.start_angle)} rad → ${dec(g.end_angle)} rad`;
    case "Text": {
      const styles = ["italic", "underline", "highlight"].filter((style) => g[style]);
      return `"${g.string}" at ${point(g.position)}, size ${dec(g.size)}${styles.length ? `, ${styles.join(", ")}` : ""}`;
    }
    case "PolyLine":
    case "Contour": {
      const ends = g.points.length > 0 ? `: ${point(g.points[0])} … ${point(g.points[g.points.length - 1])}` : "";
      const closed = kind === "PolyLine" && (g.flags & 0x01) ? "closed, " : "";
      return `${g.name ? `${g.name}, ` : ""}${closed}${g.points.length} points${ends}`;
    }
    case "Symbol": return `symbol ${g.symbol} at ${point(g.position)}, scale ${dec(g.scale)}`;
    case "BlockInsert": return `"${g.name}" at ${point(g.position)}, rotation ${dec(g.rotation)} rad, scale ${dec(g.scale_x)} × ${dec(g.scale_y)}`;
    case "Image": return `object at ${point(g.position)}, ${g.properties.map((p) => p.value.length > 80 ? `${p.key}: ${p.value.length} chars` : `${p.key}: ${p.value}`).join(", ")}`;
    case "Window": return `${g.name ? `${g.name}, ` : ""}${point(g.corner1)} – ${point(g.corner2)}`;
    case "MapSheet": return `${g.name}, ${point(g.lower_bound)} – ${point(g.upper_bound)}`;
    case "Triangle": return g.vertices.map(point).join(", ");
    case "Empty": return "no data";
    default: return `${g.bytes.length} bytes`;
  }
}

function renderDetailInfo(info) {
  detailInfo.replaceChildren();

  const card = (title, items) => {
    if (items.length === 0) return;
    const section = el("div", { className: "info-card" }, el("h2", {}, title));
    section.append(el("ul", {}, ...items.map((item) => el("li", {}, ...item))));
    detailInfo.append(section);
  };
  const key = (text) => el("span", { className: "key" }, text);

  card("Version", info.Version.map((v) => [v.name]));
  card("User", info.UserInformation.map((u) => [u.name]));
  card("Scale", info.Scale.map((s) => [`1:${Math.round(s.scale * 1000)}`]));
  card("Rasters", info.RasterReferences.flatMap((r) => r.references).map((r) => [r.path]));
  card("Metadata", info.Metadata.map((m) => [key(`${m.name}: `), describeMetadata(m.content)]));
  const layers = info.Layers.flatMap((l) => l.layers).map((l) => l.name);
  card("Layers", layers.length ? [[layers.join(", ")]] : []);
  card("Fonts", info.Fonts.map((f) => [f.names.join(", ")]));
  card("Libraries", info.Libraries.map((l) => [l.names.join(", ")]));
  card("Layers", info.LayerInformation.map((l) => [l.name, key(` (color 0x${l.color.toString(16).padStart(8, "0")})`)]));

  const lineTypes = info.LineTypes.flatMap((l) => l.types).map((t) => t.name).filter((name) => name.trim() !== "");
  card("Line types", lineTypes.length ? [[lineTypes.join(", ")]] : []);
}

function describeMetadata(content) {
  const [kind, value] = Object.entries(content)[0];
  switch (kind) {
    case "Text": return value;
    case "SymbolLibraries": return value.libraries.map((l) => `${l.name} (scale ${dec(l.scale)})`).join(", ");
    default: return `${value.length} bytes of binary data`;
  }
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
      el("td", {}, String(element.layer ?? "–")),
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
  return c.z === 0 ? `(${dec(c.x)}, ${dec(c.y)})` : `(${dec(c.x)}, ${dec(c.y)}, ${dec(c.z)})`;
}

// Plain number formatting for coordinates in the details table.
function dec(value) {
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
  node.append(...children);
  return node;
}
