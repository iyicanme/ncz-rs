//! The shapes of symbols, embedded from the files of `symbols/` (see `symbols/README.md`).

include!(concat!(env!("OUT_DIR"), "/symbols.rs"));

/// A shape of a symbol, in drawing units for scale 1, relative to the insertion point, `x` to the east.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    Line { start: (f64, f64), end: (f64, f64) },
    Circle { center: (f64, f64), radius: f64 },
}

/// Parses a symbol file.
pub fn parse(text: &str) -> Result<Vec<Shape>, String> {
    let mut shapes = vec![];
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let error = |message: &str| format!("line {}: {message}: {line}", index + 1);
        let mut words = line.split_whitespace();
        let kind = words.next().unwrap_or_default();
        let numbers = words
            .map(|word| word.parse::<f64>().ok().filter(|value| value.is_finite()))
            .collect::<Option<Vec<f64>>>()
            .ok_or_else(|| error("not a number"))?;

        let shape = match (kind, numbers.as_slice()) {
            ("line", &[x1, y1, x2, y2]) => Shape::Line {
                start: (x1, y1),
                end: (x2, y2),
            },
            ("circle", &[x, y, radius]) if radius > 0.0 => Shape::Circle {
                center: (x, y),
                radius,
            },
            ("line" | "circle", _) => return Err(error("wrong number of values")),
            _ => return Err(error("unknown shape")),
        };
        shapes.push(shape);
    }
    Ok(shapes)
}

/// The library name of a library file name as NCZ files list it, e.g. `F2` for `F2.NCS` or `C:\NETCAD\F2.NCS`.
pub fn library_name(file_name: &str) -> String {
    let name = file_name.rsplit(['\\', '/']).next().unwrap_or(file_name);
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    stem.trim().to_uppercase()
}

/// The shapes of a symbol of a library, `None` when the symbol is not embedded or has no shapes. The embedded files
/// are parsed once, on the first call.
pub fn find(library: &str, symbol: u16) -> Option<&'static [Shape]> {
    static PARSED: std::sync::OnceLock<Vec<Vec<Shape>>> = std::sync::OnceLock::new();
    let parsed = PARSED.get_or_init(|| {
        EMBEDDED
            .iter()
            .map(|&(_, _, text)| parse(text).unwrap_or_default())
            .collect()
    });

    let index = EMBEDDED
        .binary_search_by(|&(name, number, _)| (name, number).cmp(&(library, symbol)))
        .ok()?;
    Some(parsed[index].as_slice()).filter(|shapes| !shapes.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_symbols_parse() {
        assert!(!EMBEDDED.is_empty());
        for &(library, number, text) in EMBEDDED {
            let shapes =
                parse(text).unwrap_or_else(|error| panic!("{library}/{number}.txt: {error}"));
            assert!(!shapes.is_empty(), "{library}/{number}.txt has no shapes");
        }
    }

    #[test]
    fn finds_symbols() {
        // The boundary stone square of F2.
        assert_eq!(
            find("F2", 168).unwrap()[0],
            Shape::Line {
                start: (-2.0, 2.0),
                end: (-2.0, -2.0)
            }
        );
        assert!(find("F2", 300).is_none());
        assert!(find("NOPE", 168).is_none());
    }

    #[test]
    fn parses_shapes() {
        let shapes = parse("# comment\n\ncircle 0 0 0.5\nline -1 0 1 0\n").unwrap();
        assert_eq!(
            shapes,
            [
                Shape::Circle {
                    center: (0.0, 0.0),
                    radius: 0.5
                },
                Shape::Line {
                    start: (-1.0, 0.0),
                    end: (1.0, 0.0)
                }
            ]
        );
        assert!(parse("line 1 2 3").is_err());
        assert!(parse("arc 1 2 3").is_err());
        assert!(parse("circle 0 0 -1").is_err());
    }

    #[test]
    fn library_names() {
        assert_eq!(library_name("F2.NCS"), "F2");
        assert_eq!(library_name("C:\\NETCAD\\plan.ncs"), "PLAN");
        assert_eq!(library_name("3D"), "3D");
    }
}
