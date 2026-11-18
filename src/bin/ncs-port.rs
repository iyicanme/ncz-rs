//! Converts NetCAD symbol libraries (`.NCS`) into the symbol files that `nczrs` embeds, one file per symbol, see
//! `symbols/README.md`.
//!
//! Usage: `ncs-port out_dir file.NCS...`
//!
//! The symbols of `F2.NCS` are written to `out_dir/F2/<number>.txt`. Symbols without strokes get no file.
//!
//! An NCS file is a stroke font. It starts with its size (u16) and 25 zero bytes, followed by a table of 256 records of
//! 10 bytes, one per symbol number: the offset of the strokes (u16, in 4-byte elements, `0xffff` when the symbol
//! is not defined), the number of elements (u8), a byte of unknown meaning, the origin as east and north (u8 each),
//! and the size (f32). The elements follow the table. An element is four signed bytes: a line from `(north1, east1)`
//! to `(north2, east2)`, or, when the last byte is `0x81`, a circle with the centre `(north, east)` and the radius in
//! the third byte. A symbol drawn with scale 1 is `size / 32` drawing units per grid step, around the origin. This
//! matches all 144 symbols that NetCAD 5 exported in the sample DXF files, and the bounds of 13,000 symbols in the
//! NCZ corpus.

use std::fmt::Write as _;
use std::path::Path;

const USAGE: &str = "usage: ncs-port out_dir file.NCS...";

/// Offset of the symbol table.
const TABLE: usize = 0x1b;
const RECORD_LENGTH: usize = 10;
const SYMBOLS: usize = 256;
/// Offset of the stroke elements, after the table.
const ELEMENTS: usize = TABLE + RECORD_LENGTH * SYMBOLS;
const ELEMENT_LENGTH: usize = 4;
/// Marks an element as a circle when in its last byte.
const CIRCLE: u8 = 0x81;
/// Grid steps that make up the size of a symbol.
const GRID: f64 = 32.0;

/// A number with at most 6 decimals and no trailing zeros. The sizes are f32, so further digits are noise.
fn number(value: f64) -> String {
    let text = format!("{value:.6}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    match text {
        "-0" => "0".to_owned(),
        text => text.to_owned(),
    }
}

/// The symbol file of a symbol, `None` when the symbol has no strokes.
fn symbol_file(bytes: &[u8], library: &str, symbol: usize) -> Result<Option<String>, String> {
    let record = &bytes[TABLE + RECORD_LENGTH * symbol..][..RECORD_LENGTH];
    let offset = u16::from_le_bytes([record[0], record[1]]);
    let count = usize::from(record[2]);
    if offset == 0xffff || count == 0 {
        return Ok(None);
    }
    let (origin_east, origin_north) = (f64::from(record[4]), f64::from(record[5]));
    let size = f64::from(f32::from_le_bytes(record[6..10].try_into().unwrap()));
    let unit = size / GRID;

    let start = ELEMENTS + usize::from(offset) * ELEMENT_LENGTH;
    let elements = bytes
        .get(start..start + count * ELEMENT_LENGTH)
        .ok_or_else(|| format!("symbol {symbol}: strokes past the end of the file"))?;

    let east = |value: u8| number((f64::from(value as i8) - origin_east) * unit);
    let north = |value: u8| number((f64::from(value as i8) - origin_north) * unit);
    let mut file = format!("# Symbol {symbol} of {library}\n");
    for &[n1, e1, n2, e2] in elements.as_chunks::<ELEMENT_LENGTH>().0 {
        if e2 == CIRCLE {
            // A circle of radius 0 draws nothing (the innermost ring of `PLAN.NCS` symbol 156).
            if n2 == 0 {
                continue;
            }
            let radius = number(f64::from(n2) * unit);
            writeln!(file, "circle {} {} {radius}", east(e1), north(n1)).unwrap();
        } else {
            writeln!(
                file,
                "line {} {} {} {}",
                east(e1),
                north(n1),
                east(e2),
                north(n2)
            )
            .unwrap();
        }
    }
    Ok(Some(file))
}

fn port(path: &Path, out_dir: &Path) -> Result<usize, String> {
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    if bytes.len() < ELEMENTS
        || usize::from(u16::from_le_bytes([bytes[0], bytes[1]])) != bytes.len()
    {
        return Err("not an NCS file".to_owned());
    }
    let library = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or("the file name is not UTF-8")?
        .to_uppercase();
    let file_name = format!("{library}.NCS");
    let dir = out_dir.join(&library);
    std::fs::create_dir_all(&dir).map_err(|error| format!("{}: {error}", dir.display()))?;

    let mut written = 0;
    for symbol in 0..SYMBOLS {
        match symbol_file(&bytes, &file_name, symbol) {
            Ok(Some(file)) => {
                let target = dir.join(format!("{symbol}.txt"));
                std::fs::write(&target, file)
                    .map_err(|error| format!("{}: {error}", target.display()))?;
                written += 1;
            }
            Ok(None) => {}
            Err(error) => eprintln!("warning: {}: {error}", path.display()),
        }
    }
    Ok(written)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [out_dir, files @ ..] = args.as_slice() else {
        eprintln!("{USAGE}");
        std::process::exit(2);
    };
    if files.is_empty() {
        eprintln!("{USAGE}");
        std::process::exit(2);
    }

    let mut failed = false;
    for file in files {
        match port(Path::new(file), Path::new(out_dir)) {
            Ok(written) => println!("{file}: {written} symbols"),
            Err(error) => {
                eprintln!("{file}: {error}");
                failed = true;
            }
        }
    }
    std::process::exit(i32::from(failed));
}
