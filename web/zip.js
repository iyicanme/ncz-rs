// Builds a ZIP archive in the browser, deflating with the built-in CompressionStream. No ZIP64, so the archive and
// each file must stay under 4 GiB.

const CRC_TABLE = (() => {
  const table = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    table[n] = c >>> 0;
  }
  return table;
})();

function updateCrc(crc, bytes) {
  let c = crc ^ 0xffffffff;
  for (let i = 0; i < bytes.length; i++) c = CRC_TABLE[(c ^ bytes[i]) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

// Deflates a blob, measuring its CRC-32 and size on the way.
async function deflate(blob) {
  let crc = 0;
  let size = 0;
  const measure = new TransformStream({
    transform(chunk, controller) {
      crc = updateCrc(crc, chunk);
      size += chunk.length;
      controller.enqueue(chunk);
    },
  });
  const compressed = await new Response(
    blob.stream().pipeThrough(measure).pipeThrough(new CompressionStream("deflate-raw")),
  ).blob();
  return { crc, size, compressed };
}

// The MS-DOS time and date fields of a ZIP entry.
function dosDateTime(date) {
  const time = (date.getHours() << 11) | (date.getMinutes() << 5) | (date.getSeconds() >> 1);
  const day = ((date.getFullYear() - 1980) << 9) | ((date.getMonth() + 1) << 5) | date.getDate();
  return { time, day };
}

/**
 * Builds a ZIP archive of the files.
 * @param {{ name: string, blob: Blob }[]} files
 * @returns {Promise<Blob>}
 */
export async function zip(files) {
  const encoder = new TextEncoder();
  const { time, day } = dosDateTime(new Date());
  const parts = [];
  const central = [];
  let offset = 0;

  for (const { name, blob } of files) {
    const { crc, size, compressed } = await deflate(blob);
    const nameBytes = encoder.encode(name);
    if (size >= 0xffffffff || compressed.size >= 0xffffffff || offset >= 0xffffffff) {
      throw new Error("the files are too large for a ZIP archive");
    }

    // Shared by the local header (from version needed on) and the central directory entry.
    const fields = (view, at) => {
      view.setUint16(at, 20, true); // version needed: 2.0, deflate
      view.setUint16(at + 2, 0x0800, true); // flags: UTF-8 name
      view.setUint16(at + 4, 8, true); // method: deflate
      view.setUint16(at + 6, time, true);
      view.setUint16(at + 8, day, true);
      view.setUint32(at + 10, crc, true);
      view.setUint32(at + 14, compressed.size, true);
      view.setUint32(at + 18, size, true);
      view.setUint16(at + 22, nameBytes.length, true);
      view.setUint16(at + 24, 0, true); // extra field length
    };

    const local = new DataView(new ArrayBuffer(30));
    local.setUint32(0, 0x04034b50, true);
    fields(local, 4);
    parts.push(local, nameBytes, compressed);

    const entry = new DataView(new ArrayBuffer(46));
    entry.setUint32(0, 0x02014b50, true);
    entry.setUint16(4, 20, true); // version made by
    fields(entry, 6);
    // Comment length, disk number, internal and external attributes are 0.
    entry.setUint32(42, offset, true);
    central.push(entry, nameBytes);

    offset += 30 + nameBytes.length + compressed.size;
  }

  const centralSize = central.reduce((sum, part) => sum + part.byteLength, 0);
  const end = new DataView(new ArrayBuffer(22));
  end.setUint32(0, 0x06054b50, true);
  end.setUint16(8, files.length, true);
  end.setUint16(10, files.length, true);
  end.setUint32(12, centralSize, true);
  end.setUint32(16, offset, true);

  return new Blob([...parts, ...central, end], { type: "application/zip" });
}
