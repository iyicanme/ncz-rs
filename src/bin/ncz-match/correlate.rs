//! Finds NCZ fields whose value predicts the value of a DXF group code.

use crate::features::{Features, format_number};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

/// A geometry and its DXF counterpart, or a table entry and its DXF counterpart.
pub struct Record {
    /// What is compared, e.g. `Circle -> CIRCLE` or `LAYER`.
    pub domain: String,
    pub ncz: Features,
    pub dxf: Features,
}

#[derive(PartialEq, PartialOrd)]
enum Relation {
    /// Every NCZ value maps to one DXF value, but most values are seen once, so this may be chance.
    Weak,
    /// Most records follow a mapping, the others are listed as exceptions.
    Mostly,
    /// Every NCZ value maps to one DXF value, and the values repeat enough to trust it.
    Determines,
    /// As above, and the mapping is one-to-one.
    OneToOne,
    /// `dxf = a * ncz + b` holds for every record.
    Linear { a: f64, b: f64 },
    /// The values are the same text in every record.
    Equal,
}

struct Finding {
    ncz_key: String,
    relation: Relation,
    /// Records that have both keys.
    support: usize,
    /// NCZ value to DXF value, with how many records show it.
    mapping: BTreeMap<String, (String, usize)>,
    /// Records that do not follow the mapping, as NCZ value, DXF value and count.
    exceptions: Vec<(String, String, usize)>,
}

/// Share of records that must follow a mapping for [`Relation::Mostly`].
const MOSTLY: f64 = 0.9;

/// Fields whose meaning is already known. They are listed after the unknown ones.
fn is_known(key: &str) -> bool {
    let unknown = [
        "unknown",
        "attributes",
        "ctx.style",
        "ctx.hatch",
        "flags",
        "line_type2",
        "snap",
        "rotation",
    ];
    !unknown.iter().any(|u| key.contains(u))
}

fn distinct<'a>(records: &[&'a Record], key: &str, ncz: bool) -> BTreeSet<&'a str> {
    records
        .iter()
        .filter_map(|r| if ncz { r.ncz.get(key) } else { r.dxf.get(key) })
        .map(String::as_str)
        .collect()
}

fn linear_fit(pairs: &[(f64, f64)]) -> Option<(f64, f64)> {
    let (x0, y0) = pairs[0];
    let &(x1, y1) = pairs.iter().find(|(x, _)| (x - x0).abs() > 1e-9)?;
    let a = (y1 - y0) / (x1 - x0);
    let b = y0 - a * x0;
    let fits = pairs
        .iter()
        .all(|(x, y)| (a * x + b - y).abs() <= 1e-4 * y.abs().max(1.0));

    fits.then_some((a, b))
}

