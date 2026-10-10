// Thin wrapper around the bindings that wasm-bindgen generates into pkg/.
import init, { convert, parse } from "./pkg/nczrs.js";

/**
 * Loads the converter module.
 * @returns {Promise<{
 *   convert: (bytes: Uint8Array) => { summary: object, dxf: Uint8Array },
 *   parse: (bytes: Uint8Array) => object[],
 * }>} Functions that convert an NCZ file to DXF, and that list every object in it.
 */
export async function loadConverter() {
  await init();

  return {
    convert(bytes) {
      const conversion = convert(bytes);
      try {
        return { summary: JSON.parse(conversion.summary), dxf: conversion.take_dxf() };
      } finally {
        conversion.free();
      }
    },
    parse: (bytes) => JSON.parse(parse(bytes)),
  };
}
