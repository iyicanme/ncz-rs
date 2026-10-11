use jiff::{Unit, Zoned};
use nczrs::{objects_to_dxf, parse_objects};
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

/// Converts an NCZ file to a DXF file next to the working directory. With `--json`, the parsed objects are also
/// written to `out/`, which takes several times the memory of the conversion on large files.
fn main() {
    let mut json = false;
    let mut filename = None;
    for argument in std::env::args().skip(1) {
        match argument.as_str() {
            "--json" => json = true,
            _ if argument.starts_with("--") => {
                eprintln!("usage: nczrs [--json] [file.ncz]");
                std::process::exit(2);
            }
            _ => filename = Some(argument),
        }
    }
    let filename = filename.unwrap_or_else(|| "ncz/PROJE11.NCZ".to_string());
    let filename = filename.as_str();
    let mut file = File::open(filename).unwrap();
    let mut bytes = vec![];

    file.read_to_end(&mut bytes).unwrap();

    let parsed = parse_objects(&bytes).unwrap_or_else(|error| {
        eprintln!("{filename}: {error}");
        std::process::exit(1);
    });
    for failure in &parsed.failures {
        eprintln!(
            "warning: object {} at offset {:#x} (length {:?}) could not be decoded, skipped {} bytes",
            failure.object_type_id, failure.offset, failure.length, failure.skipped
        );
    }
    let objects = parsed.objects;

    if json {
        let json_filename = format!(
            "out/{}.json",
            Zoned::now()
                .round(Unit::Second)
                .unwrap()
                .strftime("%Y%m%d_%H%M%S")
        );
        let serialized = serde_json::to_vec(&objects).unwrap();
        File::create(json_filename)
            .unwrap()
            .write_all(&serialized)
            .unwrap();
    }

    let drawing = objects_to_dxf(objects);
    let dxf_filename = Path::new(filename)
        .file_stem()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string()
        + ".dxf";
    drawing.save_file(dxf_filename).unwrap()
}
