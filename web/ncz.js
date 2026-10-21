// Thin wrapper around the bindings that wasm-bindgen generates into pkg/.
import init, { convert, parse } from "./pkg/nczrs.js";

/**
 * Loads the converter module.
 * @returns {Promise<{
 *   convert: (bytes: Uint8Array) => { summary: object, dxf: Blob },
 *   parse: (bytes: Uint8Array) => object[],
 * }>} Functions that convert an NCZ file to DXF, and that list every object in it.
 */
export async function loadConverter() {
  await init();

  return {
    convert(bytes) {
      // The DXF arrives in pieces that are views of the module's memory, so each is copied.
      const chunks = [];
      const summary = convert(bytes, { write: (chunk) => chunks.push(chunk.slice()) });
      return { summary: JSON.parse(summary), dxf: new Blob(chunks, { type: "application/dxf" }) };
    },
    parse: (bytes) => JSON.parse(parse(bytes)),
  };
}
