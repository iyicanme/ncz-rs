//! Pairs an NCZ file with the DXF file NetCAD exported from it, and reports which NCZ fields predict which
//! DXF group codes. Fields of unknown meaning that predict a DXF code reveal what they mean.
//!
//! Usage: `ncz-match [--tolerance T] [--dump pairs.jsonl] [--max N] <file.ncz | file.ncz file.dxf | dir>...`
//!
//! An NCZ file given alone is paired with the DXF file of the same name next to it. A directory pairs every
//! NCZ file in it that has such a DXF file. Records from all pairs are pooled before correlating, so many
//! small test files give stronger results than one large file.

mod correlate;
mod dxf;
mod features;
mod matching;

use correlate::Record;
use features::{Features, entity_features, insert_groups, ncz_items};
use nczrs::object::Object;
use nczrs::parse_objects;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Records per domain that are correlated. Larger domains are sampled evenly.
const MAX_RECORDS_PER_DOMAIN: usize = 20_000;

struct Options {
    tolerance: f64,
    dump: Option<PathBuf>,
    max_per_code: usize,
    pairs: Vec<(PathBuf, PathBuf)>,
}

fn sibling_dxf(ncz: &Path) -> Option<PathBuf> {
    ["dxf", "DXF", "Dxf"]
        .iter()
        .map(|extension| ncz.with_extension(extension))
        .find(|path| path.is_file())
}

fn is_ncz(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("ncz"))
}

fn parse_options() -> Result<Options, String> {
    let mut options = Options {
        tolerance: 0.005,
        dump: None,
        max_per_code: 5,
        pairs: vec![],
    };

    let mut args = std::env::args().skip(1).peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--tolerance" => {
                options.tolerance = args
                    .next()
                    .and_then(|t| t.parse().ok())
                    .ok_or("--tolerance needs a number")?;
            }
            "--dump" => options.dump = Some(args.next().ok_or("--dump needs a path")?.into()),
            "--max" => {
                options.max_per_code = args
                    .next()
                    .and_then(|t| t.parse().ok())
                    .ok_or("--max needs a number")?
            }
            _ => {
                let path = PathBuf::from(arg);
                if path.is_dir() {
                    let mut entries: Vec<PathBuf> = std::fs::read_dir(&path)
                        .map_err(|e| format!("{}: {e}", path.display()))?
                        .filter_map(|e| e.ok().map(|e| e.path()))
                        .filter(|p| is_ncz(p))
                        .collect();
                    entries.sort();
                    for ncz in entries {
                        if let Some(dxf) = sibling_dxf(&ncz) {
                            options.pairs.push((ncz, dxf));
                        }
                    }
                } else if let Some(next) =
                    args.peek().filter(|n| n.to_lowercase().ends_with(".dxf"))
                {
                    let dxf = PathBuf::from(next);
                    args.next();
                    options.pairs.push((path, dxf));
                } else {
                    let dxf = sibling_dxf(&path)
                        .ok_or_else(|| format!("no DXF file next to {}", path.display()))?;
                    options.pairs.push((path, dxf));
                }
            }
        }
    }

    if options.pairs.is_empty() {
        return Err("usage: ncz-match [--tolerance T] [--dump pairs.jsonl] [--max N] <file.ncz [file.dxf] | dir>...".into());
    }

    Ok(options)
}

