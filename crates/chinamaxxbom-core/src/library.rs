//! EasyEDA Standard -> shared geometry -> KiCad 10 / Eagle 9 libraries.
//! Schema and unit conventions researched in JLCImport (MIT, see licenses/).
use crate::{
    finite, quoted as q,
    supplier::{result, Bundle},
    xml as e,
};
use anyhow::{bail, ensure, Context, Result};
use kurbo::{BezPath, PathEl};
use serde::Serialize;
use serde_json::Value;
use std::{collections::BTreeSet, fmt::Write, path::Path};
const UNIT: f64 = 0.254;
type Pt = (f64, f64);
#[derive(Debug, Clone)]
struct Pad {
    number: String,
    at: Pt,
    w: f64,
    h: f64,
    drill: f64,
    rotation: f64,
    shape: String,
    layer: String,
}
#[derive(Debug, Clone)]
struct Pin {
    number: String,
    name: String,
    at: Pt,
    length: f64,
    angle: f64,
    kind: &'static str,
}
#[derive(Debug, Clone)]
struct Line {
    points: Vec<Pt>,
    width: f64,
    layer: String,
    fill: bool,
    closed: bool,
}
#[derive(Debug, Clone)]
struct Unit {
    pins: Vec<Pin>,
    lines: Vec<Line>,
}
#[derive(Debug, Clone)]
pub struct Library {
    id: String,
    title: String,
    prefix: String,
    description: String,
    pads: Vec<Pad>,
    lines: Vec<Line>,
    holes: Vec<(Pt, f64)>,
    units: Vec<Unit>,
    pub warnings: Vec<String>,
}
#[derive(Debug, Serialize)]
pub struct Report {
    pub lcsc: String,
    pub title: String,
    pub pads: usize,
    pub pins: usize,
    pub units: usize,
    pub warnings: Vec<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Kicad,
    Eagle,
    Both,
}
fn num(x: f64) -> String {
    let x = if x.abs() < 0.0000005 { 0. } else { x };
    format!("{x:.6}")
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}
fn get<'a>(p: &'a [&str], i: usize) -> Result<&'a str> {
    p.get(i)
        .copied()
        .with_context(|| format!("Missing shape field {i}"))
}
fn n(p: &[&str], i: usize) -> Result<f64> {
    finite(get(p, i)?)
}
fn optional(p: &[&str], i: usize) -> Result<f64> {
    match p.get(i) {
        None | Some(&"") => Ok(0.),
        Some(s) => finite(s),
    }
}
fn data(v: Value) -> Result<Value> {
    let v = result(v)?;
    let d = v.get("dataStr").context("Missing EasyEDA dataStr")?;
    Ok(if let Some(s) = d.as_str() {
        serde_json::from_str(s)?
    } else {
        d.clone()
    })
}
fn origin(d: &Value) -> Result<Pt> {
    let read = |k| {
        let v = &d["head"][k];
        if let Some(n) = v.as_f64() {
            ensure!(n.is_finite(), "Invalid origin");
            Ok(n)
        } else if let Some(s) = v.as_str() {
            finite(s)
        } else {
            bail!("Missing origin {k}")
        }
    };
    Ok((read("x")?, read("y")?))
}
fn rel(p: Pt, o: Pt) -> Pt {
    ((p.0 - o.0) * UNIT, -(p.1 - o.1) * UNIT)
}
fn coords(s: &str) -> Result<Vec<Pt>> {
    let ns: Vec<f64> = s
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|s| !s.is_empty())
        .map(finite)
        .collect::<Result<_>>()?;
    ensure!(ns.len() >= 4 && ns.len() % 2 == 0, "Invalid point list");
    Ok(ns.chunks_exact(2).map(|p| (p[0], p[1])).collect())
}
fn svg(s: &str) -> Result<Vec<(Vec<Pt>, bool)>> {
    let path = BezPath::from_svg(s).map_err(|e| anyhow::anyhow!("SVG path: {e:?}"))?;
    let mut result = vec![];
    let mut points = vec![];
    kurbo::flatten(
        path.elements().iter().copied(),
        0.01 / UNIT,
        |el| match el {
            PathEl::MoveTo(p) => {
                if !points.is_empty() {
                    result.push((std::mem::take(&mut points), false));
                }
                points.push((p.x, p.y));
            }
            PathEl::LineTo(p) => points.push((p.x, p.y)),
            PathEl::ClosePath => {
                if !points.is_empty() {
                    result.push((std::mem::take(&mut points), true));
                }
            }
            _ => {}
        },
    );
    if !points.is_empty() {
        result.push((points, false));
    }
    ensure!(
        !result.is_empty()
            && result
                .iter()
                .all(|(ps, _)| ps.iter().all(|p| p.0.is_finite() && p.1.is_finite())),
        "Empty or non-finite SVG geometry"
    );
    Ok(result)
}
fn layer(id: &str) -> Result<&'static str> {
    Ok(match id {
        "1" => "F.Cu",
        "2" => "B.Cu",
        "3" => "F.SilkS",
        "4" => "B.SilkS",
        "5" => "F.Paste",
        "6" => "B.Paste",
        "7" => "F.Mask",
        "8" => "B.Mask",
        "10" => "Edge.Cuts",
        "12" | "13" => "F.Fab",
        "14" => "B.Fab",
        _ => bail!("Unsupported EasyEDA layer {id}"),
    })
}
fn circle(at: Pt, r: f64, width: f64, layer: &str, fill: bool) -> Line {
    // At least 32 segments; 0.01 mm radial approximation tolerance.
    let count = if r <= 0.01 {
        32
    } else {
        (std::f64::consts::PI / (1. - 0.01 / r).acos())
            .ceil()
            .clamp(32., 4096.) as usize
    };
    let points = (0..count)
        .map(|i| {
            let a = std::f64::consts::TAU * i as f64 / count as f64;
            (at.0 + r * a.cos(), at.1 + r * a.sin())
        })
        .collect();
    Line {
        points,
        width,
        layer: layer.into(),
        fill,
        closed: true,
    }
}
impl Library {
    pub fn parse(bundle: &Bundle) -> Result<Self> {
        let id = crate::lcsc_id(&bundle.lcsc)?;
        ensure!(!bundle.symbols.is_empty(), "No symbol units");
        let primary = result(bundle.symbols[0].clone())?;
        let sd = data(primary.clone())?;
        let cp = &sd["head"]["c_para"];
        let title = primary["title"].as_str().unwrap_or(&id).to_string();
        let prefix = cp["pre"]
            .as_str()
            .unwrap_or("U?")
            .trim_end_matches('?')
            .to_string();
        let mut lib = Self {
            id,
            title,
            prefix,
            description: primary["description"].as_str().unwrap_or("").into(),
            pads: vec![],
            lines: vec![],
            holes: vec![],
            units: vec![],
            warnings: vec![],
        };
        let fd = data(bundle.footprint.clone())?;
        let o = origin(&fd)?;
        for shape in fd["shape"].as_array().context("Missing footprint shapes")? {
            let shape = shape.as_str().context("Non-string footprint shape")?;
            let p: Vec<_> = shape.split('~').collect();
            let point = |x, y| -> Result<Pt> { Ok(rel((n(&p, x)?, n(&p, y)?), o)) };
            match p[0] {
                "PAD" => {
                    let shape = get(&p, 1)?;
                    ensure!(
                        ["RECT", "ELLIPSE", "OVAL"].contains(&shape),
                        "Unsupported pad shape {shape}; no approximate pad will be generated"
                    );
                    ensure!(
                        optional(&p, 13)? == 0.,
                        "Slotted plated pads are not implemented"
                    );
                    let (w, h) = (n(&p, 4)? * UNIT, n(&p, 5)? * UNIT);
                    let drill = optional(&p, 9)? * 2. * UNIT;
                    ensure!(
                        w > 0. && h > 0. && drill >= 0. && drill < w.min(h),
                        "Invalid pad dimensions"
                    );
                    let layer = get(&p, 6)?.to_string();
                    ensure!(
                        ["1", "2", "11"].contains(&layer.as_str()),
                        "Unsupported pad layer {layer}"
                    );
                    ensure!((layer == "11") == (drill > 0.), "Pad drill/layer mismatch");
                    lib.pads.push(Pad {
                        number: get(&p, 8)?.into(),
                        at: point(2, 3)?,
                        w,
                        h,
                        drill,
                        rotation: optional(&p, 11)?,
                        shape: shape.into(),
                        layer,
                    });
                }
                "TRACK" => {
                    if ["99", "100", "101"].contains(&get(&p, 2)?) {
                        continue;
                    }
                    let points = coords(get(&p, 4)?)?
                        .into_iter()
                        .map(|v| rel(v, o))
                        .collect();
                    lib.lines.push(Line {
                        points,
                        width: n(&p, 1)? * UNIT,
                        layer: layer(get(&p, 2)?)?.into(),
                        fill: false,
                        closed: false,
                    });
                }
                "CIRCLE" => {
                    if ["99", "100", "101"].contains(&get(&p, 5)?) {
                        continue;
                    }
                    let r = n(&p, 3)? * UNIT;
                    ensure!(r > 0., "Invalid circle radius");
                    lib.lines.push(circle(
                        point(1, 2)?,
                        r,
                        n(&p, 4)? * UNIT,
                        layer(get(&p, 5)?)?,
                        false,
                    ));
                }
                "HOLE" => {
                    let diameter = n(&p, 3)? * 2. * UNIT;
                    ensure!(diameter > 0., "Invalid hole");
                    lib.holes.push((point(1, 2)?, diameter));
                }
                "SOLIDREGION" | "ARC" => {
                    let li = if p[0] == "ARC" { 2 } else { 1 };
                    if ["99", "100", "101"].contains(&get(&p, li)?) {
                        continue;
                    }
                    let path = p
                        .iter()
                        .find(|s| s.trim_start().starts_with('M'))
                        .context("Missing SVG region/arc path")?;
                    let fill = p[0] == "SOLIDREGION";
                    if fill {
                        ensure!(p.get(4) == Some(&"solid"), "Unsupported cutout/NPTH region");
                    }
                    let paths = svg(path)?;
                    ensure!(
                        !fill || paths.len() == 1,
                        "Compound filled paths need hole handling"
                    );
                    for (points, closed) in paths {
                        lib.lines.push(Line {
                            points: points.into_iter().map(|v| rel(v, o)).collect(),
                            width: if fill { 0. } else { n(&p, 1)? * UNIT },
                            layer: layer(get(&p, li)?)?.into(),
                            fill,
                            closed,
                        });
                    }
                }
                "RECT" => {
                    let a = point(1, 2)?;
                    let (w, h) = (n(&p, 3)? * UNIT, n(&p, 4)? * UNIT);
                    lib.lines.push(Line {
                        points: vec![a, (a.0 + w, a.1), (a.0 + w, a.1 - h), (a.0, a.1 - h)],
                        width: optional(&p, 8)? * UNIT,
                        layer: layer(get(&p, 5)?)?.into(),
                        fill: false,
                        closed: true,
                    });
                }
                "SVGNODE" => lib.warnings.push(
                    "3D model metadata is present; 3D model download/alignment is not implemented."
                        .into(),
                ),
                "TEXT" => lib.warnings.push(
                    "Footprint text was omitted; generated reference/value fields are provided."
                        .into(),
                ),
                other => bail!("Unsupported footprint primitive {other}"),
            }
        }
        for symbol in &bundle.symbols {
            let d = data(symbol.clone())?;
            let o = origin(&d)?;
            let mut unit = Unit {
                pins: vec![],
                lines: vec![],
            };
            for s in d["shape"].as_array().context("Missing symbol shapes")? {
                let s = s.as_str().context("Non-string symbol primitive")?;
                let p: Vec<_> = s.split('~').collect();
                let point = |x, y| -> Result<Pt> { Ok(rel((n(&p, x)?, n(&p, y)?), o)) };
                match p[0] {
                    "P"=>{
                        let sections:Vec<_>=s.split("^^").collect();let h:Vec<_>=sections[0].split('~').collect();
                        let at=(n(&h,4)?,n(&h,5)?);
                        let path=sections.get(2).context("Missing pin path")?.split('~').next().unwrap_or("");
                        let paths=svg(path)?;ensure!(paths.len()==1&&paths[0].0.len()==2,"Pin path must be a single straight segment");
                        let pts=&paths[0].0;let a=pts[0];let b=pts[1];
                        let dist=|p:Pt| (p.0-at.0).hypot(p.1-at.1);
                        let end=if dist(a)<0.01{b}else{ensure!(dist(b)<0.01,"Pin connection does not match either path endpoint");a};
                        let(dx,dy)=(end.0-at.0,-(end.1-at.1));
                        let angle=dy.atan2(dx).to_degrees().rem_euclid(360.);
                        let name=sections.get(3).and_then(|v|v.split('~').nth(4)).unwrap_or("");
                        let number=sections.get(4).and_then(|v|v.split('~').nth(4)).filter(|s|!s.is_empty()).unwrap_or(get(&h,3)?);
                        let kind=match get(&h,2)?{"0"=>"unspecified","1"=>"input","2"=>"output","3"=>"bidirectional","4"=>"power_in",other=>bail!("Unsupported electrical pin type {other}")};
                        unit.pins.push(Pin{number:number.into(),name:name.into(),at:rel(at,o),length:dx.hypot(dy)*UNIT,angle,kind});
                    },
                    "R"=>{
                        let a=point(1,2)?;let(wi,hi)=if p.len()>=12{(5,6)}else{(3,4)};let(w,h)=(n(&p,wi)?*UNIT,n(&p,hi)?*UNIT);
                        if p.len()>=12&&(optional(&p,3)?>0.||optional(&p,4)?>0.){lib.warnings.push("Symbol rounded rectangle rendered with square corners.".into());}
                        unit.lines.push(Line{points:vec![a,(a.0+w,a.1),(a.0+w,a.1-h),(a.0,a.1-h)],width:0.254,layer:String::new(),fill:false,closed:true});
                    },
                    "E"|"C"=>{
                        let r=n(&p,3)?;if p[0]=="E"{ensure!((r-n(&p,4)?).abs()<1e-6,"Elliptical symbol primitive unsupported");}
                        unit.lines.push(circle(point(1,2)?,r*UNIT,0.254,"",false));
                    },
                    "PL"|"PG"=>unit.lines.push(Line{points:coords(get(&p,1)?)?.into_iter().map(|v|rel(v,o)).collect(),width:0.254,layer:String::new(),fill:p[0]=="PG",closed:p[0]=="PG"}),
                    "PT"|"A"=>{
                        let path=p.iter().find(|s|s.trim_start().starts_with('M')).context("Missing symbol SVG path")?;
                        for(points,closed) in svg(path)? {unit.lines.push(Line{points:points.into_iter().map(|v|rel(v,o)).collect(),width:0.254,layer:String::new(),fill:false,closed});}
                    },
                    "T"=>lib.warnings.push("Decorative symbol text omitted; pin labels and component properties are retained.".into()),
                    other=>bail!("Unsupported symbol primitive {other}"),
                }
            }
            ensure!(!unit.pins.is_empty(), "Symbol unit has no pins");
            lib.units.push(unit);
        }
        ensure!(!lib.pads.is_empty(), "Footprint has no pads");
        let pads: BTreeSet<_> = lib.pads.iter().map(|p| p.number.as_str()).collect();
        ensure!(!pads.contains(""), "Electrical pad without a number");
        for unit in &lib.units {
            let mut pins = BTreeSet::new();
            for p in &unit.pins {
                ensure!(
                    pads.contains(p.number.as_str()),
                    "Pin {} has no corresponding footprint pad",
                    p.number
                );
                ensure!(pins.insert(&p.number), "Duplicate symbol pin {}", p.number);
            }
        }
        let pins: BTreeSet<_> = lib
            .units
            .iter()
            .flat_map(|u| u.pins.iter())
            .map(|p| p.number.as_str())
            .collect();
        let extras: Vec<_> = pads.difference(&pins).copied().collect();
        if !extras.is_empty() {
            lib.warnings.push(format!(
                "Footprint pads without symbol pins (check mechanical pads): {}",
                extras.join(", ")
            ));
        }
        lib.warnings.sort();
        lib.warnings.dedup();
        Ok(lib)
    }
    pub fn report(&self) -> Report {
        Report {
            lcsc: self.id.clone(),
            title: self.title.clone(),
            pads: self.pads.len(),
            pins: self.units.iter().map(|u| u.pins.len()).sum(),
            units: self.units.len(),
            warnings: self.warnings.clone(),
        }
    }
    pub fn write(&self, out: &Path, target: Target) -> Result<Report> {
        ensure!(
            !out.exists(),
            "Library output directory exists; choose a new directory"
        );
        // Generate and validate before creating any output.
        let kc = if target != Target::Eagle {
            Some((self.kicad_symbol()?, self.kicad_footprint()?))
        } else {
            None
        };
        let eagle = if target != Target::Kicad {
            Some(self.eagle()?)
        } else {
            None
        };
        let parent = out
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        std::fs::create_dir_all(parent)?;
        let stage = tempfile::tempdir_in(parent)?;
        let dir = stage.path();
        if let Some((sym, fp)) = kc {
            let pretty = dir.join(format!("{}.pretty", self.id));
            std::fs::create_dir(&pretty)?;
            std::fs::write(pretty.join(format!("{}.kicad_mod", self.id)), fp)?;
            std::fs::write(dir.join(format!("{}.kicad_sym", self.id)), sym)?;
            std::fs::write(dir.join("sym-lib-table"),format!("(sym_lib_table (version 7) (lib (name {0}) (type KiCad) (uri \"${{KIPRJMOD}}/{1}.kicad_sym\") (options \"\") (descr \"\")))\n",q(&self.id),self.id))?;
            std::fs::write(dir.join("fp-lib-table"),format!("(fp_lib_table (version 7) (lib (name {0}) (type KiCad) (uri \"${{KIPRJMOD}}/{1}.pretty\") (options \"\") (descr \"\")))\n",q(&self.id),self.id))?;
        }
        if let Some(text) = eagle {
            std::fs::write(dir.join(format!("{}.lbr", self.id)), text)?;
        }
        std::fs::write(
            dir.join("report.json"),
            serde_json::to_vec_pretty(&self.report())?,
        )?;
        std::fs::rename(dir, out)?;
        Ok(self.report())
    }
    pub fn kicad_footprint(&self) -> Result<String> {
        let mut s=format!("(footprint {} (version 20241229) (generator \"chinamaxxbom\") (layer \"F.Cu\")\n(descr {})\n(attr {})\n",q(&self.id),q(&self.description),if self.pads.iter().any(|p|p.drill>0.){"through_hole"}else{"smd"});
        for (key, val, y) in [
            ("Reference", "REF**", -4.),
            ("Value", self.title.as_str(), 4.),
            ("LCSC", self.id.as_str(), 6.),
        ] {
            writeln!(s,"(property {} {} (at 0 {y} 0) (layer \"F.Fab\") (effects (font (size 1 1) (thickness 0.15))))",q(key),q(val))?;
        }
        for l in &self.lines {
            write_kicad_line(&mut s, l, false)?;
        }
        for p in &self.pads {
            let shape = match p.shape.as_str() {
                "RECT" => "rect",
                "ELLIPSE" if (p.w - p.h).abs() < 1e-6 => "circle",
                _ => "oval",
            };
            let layers = match p.layer.as_str() {
                "11" => "\"*.Cu\" \"*.Mask\"",
                "2" => "\"B.Cu\" \"B.Paste\" \"B.Mask\"",
                _ => "\"F.Cu\" \"F.Paste\" \"F.Mask\"",
            };
            let drill = if p.drill > 0. {
                format!(" (drill {})", num(p.drill))
            } else {
                String::new()
            };
            writeln!(
                s,
                "(pad {} {} {shape} (at {} {} {}) (size {} {}){drill} (layers {layers}))",
                q(&p.number),
                if p.drill > 0. { "thru_hole" } else { "smd" },
                num(p.at.0),
                num(-p.at.1),
                num(p.rotation),
                num(p.w),
                num(p.h)
            )?;
        }
        for (at, d) in &self.holes {
            writeln!(s,"(pad \"\" np_thru_hole circle (at {} {}) (size {d} {d}) (drill {d}) (layers \"*.Cu\" \"*.Mask\"))",num(at.0),num(-at.1))?;
        }
        s.push_str("(embedded_fonts no)\n)\n");
        crate::sexpr::parse(&s)?;
        Ok(s)
    }
    pub fn kicad_symbol(&self) -> Result<String> {
        let mut s=format!("(kicad_symbol_lib (version 20241209) (generator \"chinamaxxbom\")\n(symbol {} (in_bom yes) (on_board yes)\n",q(&self.id));
        for (i, (key, value)) in [
            ("Reference", self.prefix.clone()),
            ("Value", self.title.clone()),
            ("Footprint", format!("{}:{}", self.id, self.id)),
            ("LCSC", self.id.clone()),
            ("Description", self.description.clone()),
        ]
        .iter()
        .enumerate()
        {
            writeln!(
                s,
                "(property {} {} (at 0 {} 0) (effects (font (size 1.27 1.27)) {}))",
                q(key),
                q(value),
                num(10. + i as f64 * 2.),
                if i > 1 { "(hide yes)" } else { "" }
            )?;
        }
        for (i, u) in self.units.iter().enumerate() {
            writeln!(s, "(symbol {}", q(&format!("{}_{}_1", self.id, i + 1)))?;
            for l in &u.lines {
                write_kicad_line(&mut s, l, true)?;
            }
            for p in &u.pins {
                writeln!(s,"(pin {} line (at {} {} {}) (length {}) (name {} (effects (font (size 1.27 1.27)))) (number {} (effects (font (size 1.27 1.27)))))",p.kind,num(p.at.0),num(p.at.1),num(p.angle),num(p.length),q(&p.name),q(&p.number))?;
            }
            s.push_str(")\n");
        }
        s.push_str(")\n)\n");
        crate::sexpr::parse(&s)?;
        Ok(s)
    }
    pub fn eagle(&self) -> Result<String> {
        let id = e(&self.id);
        let mut s=String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!DOCTYPE eagle SYSTEM \"eagle.dtd\">\n<eagle version=\"9.6.2\"><drawing><layers>\n");
        for (n, name) in [
            (1, "Top"),
            (16, "Bottom"),
            (21, "tPlace"),
            (22, "bPlace"),
            (25, "tNames"),
            (27, "tValues"),
            (29, "tStop"),
            (30, "bStop"),
            (31, "tCream"),
            (32, "bCream"),
            (20, "Dimension"),
            (51, "tDocu"),
            (52, "bDocu"),
            (94, "Symbols"),
            (95, "Names"),
            (96, "Values"),
        ] {
            writeln!(s,"<layer number=\"{n}\" name=\"{name}\" color=\"7\" fill=\"1\" visible=\"yes\" active=\"yes\"/>")?;
        }
        write!(
            s,
            "</layers><library><description>{}</description><packages><package name=\"{id}\">",
            e(&self.title)
        )?;
        let mut seen = BTreeSet::new();
        for p in &self.pads {
            ensure!(
                seen.insert(&p.number),
                "Eagle requires unique pad names; split pad {} is unsupported",
                p.number
            );
            if p.drill > 0. {
                ensure!(
                    (p.w - p.h).abs() < 0.00001,
                    "Eagle through-hole pad {} is non-circular/non-square; unsupported",
                    p.number
                );
                writeln!(s,"<pad name=\"{}\" x=\"{}\" y=\"{}\" drill=\"{}\" diameter=\"{}\" shape=\"{}\" rot=\"R{}\"/>",e(&p.number),num(p.at.0),num(p.at.1),num(p.drill),num(p.w),if p.shape=="RECT"{"square"}else{"round"},num(p.rotation))?;
            } else {
                writeln!(s,"<smd name=\"{}\" x=\"{}\" y=\"{}\" dx=\"{}\" dy=\"{}\" layer=\"{}\" roundness=\"{}\" rot=\"R{}\"/>",e(&p.number),num(p.at.0),num(p.at.1),num(p.w),num(p.h),if p.layer=="2"{16}else{1},if p.shape=="RECT"{0}else{100},num(p.rotation))?;
            }
        }
        for l in &self.lines {
            write_eagle_line(&mut s, l, false)?;
        }
        for (at, d) in &self.holes {
            writeln!(
                s,
                "<hole x=\"{}\" y=\"{}\" drill=\"{}\"/>",
                num(at.0),
                num(at.1),
                num(*d)
            )?;
        }
        s.push_str("<text x=\"0\" y=\"4\" size=\"1.27\" layer=\"25\">&gt;NAME</text><text x=\"0\" y=\"-4\" size=\"1.27\" layer=\"27\">&gt;VALUE</text></package></packages><symbols>");
        for (i, u) in self.units.iter().enumerate() {
            write!(s, "<symbol name=\"{id}_{}\">", i + 1)?;
            for l in &u.lines {
                write_eagle_line(&mut s, l, true)?;
            }
            for p in &u.pins {
                let length = [
                    (0., "point"),
                    (2.54, "short"),
                    (5.08, "middle"),
                    (7.62, "long"),
                ]
                .into_iter()
                .find(|(n, _)| (n - p.length).abs() < 0.001)
                .map(|(_, n)| n)
                .with_context(|| {
                    format!(
                        "Eagle pin {} has unsupported length {} mm",
                        p.number, p.length
                    )
                })?;
                ensure!(
                    (p.angle / 90. - (p.angle / 90.).round()).abs() < 0.0001,
                    "Eagle pin angle must be orthogonal"
                );
                let direction = match p.kind {
                    "input" => "in",
                    "output" => "out",
                    "bidirectional" => "io",
                    "power_in" => "pwr",
                    _ => "pas",
                };
                writeln!(s,"<pin name=\"{}\" x=\"{}\" y=\"{}\" visible=\"both\" length=\"{length}\" direction=\"{direction}\" rot=\"R{}\"/>",e(&pin_name(p)),num(p.at.0),num(p.at.1),num(p.angle))?;
            }
            s.push_str("<text x=\"0\" y=\"10\" size=\"1.27\" layer=\"95\">&gt;NAME</text><text x=\"0\" y=\"-10\" size=\"1.27\" layer=\"96\">&gt;VALUE</text></symbol>");
        }
        write!(s,"</symbols><devicesets><deviceset name=\"{id}\" prefix=\"{}\" uservalue=\"yes\"><description>{}</description><gates>",e(&self.prefix),e(&self.title))?;
        for i in 1..=self.units.len() {
            write!(
                s,
                "<gate name=\"G{i}\" symbol=\"{id}_{i}\" x=\"0\" y=\"0\"/>"
            )?;
        }
        write!(
            s,
            "</gates><devices><device name=\"\" package=\"{id}\"><connects>"
        )?;
        for (i, u) in self.units.iter().enumerate() {
            for p in &u.pins {
                write!(
                    s,
                    "<connect gate=\"G{}\" pin=\"{}\" pad=\"{}\"/>",
                    i + 1,
                    e(&pin_name(p)),
                    e(&p.number)
                )?;
            }
        }
        write!(s,"</connects><technologies><technology name=\"\"><attribute name=\"LCSC\" value=\"{id}\" constant=\"no\"/></technology></technologies></device></devices></deviceset></devicesets></library></drawing></eagle>\n")?;
        roxmltree::Document::parse_with_options(
            &s,
            roxmltree::ParsingOptions {
                allow_dtd: true,
                ..Default::default()
            },
        )?;
        Ok(s)
    }
}
fn pin_name(p: &Pin) -> String {
    format!("{}@{}", p.name.replace('@', "_"), p.number)
}
fn write_kicad_line(s: &mut String, l: &Line, symbol: bool) -> Result<()> {
    let mut pts = l.points.clone();
    if l.closed && !l.fill {
        if let Some(p) = pts.first().copied() {
            pts.push(p);
        }
    }
    if symbol || l.fill {
        let points = pts
            .iter()
            .map(|p| format!("(xy {} {})", num(p.0), num(if symbol { p.1 } else { -p.1 })))
            .collect::<Vec<_>>()
            .join(" ");
        writeln!(
            s,
            "({} (pts {points}) (stroke (width {}) (type {})) (fill {}) {})",
            if symbol { "polyline" } else { "fp_poly" },
            num(l.width),
            if symbol { "default" } else { "solid" },
            if symbol {
                if l.fill {
                    "(type background)"
                } else {
                    "(type none)"
                }
            } else if l.fill {
                "solid"
            } else {
                "none"
            },
            if symbol {
                String::new()
            } else {
                format!("(layer {})", q(&l.layer))
            }
        )?;
    } else {
        for p in pts.windows(2) {
            writeln!(
                s,
                "(fp_line (start {} {}) (end {} {}) (stroke (width {}) (type solid)) (layer {}))",
                num(p[0].0),
                num(-p[0].1),
                num(p[1].0),
                num(-p[1].1),
                num(l.width),
                q(&l.layer)
            )?;
        }
    }
    Ok(())
}
fn write_eagle_line(s: &mut String, l: &Line, symbol: bool) -> Result<()> {
    let layer = if symbol {
        94
    } else {
        match l.layer.as_str() {
            "F.Cu" => 1,
            "B.Cu" => 16,
            "F.SilkS" => 21,
            "B.SilkS" => 22,
            "F.Mask" => 29,
            "B.Mask" => 30,
            "F.Paste" => 31,
            "B.Paste" => 32,
            "Edge.Cuts" => 20,
            "F.Fab" => 51,
            "B.Fab" => 52,
            other => bail!("Unsupported Eagle layer {other}"),
        }
    };
    if l.fill {
        write!(s, "<polygon width=\"0\" layer=\"{layer}\">")?;
        for p in &l.points {
            write!(s, "<vertex x=\"{}\" y=\"{}\"/>", num(p.0), num(p.1))?;
        }
        s.push_str("</polygon>\n");
    } else {
        let mut pts = l.points.clone();
        if l.closed {
            if let Some(p) = pts.first().copied() {
                pts.push(p);
            }
        }
        for p in pts.windows(2) {
            writeln!(
                s,
                "<wire x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" width=\"{}\" layer=\"{layer}\"/>",
                num(p[0].0),
                num(p[0].1),
                num(p[1].0),
                num(p[1].1),
                num(l.width)
            )?;
        }
    }
    Ok(())
}
