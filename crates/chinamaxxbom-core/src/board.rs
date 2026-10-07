use crate::{finite, quoted, sexpr, xml};
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Kicad,
    Eagle,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Top,
    Bottom,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Part {
    pub reference: String,
    pub value: String,
    pub footprint: String,
    pub lcsc: String,
    /// Millimetres, positive Y upwards. Relative to the assembly origin.
    pub x: f64,
    pub y: f64,
    /// Assembly rotation, degrees CCW viewed from above; bottom mirrored.
    pub rotation: f64,
    pub side: Side,
    pub dnp: bool,
    pub exclude_bom: bool,
    pub exclude_cpl: bool,
    #[serde(default = "default_placed")]
    pub placed: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Board {
    pub format: Format,
    pub parts: Vec<Part>,
    pub warnings: Vec<String>,
}

fn default_placed() -> bool {
    true
}

const LCSC_FIELDS: &[&str] = &[
    "LCSC",
    "LCSC Part",
    "LCSC Part #",
    "LCSC Part Number",
    "JLCPCB",
    "JLCPCB Part #",
    "JLCPCB Part Number",
    "JLCPCB Part",
];
fn is_lcsc(k: &str) -> bool {
    LCSC_FIELDS.iter().any(|s| s.eq_ignore_ascii_case(k))
}
pub(crate) fn field(props: &BTreeMap<String, String>) -> String {
    LCSC_FIELDS
        .iter()
        .find_map(|k| {
            props
                .iter()
                .find(|(p, v)| p.eq_ignore_ascii_case(k) && !v.trim().is_empty())
                .map(|(_, v)| v.trim().to_ascii_uppercase())
        })
        .unwrap_or_default()
}
pub(crate) fn truth(s: &str) -> bool {
    matches!(
        s.to_ascii_lowercase().as_str(),
        "yes" | "true" | "1" | "dnp" | "dnf"
    )
}
fn number(n: Option<&sexpr::Node>, i: usize, default: Option<f64>) -> Result<f64> {
    let s = n.map(|n| n.at(i)).unwrap_or("");
    if s.is_empty() {
        return default.context("Missing required board coordinate");
    }
    finite(s)
}

pub fn load(path: &Path) -> Result<Board> {
    let s = std::fs::read_to_string(path).with_context(|| {
        format!(
            "Read {} (binary Eagle files are unsupported; save as Eagle 6+ XML)",
            path.display()
        )
    })?;
    parse(&s)
}
pub fn parse(s: &str) -> Result<Board> {
    let mut board = if s.trim_start().starts_with('(') {
        kicad(s)?
    } else {
        eagle(s)?
    };
    let mut refs = BTreeSet::new();
    for p in &board.parts {
        ensure!(!p.reference.is_empty(), "Board has an empty reference");
        ensure!(
            refs.insert(&p.reference),
            "Duplicate reference {}",
            p.reference
        );
    }
    board
        .parts
        .sort_by(|a, b| natural(&a.reference).cmp(&natural(&b.reference)));
    Ok(board)
}
fn natural(s: &str) -> (String, u64, String) {
    let i = s.find(|c: char| c.is_ascii_digit()).unwrap_or(s.len());
    (s[..i].into(), s[i..].parse().unwrap_or(0), s.into())
}
fn kicad(s: &str) -> Result<Board> {
    let root = sexpr::parse(s)?;
    ensure!(
        root.tag() == "kicad_pcb",
        "Expected a .kicad_pcb board, not a schematic or library"
    );
    let origin = root.child("setup").and_then(|s| s.child("aux_axis_origin"));
    let ox = number(origin, 1, Some(0.))?;
    let oy = number(origin, 2, Some(0.))?;
    let mut parts = vec![];
    let mut warnings = vec!["CPL uses footprint origins; apply per-part offsets where the assembly centre differs. Review rotations against JLCPCB's preview.".into()];
    for fp in root.all("footprint") {
        let props: BTreeMap<_, _> = fp
            .all("property")
            .map(|p| (p.at(1).to_owned(), p.at(2).to_owned()))
            .collect();
        let legacy = |kind| {
            fp.all("fp_text")
                .find(|p| p.at(1) == kind)
                .map(|p| p.at(2).to_owned())
                .unwrap_or_default()
        };
        let reference = props
            .get("Reference")
            .cloned()
            .unwrap_or_else(|| legacy("reference"));
        let value = props
            .get("Value")
            .cloned()
            .unwrap_or_else(|| legacy("value"));
        let side = match fp.value("layer") {
            "F.Cu" => Side::Top,
            "B.Cu" => Side::Bottom,
            l => bail!("{reference}: unsupported footprint layer {l}"),
        };
        let rot = number(fp.child("at"), 3, Some(0.))?;
        let attr = |a| fp.child("attr").is_some_and(|n| n.has_atom(a)) || truth(fp.value(a));
        parts.push(Part {
            placed: true,
            reference,
            value,
            footprint: fp.at(1).into(),
            lcsc: field(&props),
            x: number(fp.child("at"), 1, None)? - ox,
            y: oy - number(fp.child("at"), 2, None)?,
            rotation: (if side == Side::Bottom {
                180. - rot
            } else {
                rot
            })
            .rem_euclid(360.),
            side,
            dnp: attr("dnp") || props.get("DNP").is_some_and(|v| truth(v)),
            exclude_bom: attr("exclude_from_bom"),
            exclude_cpl: attr("exclude_from_pos"),
        });
    }
    if s.contains("(variant") {
        warnings.push(
            "Named variants are not evaluated; this is the board's default assembly state.".into(),
        );
    }
    Ok(Board {
        format: Format::Kicad,
        parts,
        warnings,
    })
}
fn doc(s: &str) -> Result<roxmltree::Document<'_>> {
    // roxmltree never fetches the external eagle.dtd. Allow the standard declaration.
    Ok(roxmltree::Document::parse_with_options(
        s,
        roxmltree::ParsingOptions {
            allow_dtd: true,
            nodes_limit: 2_000_000,
        },
    )?)
}
fn eagle(s: &str) -> Result<Board> {
    let d = doc(s)?;
    ensure!(
        d.root_element().has_tag_name("eagle"),
        "Expected Eagle XML .brd file"
    );
    let board = d
        .descendants()
        .find(|n| n.has_tag_name("board"))
        .context("Eagle file has no board; open the .brd, not .sch or .lbr")?;
    let mut parts = vec![];
    for el in board
        .children()
        .find(|n| n.has_tag_name("elements"))
        .into_iter()
        .flat_map(|n| n.children())
        .filter(|n| n.has_tag_name("element"))
    {
        let a = |key| el.attribute(key).unwrap_or("");
        let rotation = a("rot");
        let side = if rotation.contains('M') {
            Side::Bottom
        } else {
            Side::Top
        };
        let rot = if rotation.is_empty() {
            0.
        } else {
            finite(rotation.trim_start_matches(['S', 'M', 'R']))?
        };
        let props: BTreeMap<_, _> = el
            .children()
            .filter(|n| n.has_tag_name("attribute"))
            .map(|n| {
                (
                    n.attribute("name").unwrap_or("").into(),
                    n.attribute("value").unwrap_or("").into(),
                )
            })
            .collect();
        parts.push(Part {
            placed: true,
            reference: a("name").into(),
            value: a("value").into(),
            footprint: format!("{}:{}", a("library"), a("package")),
            lcsc: field(&props),
            x: finite(a("x"))?,
            y: finite(a("y"))?,
            rotation: (if side == Side::Bottom {
                180. - rot
            } else {
                rot
            })
            .rem_euclid(360.),
            side,
            dnp: a("populate") == "no"
                || props
                    .iter()
                    .any(|(k, v)| k.eq_ignore_ascii_case("DNP") && truth(v)),
            exclude_bom: false,
            exclude_cpl: false,
        });
    }
    Ok(Board { format: Format::Eagle, parts, warnings: vec!["CPL uses Eagle element origins and the drawing origin (0,0). Apply centre offsets and review bottom-side rotations in JLCPCB's preview.".into(), "Eagle assembly variants are not evaluated; default populate/DNP attributes are used.".into()] })
}

