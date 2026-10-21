//! Converts every NCZ file of a directory, checks the DXF output and compares the results with a baseline.
//!
//! Usage: `corpus-check [--all] [--update] [--jobs N] [--baseline path] [dir]`
//!
//! For each file it records whether the file was rejected, how many objects could not be decoded, the geometries by
//! kind, the DXF entities by type and the geometries left out. The DXF output is checked for handles that are zero,
//! repeated or not below `$HANDSEED`, for extended data that follows a `SEQEND`, and for loading back with the dxf
//! crate. The results are compared with the baseline, `target/corpus-baseline.json` by default, which `--update`
//! replaces. The baseline is kept out of the repository, since it names the files of the corpus.
//!
//! Files over 60 MB are left out unless `--all` is given. The exit code is 1 when a check fails or a result differs
//! from the baseline, unless `--update` is given.

use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Files above this size take minutes and gigabytes each, so they are only checked with `--all`.
const LARGE_FILE: u64 = 60_000_000;

const USAGE: &str = "usage: corpus-check [--all] [--update] [--jobs N] [--baseline path] [dir]";

struct Options {
    dir: PathBuf,
    baseline: PathBuf,
    all: bool,
    update: bool,
    jobs: usize,
}

#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
struct Record {
    /// `ok`, or why the file could not be converted.
    status: String,
    unreadable: usize,
    geometries: BTreeMap<String, usize>,
    entities: BTreeMap<String, usize>,
    skipped: BTreeMap<String, usize>,
    /// Checks of the DXF output that failed.
    problems: Vec<String>,
}

fn parse_options() -> Result<Options, String> {
    let mut options = Options {
        dir: PathBuf::from("ncz"),
        baseline: PathBuf::from("target/corpus-baseline.json"),
        all: false,
        update: false,
        jobs: 4,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--all" => options.all = true,
            "--update" => options.update = true,
            "--jobs" => {
                options.jobs = args
                    .next()
                    .and_then(|n| n.parse().ok())
                    .filter(|&n| n > 0)
                    .ok_or(USAGE)?;
            }
            "--baseline" => options.baseline = args.next().ok_or(USAGE)?.into(),
            _ if arg.starts_with("--") => return Err(USAGE.into()),
            _ => options.dir = arg.into(),
        }
    }
    Ok(options)
}

fn is_ncz(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("ncz"))
}

/// Checks the handles and the placement of extended data in an ASCII DXF file.
fn check_dxf_text(dxf: &[u8], problems: &mut Vec<String>) {
    let text = String::from_utf8_lossy(dxf);
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    let pairs: Vec<(&str, &str)> = lines
        .chunks_exact(2)
        .map(|pair| (pair[0], pair[1]))
        .collect();

    let mut seed = None;
    let mut handles = vec![];
    let mut entity = "";
    let mut x_data_after_seqend = 0;
    for (index, &(code, value)) in pairs.iter().enumerate() {
        match code {
            "0" => entity = value,
            "5" | "105" if index > 0 && pairs[index - 1] == ("9", "$HANDSEED") => {
                seed = u64::from_str_radix(value, 16).ok();
            }
            "5" | "105" => match u64::from_str_radix(value, 16) {
                Ok(handle) => handles.push(handle),
                Err(_) => problems.push(format!("handle {value:?} is not hexadecimal")),
            },
            "1001" if entity == "SEQEND" => x_data_after_seqend += 1,
            _ => {}
        }
    }

    let Some(seed) = seed else {
        problems.push("no $HANDSEED".into());
        return;
    };
    let zero = handles.iter().filter(|&&handle| handle == 0).count();
    let not_below_seed = handles.iter().filter(|&&handle| handle >= seed).count();
    let unique = handles.iter().collect::<HashSet<_>>().len();
    for (count, what) in [
        (zero, "handles are zero"),
        (not_below_seed, "handles are not below $HANDSEED"),
        (handles.len() - unique, "handles are repeated"),
        (x_data_after_seqend, "SEQENDs have extended data"),
    ] {
        if count > 0 {
            problems.push(format!("{count} {what}"));
        }
    }
}

fn check_file(path: &Path) -> Record {
    let to_strings =
        |map: BTreeMap<&str, usize>| map.into_iter().map(|(k, v)| (k.to_owned(), v)).collect();
    let mut record = Record {
        status: "ok".into(),
        unreadable: 0,
        geometries: BTreeMap::new(),
        entities: BTreeMap::new(),
        skipped: BTreeMap::new(),
        problems: vec![],
    };

    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            record.status = error.to_string();
            return record;
        }
    };
    let mut dxf = vec![];
    let summary = match nczrs::summary::convert(&bytes, &mut dxf) {
        Ok(summary) => summary,
        Err(error) => {
            record.status = error.to_string();
            return record;
        }
    };
    drop(bytes);

    record.unreadable = summary.file.unreadable;
    record.geometries = to_strings(summary.file.geometries);
    record.entities = to_strings(summary.dxf.entities);
    record.skipped = to_strings(summary.dxf.skipped);

    check_dxf_text(&dxf, &mut record.problems);
    if let Err(error) = dxf::Drawing::load(&mut dxf.as_slice()) {
        record.problems.push(format!("does not load back: {error}"));
    }
    record
}

