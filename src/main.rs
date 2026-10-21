use nczrs::{parse_objects, write_dxf};
use std::io::Write;
use std::path::{Path, PathBuf};

const USAGE: &str = "usage: nczrs [--json] [-o file.dxf] file.ncz

Converts an NCZ file to DXF.

  -o, --output file.dxf  where to write the DXF, by default <name>.dxf in the working directory
      --json             also write the parsed objects to <name>.json in the working directory, which takes
                         several times the memory of the conversion on large files
  -h, --help             show this help";

struct Options {
    input: PathBuf,
    output: PathBuf,
    json: bool,
}

/// Prints the message and exits with the code.
fn exit(message: impl std::fmt::Display, code: i32) -> ! {
    eprintln!("{message}");
    std::process::exit(code)
}

fn parse_options() -> Options {
    let mut json = false;
    let mut input = None;
    let mut output = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            "--json" => json = true,
            "-o" | "--output" => match args.next() {
                Some(path) => output = Some(PathBuf::from(path)),
                None => exit(USAGE, 2),
            },
            _ if arg.starts_with('-') => exit(USAGE, 2),
            _ if input.is_some() => exit(USAGE, 2),
            _ => input = Some(PathBuf::from(arg)),
        }
    }

    let Some(input) = input else { exit(USAGE, 2) };
    let output = output.unwrap_or_else(|| {
        let stem = input.file_stem().unwrap_or_default();
        Path::new(stem).with_extension("dxf")
    });
    Options {
        input,
        output,
        json,
    }
}

fn main() {
    let options = parse_options();
    let input = options.input.display();
    let output = options.output.display();

    let bytes =
        std::fs::read(&options.input).unwrap_or_else(|error| exit(format!("{input}: {error}"), 1));

    if options.json {
        let parsed =
            parse_objects(&bytes).unwrap_or_else(|error| exit(format!("{input}: {error}"), 1));
        let json_path =
            Path::new(options.input.file_stem().unwrap_or_default()).with_extension("json");
        let serialized = serde_json::to_vec(&parsed.objects).expect("objects serialize");
        std::fs::write(&json_path, serialized)
            .unwrap_or_else(|error| exit(format!("{}: {error}", json_path.display()), 1));
    }

    let file = std::fs::File::create(&options.output)
        .unwrap_or_else(|error| exit(format!("{output}: {error}"), 1));
    let mut writer = std::io::BufWriter::new(file);
    let result = write_dxf(&bytes, &mut writer, |item| {
        if let Err(failure) = item {
            eprintln!(
                "warning: object {} at offset {:#x} (length {:?}) could not be decoded, skipped {} bytes",
                failure.object_type_id, failure.offset, failure.length, failure.skipped
            );
        }
    })
    .and_then(|_| Ok(writer.flush()?));

    if let Err(error) = result {
        // A half written DXF is of no use.
        let _ = std::fs::remove_file(&options.output);
        exit(format!("{input}: {error}"), 1);
    }
}