/// Layers and line types are matched by name.
fn table_records(objects: &[Object], dxf: &dxf::Dxf, records: &mut Vec<Record>) {
    for object in objects {
        let (table, entries): (&str, Vec<(String, Features)>) = match object {
            Object::Layers { layers, .. } => (
                "LAYER",
                layers
                    .iter()
                    .enumerate()
                    .map(|(i, layer)| (layer.name.clone(), serde_features(layer, i)))
                    .collect(),
            ),
            Object::LineTypes { types } => (
                "LTYPE",
                types
                    .iter()
                    .enumerate()
                    .map(|(i, line_type)| (line_type.name.clone(), serde_features(line_type, i)))
                    .collect(),
            ),
            _ => continue,
        };

        for (name, ncz) in entries {
            let entry = dxf.tables.iter().find(|e| {
                e.table == table
                    && e.groups.iter().any(|(c, v)| {
                        let v = features::decode_unicode_escapes(v);
                        *c == 2
                            && (v.to_lowercase() == name.to_lowercase()
                                || v == ascii_table_name(&name))
                    })
            });
            if let Some(entry) = entry {
                let mut dxf_features = Features::new();
                insert_groups(&mut dxf_features, "", &entry.groups);
                // A negative layer color means the layer is off.
                if let Some(color) = dxf_features.get("62").and_then(|c| c.parse::<f64>().ok()) {
                    dxf_features.insert("62.off".to_owned(), u8::from(color < 0.0).to_string());
                }
                records.push(Record {
                    domain: table.to_owned(),
                    ncz,
                    dxf: dxf_features,
                });
            }
        }
    }
}

/// The name NetCAD 5 writes for a table entry in a DXF R12 file, e.g. `@EĞRİ` becomes `XEGRI`.
fn ascii_table_name(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            'Ğ' | 'ğ' => 'G',
            'ı' | 'İ' => 'I',
            'Ş' | 'ş' => 'S',
            'Ç' | 'ç' => 'C',
            'Ö' | 'ö' => 'O',
            'Ü' | 'ü' => 'U',
            '@' => 'X',
            c if c.is_ascii_alphanumeric() || c == '_' || c == '-' => c.to_ascii_uppercase(),
            _ => '-',
        })
        .collect()
}

fn serde_features(value: &impl serde::Serialize, index: usize) -> Features {
    let mut features = Features::new();
    features.insert("index".to_owned(), index.to_string());
    if let Ok(serde_json::Value::Object(fields)) = serde_json::to_value(value) {
        for (key, value) in fields {
            match value {
                serde_json::Value::Number(n) => {
                    let number = n.as_f64().unwrap_or(f64::NAN);
                    features.insert(key.clone(), features::format_number(number));
                    if key == "flags" {
                        for bit in 0..32 {
                            features.insert(
                                format!("flags.bit{bit}"),
                                ((number as u64 >> bit) & 1).to_string(),
                            );
                        }
                    }
                }
                serde_json::Value::String(s) => {
                    features.insert(key, s);
                }
                serde_json::Value::Array(a) => {
                    for (i, v) in a.iter().enumerate() {
                        let value = v
                            .as_f64()
                            .map_or_else(|| v.to_string(), features::format_number);
                        features.insert(format!("{key}[{i}]"), value);
                    }
                }
                _ => {}
            }
        }
    }

    features
}

/// One record per file, so that file level fields can be correlated with DXF header variables across files.
fn file_record(objects: &[Object], dxf: &dxf::Dxf) -> Record {
    let mut ncz = Features::new();
    for object in objects {
        match object {
            Object::FileHeader { bytes } => {
                for (i, chunk) in bytes.chunks_exact(4).enumerate() {
                    let value = u32::from_le_bytes(chunk.try_into().unwrap());
                    ncz.insert(format!("file_header.u32[{}]", i * 4), value.to_string());
                }
            }
            Object::Layers {
                unknown1,
                unknown2,
                unknown3,
                ..
            } => {
                for (i, b) in unknown1.iter().enumerate() {
                    ncz.insert(format!("layers.unknown1[{i}]"), b.to_string());
                }
                ncz.insert("layers.unknown2".to_owned(), unknown2.to_string());
                ncz.insert("layers.unknown3".to_owned(), unknown3.to_string());
            }
            Object::Scale { scale } => {
                ncz.insert("scale".to_owned(), features::format_number(*scale));
            }
            _ => {}
        }
    }

    let mut dxf_features = Features::new();
    for (name, groups) in &dxf.header {
        insert_groups(&mut dxf_features, &format!("{name}."), groups);
    }

    Record {
        domain: "FILE".to_owned(),
        ncz,
        dxf: dxf_features,
    }
}