/// The fields of two JSON values that differ, as `path: old → new` lines.
fn differences(path: &str, old: &Value, new: &Value, out: &mut Vec<String>) {
    match (old, new) {
        (Value::Object(old), Value::Object(new)) => {
            let keys: std::collections::BTreeSet<&String> = old.keys().chain(new.keys()).collect();
            for key in keys {
                let missing = Value::Null;
                let child = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                differences(
                    &child,
                    old.get(key).unwrap_or(&missing),
                    new.get(key).unwrap_or(&missing),
                    out,
                );
            }
        }
        _ if old != new => out.push(format!("{path}: {old} → {new}")),
        _ => {}
    }
}

fn main() {
    let options = match parse_options() {
        Ok(options) => options,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    };

    let mut files: Vec<(PathBuf, u64)> = match std::fs::read_dir(&options.dir) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| is_ncz(path))
            .filter_map(|path| Some((path.clone(), std::fs::metadata(&path).ok()?.len())))
            .collect(),
        Err(error) => {
            eprintln!("{}: {error}", options.dir.display());
            std::process::exit(2);
        }
    };
    let large = files.iter().filter(|&&(_, size)| size > LARGE_FILE).count();
    if !options.all {
        files.retain(|&(_, size)| size <= LARGE_FILE);
    }
    // Largest first, so the slowest files do not start last.
    files.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    let next = AtomicUsize::new(0);
    let results = Mutex::new(BTreeMap::new());
    std::thread::scope(|scope| {
        for _ in 0..options.jobs {
            scope.spawn(|| {
                while let Some((path, _)) = files.get(next.fetch_add(1, Ordering::Relaxed)) {
                    let name = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    let record = check_file(path);
                    results.lock().unwrap().insert(name, record);
                }
            });
        }
    });
    let results = results.into_inner().unwrap();

    let mut failed = false;
    let ok = results
        .values()
        .filter(|record| record.status == "ok")
        .count();
    let with_unreadable = results
        .values()
        .filter(|record| record.unreadable > 0)
        .count();
    println!(
        "{} files ({} over 60 MB {}): {ok} converted, {} not, {with_unreadable} with unreadable objects",
        results.len(),
        large,
        if options.all {
            "included"
        } else {
            "left out, use --all"
        },
        results.len() - ok,
    );
    for (name, record) in &results {
        if record.status != "ok" {
            println!("  not converted: {name}: {}", record.status);
        }
        for problem in &record.problems {
            println!("  CHECK FAILED: {name}: {problem}");
            failed = true;
        }
    }

    let new = serde_json::to_value(&results).expect("records serialize");
    let baseline: Option<Map<String, Value>> = std::fs::read_to_string(&options.baseline)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok());
    match &baseline {
        Some(old) => {
            let mut changes = vec![];
            // Large files left out of this run are not compared.
            let old = Value::Object(
                old.iter()
                    .filter(|(name, _)| results.contains_key(*name) || options.all)
                    .map(|(name, record)| (name.clone(), record.clone()))
                    .collect(),
            );
            differences("", &old, &new, &mut changes);
            if changes.is_empty() {
                println!("same as the baseline {}", options.baseline.display());
            } else {
                println!(
                    "{} differences from the baseline {}:",
                    changes.len(),
                    options.baseline.display()
                );
                for change in &changes {
                    println!("  {change}");
                }
                failed |= !options.update;
            }
        }
        None if !options.update => {
            println!(
                "no baseline at {}, create one with --update",
                options.baseline.display()
            );
        }
        None => {}
    }

    if options.update {
        // Keeps the large files of an earlier run with `--all` when this run left them out.
        let mut merged = baseline.unwrap_or_default();
        if options.all {
            merged.clear();
        }
        if let Value::Object(new) = new {
            merged.extend(new);
        }
        let text = serde_json::to_string(&merged).expect("records serialize");
        match std::fs::write(&options.baseline, text) {
            Ok(()) => println!("wrote the baseline {}", options.baseline.display()),
            Err(error) => {
                eprintln!("{}: {error}", options.baseline.display());
                failed = true;
            }
        }
    }

    std::process::exit(i32::from(failed));
}
