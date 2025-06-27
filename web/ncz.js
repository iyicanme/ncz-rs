// Thin wrapper around the C ABI exported by src/wasm.rs.

/**
 * Loads the parser module.
 * @param {string | URL | ArrayBuffer | Uint8Array} source URL of nczrs.wasm, or its bytes.
 * @returns {Promise<(bytes: Uint8Array) => object[]>} A function that parses an NCZ file.
 */
export async function loadParser(source) {
  const module = await compile(source);

  return function parse(bytes) {
    // A fresh instance per file, so a panic (a wasm trap) on one file can't leave
    // a half-updated allocator behind for the next one.
    const { memory, nczrs_alloc, nczrs_parse, nczrs_result_length } = new WebAssembly.Instance(module, {}).exports;

    const input = nczrs_alloc(bytes.length);
    new Uint8Array(memory.buffer, input, bytes.length).set(bytes);

    const output = nczrs_parse(input, bytes.length);
    const length = nczrs_result_length();
    // memory.buffer may have been replaced if the module grew its memory, so read it again.
    const json = new TextDecoder().decode(new Uint8Array(memory.buffer, output, length));

    return JSON.parse(json);
  };
}

async function compile(source) {
  if (source instanceof ArrayBuffer || ArrayBuffer.isView(source)) {
    return WebAssembly.compile(source);
  }

  const response = fetch(source);
  try {
    return await WebAssembly.compileStreaming(response);
  } catch {
    // Some static hosts don't serve .wasm as application/wasm, which streaming requires.
    const fetched = await response;
    if (!fetched.ok) throw new Error(`Could not load ${fetched.url}: HTTP ${fetched.status}`);
    return WebAssembly.compile(await fetched.arrayBuffer());
  }
}