fn relate(records: &[&Record], ncz_key: &str, dxf_key: &str) -> Option<Finding> {
    let mut mapping: BTreeMap<&str, BTreeMap<&str, usize>> = BTreeMap::new();
    let mut numbers = vec![];
    for record in records {
        let (Some(n), Some(d)) = (record.ncz.get(ncz_key), record.dxf.get(dxf_key)) else {
            continue;
        };
        *mapping.entry(n).or_default().entry(d).or_default() += 1;
        if let (Ok(x), Ok(y)) = (n.parse::<f64>(), d.parse::<f64>()) {
            numbers.push((x, y));
        }
    }

    let support: usize = mapping.values().flat_map(|m| m.values()).sum();
    let ncz_values = mapping.len();
    let dxf_values: BTreeSet<&str> = mapping.values().flat_map(|m| m.keys().copied()).collect();
    if ncz_values < 2 || dxf_values.len() < 2 {
        return None;
    }

    let mut exceptions = vec![];
    for (n, m) in &mapping {
        let (majority, _) = m.iter().max_by_key(|(_, c)| **c).unwrap();
        for (d, c) in m {
            if d != majority {
                exceptions.push((n.to_string(), d.to_string(), *c));
            }
        }
    }
    let agreeing = support - exceptions.iter().map(|(_, _, c)| c).sum::<usize>();

    let flat = mapping
        .iter()
        .map(|(n, m)| {
            let (d, count) = m.iter().max_by_key(|(_, c)| **c).unwrap();
            (n.to_string(), (d.to_string(), *count))
        })
        .collect();

    if mapping
        .iter()
        .all(|(n, m)| m.len() == 1 && m.contains_key(n))
    {
        return Some(Finding {
            ncz_key: ncz_key.to_owned(),
            relation: Relation::Equal,
            support,
            mapping: flat,
            exceptions,
        });
    }

    let all_numeric = numbers.len() == support;
    let distinct_x: BTreeSet<String> = numbers.iter().map(|(x, _)| format_number(*x)).collect();
    if all_numeric
        && distinct_x.len() >= 3
        && let Some((a, b)) = linear_fit(&numbers)
    {
        return Some(Finding {
            ncz_key: ncz_key.to_owned(),
            relation: Relation::Linear { a, b },
            support,
            mapping: flat,
            exceptions,
        });
    }

    let functional = exceptions.is_empty();
    let mostly = agreeing as f64 >= MOSTLY * support as f64;
    let repeats = support >= 2 * ncz_values;
    if !(functional || mostly && repeats) || support < 2 {
        return None;
    }

    // A field that differs in every record, such as a name or an index, predicts anything.
    let is_identifier = ncz_values == support && support >= 5;
    if is_identifier {
        return None;
    }

    // A value seen once predicts anything, so every NCZ value has to repeat on average to be trusted.
    let relation = if !functional {
        Relation::Mostly
    } else if support < 2 * ncz_values {
        Relation::Weak
    } else if dxf_values.len() == ncz_values {
        Relation::OneToOne
    } else {
        Relation::Determines
    };

    Some(Finding {
        ncz_key: ncz_key.to_owned(),
        relation,
        support,
        mapping: flat,
        exceptions,
    })
}

fn describe(finding: &Finding) -> String {
    let relation = match finding.relation {
        Relation::Linear { a, b } if (a - 1.0).abs() < 1e-6 && b.abs() < 1e-6 => {
            "equals".to_owned()
        }
        Relation::Linear { a, b } => format!("= {} * x + {}", format_number(a), format_number(b)),
        Relation::Equal => "equals".to_owned(),
        Relation::OneToOne => "one-to-one".to_owned(),
        Relation::Determines => "determines".to_owned(),
        Relation::Weak => "weak, values seldom repeat".to_owned(),
        Relation::Mostly => {
            let exceptions: usize = finding.exceptions.iter().map(|(_, _, c)| c).sum();
            let listed: Vec<String> = finding
                .exceptions
                .iter()
                .take(5)
                .map(|(n, d, c)| format!("{n}→{d} ({c})"))
                .collect();
            format!("{exceptions} exceptions: {}", listed.join(", "))
        }
    };
    let mut mapping: Vec<String> = finding
        .mapping
        .iter()
        .take(10)
        .map(|(n, (d, c))| format!("{n}→{d} ({c})"))
        .collect();
    if finding.mapping.len() > 10 {
        mapping.push(format!("… {} more", finding.mapping.len() - 10));
    }

    format!(
        "{}{} [{relation}, n={}]: {}",
        finding.ncz_key,
        if is_known(&finding.ncz_key) { "" } else { " *" },
        finding.support,
        mapping.join(", ")
    )
}

/// How strong a finding is, strongest first: relation, then fields with unknown meaning, then support.
fn rank(finding: &Finding) -> (u8, bool, usize) {
    let relation = match finding.relation {
        Relation::Equal | Relation::Linear { .. } => 0,
        Relation::OneToOne => 1,
        Relation::Determines => 2,
        Relation::Mostly => 3,
        Relation::Weak => 4,
    };

    (
        relation,
        is_known(&finding.ncz_key),
        usize::MAX - finding.support,
    )
}

