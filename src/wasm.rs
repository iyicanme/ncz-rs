use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct Conversion {
    summary: String,
    dxf: Vec<u8>,
}

#[wasm_bindgen]
impl Conversion {
    /// JSON of [`crate::summary::Summary`].
    #[wasm_bindgen(getter)]
    pub fn summary(&self) -> String {
        self.summary.clone()
    }

    /// Moves the DXF file out, so it is not copied.
    pub fn take_dxf(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.dxf)
    }
}

/// Parses an NCZ file and returns its objects as JSON.
#[wasm_bindgen]
pub fn parse(bytes: &[u8]) -> Result<String, JsError> {
    let parsed = crate::parse_objects(bytes)?;

    Ok(serde_json::to_string(&parsed.objects)?)
}

/// Converts an NCZ file to DXF.
#[wasm_bindgen]
pub fn convert(bytes: &[u8]) -> Result<Conversion, JsError> {
    let conversion =
        crate::summary::convert(bytes).map_err(|error| JsError::new(&error.to_string()))?;
    let summary = serde_json::to_string(&conversion.summary)?;

    Ok(Conversion {
        summary,
        dxf: conversion.dxf,
    })
}