fn kind_counts<'a>(kinds: impl Iterator<Item = &'a str>) -> String {
    let mut counts = std::collections::BTreeMap::new();
    for kind in kinds {
        *counts.entry(kind).or_insert(0) += 1;
    }

    counts
        .iter()
        .map(|(k, c)| format!("{k} {c}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn run(options: &Options) -> Result<(), String> {
    let mut records = vec![];
    let mut dump = match &options.dump {
        Some(path) => Some(std::io::BufWriter::new(
            std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?,
        )),
        None => None,
    };

    for (ncz_path, dxf_path) in &options.pairs {
        let ncz_bytes =
            std::fs::read(ncz_path).map_err(|e| format!("{}: {e}", ncz_path.display()))?;
        let dxf_bytes =
            std::fs::read(dxf_path).map_err(|e| format!("{}: {e}", dxf_path.display()))?;
        let objects = parse_objects(&ncz_bytes);
        let dxf = dxf::parse(&dxf_bytes).map_err(|e| format!("{}: {e}", dxf_path.display()))?;

        let items = ncz_items(&objects);
        let matching = matching::match_items(&items, &dxf.entities, options.tolerance);

        println!("{} ↔ {}", ncz_path.display(), dxf_path.display());
        println!(
            "    {} geometries, {} entities, {} pairs ({} secondary){}",
            items.len(),
            dxf.entities.len(),
            matching.matches.len(),
            matching.matches.iter().filter(|m| m.secondary).count(),
            if matching.swapped {
                ", X and Y swapped"
            } else {
                ""
            }
        );
        if !matching.unmatched_items.is_empty() {
            let kinds = matching.unmatched_items.iter().map(|&i| items[i].kind);
            println!("    unmatched geometries: {}", kind_counts(kinds));
        }
        if !matching.unmatched_entities.is_empty() {
            let kinds = matching
                .unmatched_entities
                .iter()
                .map(|&e| dxf.entities[e].kind.as_str());
            println!("    unmatched entities: {}", kind_counts(kinds));
        }

        for m in &matching.matches {
            let item = &items[m.item];
            let entity = &dxf.entities[m.entity];
            let record = Record {
                domain: format!("{} -> {}", item.kind, entity.kind),
                ncz: item.features.clone(),
                dxf: entity_features(entity),
            };

            if let Some(dump) = &mut dump {
                let line = serde_json::json!({
                    "file": ncz_path.display().to_string(),
                    "geometry": item.index,
                    "score": m.score,
                    "secondary": m.secondary,
                    "domain": record.domain,
                    "ncz": record.ncz,
                    "dxf": entity.groups,
                    "children": entity.children.iter().map(|c| &c.groups).collect::<Vec<_>>(),
                });
                writeln!(dump, "{line}").map_err(|e| e.to_string())?;
            }
            records.push(record);
        }

        table_records(&objects, &dxf, &mut records);
        records.push(file_record(&objects, &dxf));
    }

    // Even sampling keeps large drawings fast while keeping every domain.
    let mut by_domain: std::collections::BTreeMap<String, Vec<Record>> =
        std::collections::BTreeMap::new();
    for record in records {
        by_domain
            .entry(record.domain.clone())
            .or_default()
            .push(record);
    }
    let records: Vec<Record> = by_domain
        .into_values()
        .flat_map(|domain| {
            let step = domain.len().div_ceil(MAX_RECORDS_PER_DOMAIN);
            domain.into_iter().step_by(step.max(1))
        })
        .collect();

    println!("\nFields marked * have an unknown meaning.");
    print!("{}", correlate::report(&records, options.max_per_code));

    Ok(())
}

fn main() {
    let result = parse_options().and_then(|options| run(&options));
    if let Err(error) = result {
        eprintln!("ncz-match: {error}");
        std::process::exit(1);
    }
}
