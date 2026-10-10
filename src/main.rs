use jiff::{Unit, Zoned};
use nczrs::{objects_to_dxf, parse_objects};
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

fn main() {
    let filename = std::env::args().nth(1).unwrap_or_else(|| "ncz/PROJE11.NCZ".to_string());
    let filename = filename.as_str();
    let mut file = File::open(filename).unwrap();
    let mut bytes = vec![];

    file.read_to_end(&mut bytes).unwrap();

    let objects = parse_objects(&bytes);

    let serialized = serde_json::to_vec(&objects).unwrap();

    let json_filename = format!(
        "out/{}.json",
        &Zoned::now()
            .round(Unit::Second)
            .unwrap()
            .strftime("%Y%m%d_%H%M%S")
    );

    let mut outfile = File::create(json_filename).unwrap();
    let _ = outfile.write(&serialized).unwrap();

    let drawing = objects_to_dxf(objects);
    let dxf_filename = Path::new(filename).file_stem().unwrap().to_str().unwrap().to_string() + ".dxf";
    drawing.save_file(dxf_filename).unwrap()
}
