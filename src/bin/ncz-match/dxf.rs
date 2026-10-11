//! A raw ASCII DXF reader that keeps every group code, so nothing that NetCAD writes is lost.

use encoding_rs::WINDOWS_1254;

pub type Group = (i32, String);

#[derive(Debug, Default)]
pub struct Entity {
    /// Entity type, e.g. `LINE`.
    pub kind: String,
    /// Name of the block that holds the entity, `None` in the model space.
    pub block: Option<String>,
    pub groups: Vec<Group>,
    /// Entities that follow and belong to this one, such as `VERTEX` and `ATTRIB`.
    pub children: Vec<Entity>,
}

#[derive(Debug)]
pub struct TableEntry {
    /// Table name, e.g. `LAYER`.
    pub table: String,
    pub groups: Vec<Group>,
}

#[derive(Debug, Default)]
pub struct Dxf {
    /// Header variables with their groups, e.g. `$LTSCALE`.
    pub header: Vec<(String, Vec<Group>)>,
    pub tables: Vec<TableEntry>,
    pub entities: Vec<Entity>,
}

impl Entity {
    /// The first value of a group code.
    pub fn get(&self, code: i32) -> Option<&str> {
        self.groups
            .iter()
            .find(|(c, _)| *c == code)
            .map(|(_, v)| v.as_str())
    }

    /// Every 2D point of the entity and its children, in the order they are written.
    pub fn points(&self) -> Vec<(f64, f64)> {
        let mut points = vec![];
        collect_points(&self.groups, &mut points);
        for child in &self.children {
            collect_points(&child.groups, &mut points);
        }

        points
    }

    /// The text content, with MTEXT chunks joined.
    pub fn text(&self) -> Option<String> {
        let mut text: String = self
            .groups
            .iter()
            .filter(|(c, _)| *c == 3)
            .map(|(_, v)| v.as_str())
            .collect();
        text.push_str(self.get(1)?);

        Some(text)
    }
}

/// Pairs each X (codes 10 to 18) with the Y that follows it (codes 20 to 28).
fn collect_points(groups: &[Group], points: &mut Vec<(f64, f64)>) {
    let mut pending: Option<(i32, f64)> = None;
    for (code, value) in groups {
        if (10..=18).contains(code) {
            pending = value.trim().parse().ok().map(|x| (*code, x));
        } else if (20..=28).contains(code)
            && let Some((x_code, x)) = pending.take()
            && x_code + 10 == *code
            && let Ok(y) = value.trim().parse()
        {
            points.push((x, y));
        }
    }
}

/// Characters 0x80 to 0xFF of DOS code page 857, which NetCAD 5 uses for DXF R12 files.
#[rustfmt::skip]
const CP857_HIGH: [char; 128] = [
    '\u{00c7}', '\u{00fc}', '\u{00e9}', '\u{00e2}', '\u{00e4}', '\u{00e0}', '\u{00e5}', '\u{00e7}', '\u{00ea}', '\u{00eb}', '\u{00e8}', '\u{00ef}', '\u{00ee}', '\u{0131}', '\u{00c4}', '\u{00c5}',
    '\u{00c9}', '\u{00e6}', '\u{00c6}', '\u{00f4}', '\u{00f6}', '\u{00f2}', '\u{00fb}', '\u{00f9}', '\u{0130}', '\u{00d6}', '\u{00dc}', '\u{00f8}', '\u{00a3}', '\u{00d8}', '\u{015e}', '\u{015f}',
    '\u{00e1}', '\u{00ed}', '\u{00f3}', '\u{00fa}', '\u{00f1}', '\u{00d1}', '\u{011e}', '\u{011f}', '\u{00bf}', '\u{00ae}', '\u{00ac}', '\u{00bd}', '\u{00bc}', '\u{00a1}', '\u{00ab}', '\u{00bb}',
    '\u{2591}', '\u{2592}', '\u{2593}', '\u{2502}', '\u{2524}', '\u{00c1}', '\u{00c2}', '\u{00c0}', '\u{00a9}', '\u{2563}', '\u{2551}', '\u{2557}', '\u{255d}', '\u{00a2}', '\u{00a5}', '\u{2510}',
    '\u{2514}', '\u{2534}', '\u{252c}', '\u{251c}', '\u{2500}', '\u{253c}', '\u{00e3}', '\u{00c3}', '\u{255a}', '\u{2554}', '\u{2569}', '\u{2566}', '\u{2560}', '\u{2550}', '\u{256c}', '\u{00a4}',
    '\u{00ba}', '\u{00aa}', '\u{00ca}', '\u{00cb}', '\u{00c8}', '\u{fffd}', '\u{00cd}', '\u{00ce}', '\u{00cf}', '\u{2518}', '\u{250c}', '\u{2588}', '\u{2584}', '\u{00a6}', '\u{00cc}', '\u{2580}',
    '\u{00d3}', '\u{00df}', '\u{00d4}', '\u{00d2}', '\u{00f5}', '\u{00d5}', '\u{00b5}', '\u{fffd}', '\u{00d7}', '\u{00da}', '\u{00db}', '\u{00d9}', '\u{00ec}', '\u{00ff}', '\u{00af}', '\u{00b4}',
    '\u{00ad}', '\u{00b1}', '\u{fffd}', '\u{00be}', '\u{00b6}', '\u{00a7}', '\u{00f7}', '\u{00b8}', '\u{00b0}', '\u{00a8}', '\u{00b7}', '\u{00b9}', '\u{00b3}', '\u{00b2}', '\u{25a0}', '\u{00a0}',
];