/// Change only LCSC attribute values, preserving every other byte of the CAD file.
/// Caller writes to a separate destination to retain Eagle board/schematic pairing.
pub fn assigned_copy(s: &str, assignments: &BTreeMap<String, String>) -> Result<String> {
    let board = parse(s)?;
    for (r, id) in assignments {
        ensure!(
            board.parts.iter().any(|p| &p.reference == r),
            "Unknown reference {r}"
        );
        crate::lcsc_id(id)?;
    }
    let mut edits = vec![];
    match board.format {
        Format::Kicad => {
            let root = sexpr::parse(s)?;
            for fp in root.all("footprint") {
                let reference = fp
                    .all("property")
                    .find(|p| p.at(1) == "Reference")
                    .map(|p| p.at(2))
                    .or_else(|| {
                        fp.all("fp_text")
                            .find(|p| p.at(1) == "reference")
                            .map(|p| p.at(2))
                    })
                    .unwrap_or("");
                if let Some(id) = assignments.get(reference) {
                    let fields: Vec<_> = fp.all("property").filter(|p| is_lcsc(p.at(1))).collect();
                    if fields.is_empty() {
                        let i = fp.span.end - 1;
                        edits.push((i..i, format!("\n(property \"LCSC\" {} (at 0 0 0) (layer \"F.Fab\") (effects (font (size 1 1) (thickness 0.15)) (hide yes)))\n", quoted(id))));
                    } else {
                        for p in fields {
                            let v = p.children.get(2).context("Malformed LCSC property")?;
                            edits.push((v.span.clone(), quoted(id)));
                        }
                    }
                }
            }
        }
        Format::Eagle => {
            let d = doc(s)?;
            for el in d.descendants().filter(|n| {
                n.has_tag_name("element") && n.parent().is_some_and(|p| p.has_tag_name("elements"))
            }) {
                if let Some(id) = assignments.get(el.attribute("name").unwrap_or("")) {
                    let fields: Vec<_> = el
                        .children()
                        .filter(|n| {
                            n.has_tag_name("attribute")
                                && is_lcsc(n.attribute("name").unwrap_or(""))
                        })
                        .collect();
                    if fields.is_empty() {
                        let span = el.range();
                        let raw = &s[span.clone()];
                        let field = format!("<attribute name=\"LCSC\" value=\"{}\"/>\n", xml(id));
                        if raw.ends_with("/>") {
                            edits.push((span.end - 2..span.end, format!(">{field}</element>")));
                        } else {
                            let i = span.start
                                + raw.rfind("</element>").context("Malformed Eagle element")?;
                            edits.push((i..i, field));
                        }
                    } else {
                        for n in fields {
                            let attr = n
                                .attributes()
                                .find(|a| a.name() == "value")
                                .context("Missing attribute value")?;
                            edits.push((attr.range_value(), xml(id)));
                        }
                    }
                }
            }
        }
    }
    edits.sort_by_key(|e| std::cmp::Reverse(e.0.start));
    let mut result = s.to_string();
    for (range, replacement) in edits {
        result.replace_range(range, &replacement);
    }
    parse(&result).context("Validate assigned board copy")?;
    Ok(result)
}