pub fn report(records: &[Record], max_per_code: usize) -> String {
    let mut out = String::new();
    let mut domains: BTreeMap<&str, Vec<&Record>> = BTreeMap::new();
    for record in records {
        domains.entry(&record.domain).or_default().push(record);
    }

    for (domain, records) in domains {
        let ncz_keys: BTreeSet<&String> = records.iter().flat_map(|r| r.ncz.keys()).collect();
        let dxf_keys: BTreeSet<&String> = records.iter().flat_map(|r| r.dxf.keys()).collect();
        let varying_ncz: Vec<&String> = ncz_keys
            .into_iter()
            .filter(|k| distinct(&records, k, true).len() >= 2)
            .collect();
        let varying_dxf: Vec<&String> = dxf_keys
            .into_iter()
            .filter(|k| distinct(&records, k, false).len() >= 2)
            .collect();

        let _ = writeln!(out, "\n=== {domain}  ({} pairs) ===", records.len());

        let mut explained_ncz: BTreeSet<&str> = BTreeSet::new();
        let mut unexplained_dxf = vec![];
        for dxf_key in &varying_dxf {
            let values = distinct(&records, dxf_key, false);
            let mut findings: Vec<Finding> = varying_ncz
                .iter()
                .filter_map(|n| relate(&records, n, dxf_key))
                .collect();
            if findings.is_empty() {
                unexplained_dxf.push((dxf_key, values));
                continue;
            }
            findings.sort_by_key(rank);
            for finding in &findings {
                if let Some(key) = varying_ncz.iter().find(|k| ***k == finding.ncz_key) {
                    explained_ncz.insert(key.as_str());
                }
            }

            let _ = writeln!(out, "\nDXF {dxf_key}  ({} distinct values)", values.len());
            for finding in findings.iter().take(max_per_code) {
                let _ = writeln!(out, "    {}", describe(finding));
            }
            if findings.len() > max_per_code {
                let _ = writeln!(out, "    … {} more", findings.len() - max_per_code);
            }
        }

        if !unexplained_dxf.is_empty() {
            let _ = writeln!(
                out,
                "\nDXF codes that vary but no NCZ field predicts (a field may be missing from the parser):"
            );
            for (key, values) in unexplained_dxf {
                let sample: Vec<&str> = values.iter().take(8).copied().collect();
                let _ = writeln!(out, "    {key}: {}", sample.join(", "));
            }
        }

        let silent_keys: Vec<&&String> = varying_ncz
            .iter()
            .filter(|k| !is_known(k) && !explained_ncz.contains(k.as_str()))
            .collect();
        // Bits are listed per field, e.g. `flags bits 0 8 9`, rather than one line each.
        let mut bits: BTreeMap<&str, Vec<u32>> = BTreeMap::new();
        for key in &silent_keys {
            if let Some((field, bit)) = key.rsplit_once(".bit")
                && let Ok(bit) = bit.parse()
            {
                bits.entry(field).or_default().push(bit);
            }
        }
        let mut silent: Vec<String> = silent_keys
            .iter()
            .filter(|k| !k.contains(".bit"))
            .map(|k| {
                let values: Vec<&str> = distinct(&records, k, true).into_iter().take(6).collect();
                format!("    {k}: {}", values.join(", "))
            })
            .collect();
        for (field, mut numbers) in bits {
            numbers.sort_unstable();
            let numbers: Vec<String> = numbers.iter().map(u32::to_string).collect();
            silent.push(format!("    {field} bits {}", numbers.join(" ")));
        }
        if !silent.is_empty() {
            let _ = writeln!(
                out,
                "\nUnknown NCZ fields that vary but predict no DXF code:"
            );
            let _ = writeln!(out, "{}", silent.join("\n"));
        }
    }

    out
}