/// Whether the text is in code page 857 rather than Windows-1254. Uses `$DWGCODEPAGE` when the header has it,
/// otherwise the Turkish letters of code page 857 (0x80 to 0xA7), which are mostly unused in Windows-1254.
fn is_cp857(bytes: &[u8]) -> bool {
    let header = &bytes[..bytes.len().min(64 * 1024)];
    if let Some(start) = header.windows(12).position(|w| w == b"$DWGCODEPAGE") {
        let rest =
            String::from_utf8_lossy(&header[start..header.len().min(start + 64)]).to_uppercase();
        return rest.contains("DOS857") || rest.contains("857") && !rest.contains("1254");
    }

    let high = bytes.iter().filter(|&&b| b >= 0x80).count();
    let oem = bytes
        .iter()
        .filter(|&&b| (0x80..=0xa7).contains(&b))
        .count();
    high > 0 && oem * 4 >= high * 3
}

fn decode(bytes: &[u8]) -> String {
    if let Ok(text) = std::str::from_utf8(bytes) {
        return text.to_owned();
    }

    if is_cp857(bytes) {
        bytes
            .iter()
            .map(|&b| {
                if b < 0x80 {
                    char::from(b)
                } else {
                    CP857_HIGH[usize::from(b - 0x80)]
                }
            })
            .collect()
    } else {
        WINDOWS_1254.decode(bytes).0.into_owned()
    }
}

pub fn parse(bytes: &[u8]) -> Result<Dxf, String> {
    if bytes.starts_with(b"AutoCAD Binary DXF") {
        return Err("binary DXF is not supported, export it as ASCII".to_owned());
    }

    let text = decode(bytes);
    let mut lines = text.lines();
    let mut groups = vec![];
    while let (Some(code), Some(value)) = (lines.next(), lines.next()) {
        let code = code
            .trim()
            .parse()
            .map_err(|_| format!("bad group code {code:?} at pair {}", groups.len()))?;
        groups.push((code, value.trim_end_matches('\r').to_owned()));
    }

    let mut dxf = Dxf::default();
    let mut section = String::new();
    let mut block: Option<String> = None;
    let mut index = 0;
    while index < groups.len() {
        let (code, value) = &groups[index];
        index += 1;
        if *code != 0 && *code != 9 {
            continue;
        }

        // The groups up to the next 0 (or 9 in the header).
        let end = groups[index..]
            .iter()
            .position(|(c, _)| *c == 0 || (section == "HEADER" && *c == 9))
            .map_or(groups.len(), |p| index + p);
        let body = groups[index..end].to_vec();
        index = end;

        match (section.as_str(), *code, value.as_str()) {
            (_, 0, "SECTION") => {
                section = body
                    .iter()
                    .find(|(c, _)| *c == 2)
                    .map(|(_, v)| v.clone())
                    .unwrap_or_default();
            }
            (_, 0, "ENDSEC") => section.clear(),
            ("HEADER", 9, name) => dxf.header.push((name.to_owned(), body)),
            ("TABLES", 0, "TABLE" | "ENDTAB") => {}
            ("TABLES", 0, table) => dxf.tables.push(TableEntry {
                table: table.to_owned(),
                groups: body,
            }),
            ("BLOCKS", 0, "BLOCK") => {
                block = body.iter().find(|(c, _)| *c == 2).map(|(_, v)| v.clone());
            }
            ("BLOCKS", 0, "ENDBLK") => block = None,
            ("BLOCKS" | "ENTITIES", 0, "SEQEND") => {}
            ("BLOCKS" | "ENTITIES", 0, kind @ ("VERTEX" | "ATTRIB")) => {
                let child = Entity {
                    kind: kind.to_owned(),
                    block: block.clone(),
                    groups: body,
                    children: vec![],
                };
                match dxf.entities.last_mut() {
                    Some(parent) => parent.children.push(child),
                    None => dxf.entities.push(child),
                }
            }
            ("BLOCKS" | "ENTITIES", 0, kind) => dxf.entities.push(Entity {
                kind: kind.to_owned(),
                block: if section == "BLOCKS" {
                    block.clone()
                } else {
                    None
                },
                groups: body,
                children: vec![],
            }),
            _ => {}
        }
    }

    Ok(dxf)
}
