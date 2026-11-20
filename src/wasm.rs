use wasm_bindgen::prelude::*;

/// Size of the pieces in which the DXF file is passed to JavaScript.
const CHUNK_SIZE: usize = 1 << 20;

#[wasm_bindgen]
extern "C" {
    /// Receives the DXF file in pieces, see [`convert`].
    pub type DxfSink;

    /// `chunk` is a view of the module's memory that is only valid during the call, so it must be copied.
    #[wasm_bindgen(method, catch)]
    fn write(this: &DxfSink, chunk: &[u8]) -> Result<(), JsValue>;

    /// Receives a file to save next to the DXF (see [`crate::extra_files`]), named by the DXF's name without extension
    /// followed by `suffix`. `bytes` is a view of the module's memory, like the chunks of [`DxfSink::write`].
    #[wasm_bindgen(method, catch)]
    fn file(this: &DxfSink, suffix: &str, bytes: &[u8]) -> Result<(), JsValue>;
}

/// Passes what is written to a [`DxfSink`] in pieces of [`CHUNK_SIZE`].
struct SinkWriter<'a> {
    sink: &'a DxfSink,
    buffer: Vec<u8>,
}

impl std::io::Write for SinkWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.buffer.extend_from_slice(buf);
        if self.buffer.len() >= CHUNK_SIZE {
            self.flush()?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if !self.buffer.is_empty() {
            self.sink
                .write(&self.buffer)
                .map_err(|error| std::io::Error::other(format!("{error:?}")))?;
            self.buffer.clear();
        }
        Ok(())
    }
}

/// Parses an NCZ file and returns its objects as JSON.
#[wasm_bindgen]
pub fn parse(bytes: &[u8]) -> Result<String, JsError> {
    let parsed = crate::parse_objects(bytes)?;

    Ok(serde_json::to_string(&parsed.objects)?)
}

/// Converts an NCZ file to DXF, which is passed to `sink` in pieces so the whole file is never held in the module's
/// memory, followed by the files that go next to it. Returns JSON of [`crate::summary::Summary`].
#[wasm_bindgen]
pub fn convert(bytes: &[u8], sink: &DxfSink) -> Result<String, JsError> {
    let mut writer = SinkWriter {
        sink,
        buffer: Vec::with_capacity(CHUNK_SIZE),
    };
    let summary = crate::summary::convert(bytes, &mut writer)
        .map_err(|error| JsError::new(&error.to_string()))?;
    for extra in &summary.extra_files {
        sink.file(&extra.suffix, &extra.bytes)
            .map_err(|error| JsError::new(&format!("{error:?}")))?;
    }

    Ok(serde_json::to_string(&summary)?)
}
