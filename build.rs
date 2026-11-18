//! Embeds the symbol files of `symbols/`, see `symbols/README.md`. Writes `symbols.rs` into `OUT_DIR`, a table of
//! `(library, number, contents)` sorted by library and number.

use std::fmt::Write as _;
use std::path::Path;

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("symbols");
    println!("cargo::rerun-if-changed={}", root.display());

    let mut symbols = vec![];
    for library in std::fs::read_dir(&root).expect("symbols/ is readable") {
        let library = library.expect("symbols/ is readable").path();
        if !library.is_dir() {
            continue;
        }
        let name = library
            .file_name()
            .unwrap()
            .to_str()
            .expect("library names are UTF-8")
            .to_owned();
        assert_eq!(
            name,
            name.to_uppercase(),
            "library directory {name} must be upper case"
        );

        for file in std::fs::read_dir(&library).expect("library directories are readable") {
            let path = file.expect("library directories are readable").path();
            if path.extension().is_none_or(|extension| extension != "txt") {
                continue;
            }
            let number: u16 = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .and_then(|stem| stem.parse().ok())
                .unwrap_or_else(|| panic!("{} is not named after a symbol number", path.display()));
            symbols.push((name.clone(), number, path));
        }
    }
    symbols.sort();

    let mut code = String::from("pub(crate) static EMBEDDED: &[(&str, u16, &str)] = &[\n");
    for (library, number, path) in symbols {
        writeln!(
            code,
            "    ({library:?}, {number}, include_str!({:?})),",
            path.display().to_string()
        )
        .unwrap();
    }
    code.push_str("];\n");

    let out = Path::new(&std::env::var("OUT_DIR").unwrap()).join("symbols.rs");
    std::fs::write(out, code).unwrap();
}
