// Thin wrapper around the bindings that wasm-bindgen generates into pkg/.
import init, { convert, parse } from "./pkg/nczrs.js";

/**
 * Loads the converter module.
 * @returns {Promise<{
 *   convert: (bytes: Uint8Array) => { summary: object, dxf: Blob, extraFiles: { suffix: string, blob: Blob }[] },
 *   parse: (bytes: Uint8Array) => object[],
 * }>} Functions that convert an NCZ file to DXF (with the files that go next to it, such as the `.prj`), and that
 * list every object in it.
 */
function mimeType(suffix) {
  const types = { prj: "text/plain", png: "image/png", jpg: "image/jpeg", bmp: "image/bmp", gif: "image/gif", rtf: "application/rtf" };
  return types[suffix.split(".").pop()] ?? "application/octet-stream";
}

export async function loadConverter() {
  await init();

  return {
    convert(bytes) {
      // The DXF and the extra files arrive as views of the module's memory, so each is copied.
      const chunks = [];
      const extraFiles = [];
      const summary = convert(bytes, {
        write: (chunk) => chunks.push(chunk.slice()),
        file: (suffix, data) => extraFiles.push({ suffix, blob: new Blob([data.slice()], { type: mimeType(suffix) }) }),
      });
      return { summary: JSON.parse(summary), dxf: new Blob(chunks, { type: "application/dxf" }), extraFiles };
    },
    parse: (bytes) => JSON.parse(parse(bytes)),
  };
}
