use crate::diag::Diags;
use crate::geom::{self, P, Transform};
use crate::graphic::Bounds;
use crate::symbol::{Pin, PinType, Symbol};
use crate::units::Point;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BinaryHeap, HashMap, HashSet};

pub const GRID_MM: f64 = 1.27;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetStyle {
    #[default]
    Wire,
    Label,
    Power,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartFile {
    #[serde(rename = "ref")]
    pub reference: String,
    pub symbol: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub footprint: Option<String>,
    pub at: Point,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation: Option<f64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mirror: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<u32>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dnp: bool,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fields: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetFile {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<NetStyle>,
    pub pins: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub wires: Vec<Vec<Point>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchematicFile {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub board: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<PartFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nets: Vec<NetFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub no_connect: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sheets: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Part {
    pub reference: String,
    pub value: String,
    pub footprint: Option<String>,
    pub at: Point,
    pub rotation: f64,
    pub mirror: bool,
    pub unit: u32,
    pub dnp: bool,
    pub fields: BTreeMap<String, String>,
    #[serde(skip)]
    pub symbol: Symbol,
    pub symbol_name: String,
}

impl Part {
    pub fn transform(&self) -> Transform {
        Transform { at: self.at.to_mm(), rotation: self.rotation, mirror: self.mirror }
    }

    pub fn pins(&self) -> impl Iterator<Item = (usize, &Pin)> {
        self.symbol.pins.iter().enumerate().filter(|(_, p)| p.in_unit(self.unit))
    }

    pub fn pin_at(&self, pin: usize) -> P {
        self.transform().apply(self.symbol.pins[pin].at.to_mm())
    }

    pub fn pin_outward(&self, pin: usize) -> P {
        self.transform().direction(self.symbol.pins[pin].side.outward())
    }

    pub fn body_bounds(&self) -> Bounds {
        let t = self.transform();
        let mut b = Bounds::EMPTY;
        for g in self.symbol.graphics.iter().filter(|g| g.unit == 0 || g.unit == self.unit) {
            if matches!(g.shape, crate::graphic::Shape::Text { .. }) {
                continue;
            }
            let gb = g.bounds();
            if gb.is_empty() {
                continue;
            }
            for c in [gb.min, gb.max, [gb.min[0], gb.max[1]], [gb.max[0], gb.min[1]]] {
                b.add(t.apply(c));
            }
        }
        b
    }

    pub fn bounds(&self) -> Bounds {
        let mut b = self.body_bounds();
        for (i, _) in self.pins() {
            b.add(self.pin_at(i));
        }
        b
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct PinRef {
    pub part: usize,
    pub pin: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct Net {
    pub name: String,
    pub class: String,
    pub style: NetStyle,
    pub pins: Vec<PinRef>,
    pub wires: Vec<Vec<P>>,
    pub junctions: Vec<P>,
    pub drawn_by_hand: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Schematic {
    pub name: String,
    pub description: String,
    pub board: Option<String>,
    pub parts: Vec<Part>,
    pub nets: Vec<Net>,
    pub no_connect: Vec<PinRef>,
    pub sheets: Vec<SheetFrame>,
    pub parent: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SheetFrame {
    pub name: String,
    pub min: P,
    pub max: P,
}

pub struct Library<'a> {
    pub symbols: HashMap<&'a str, &'a Symbol>,
    pub footprints: HashMap<&'a str, &'a crate::footprint::Footprint>,
    pub netclasses: Option<Vec<String>>,
}

fn short(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

pub const SHEET_GAP_MM: f64 = 38.1;

impl SchematicFile {
    pub fn merge(
        &self,
        sheets: &[(&SchematicFile, Bounds)],
        d: &mut Diags,
    ) -> (SchematicFile, Vec<SheetFrame>) {
        let snap = |v: f64| (v / (2.0 * GRID_MM)).round() * 2.0 * GRID_MM;
        let own = self.parts.iter().fold(Bounds::EMPTY, |mut b, p| {
            b.add(p.at.to_mm());
            b
        });
        let mut cursor = if own.is_empty() { 0.0 } else { own.max[1] + SHEET_GAP_MM };
        let mut sources: Vec<(&SchematicFile, Point)> = vec![(self, Point::ZERO)];
        let mut frames = Vec::new();
        for (f, b) in sheets {
            if b.is_empty() {
                sources.push((f, Point::ZERO));
                continue;
            }
            let offset = Point::mm(snap(-b.min[0]), snap(cursor - b.min[1]));
            cursor += b.max[1] - b.min[1] + SHEET_GAP_MM;
            sources.push((f, offset));
            let [ox, oy] = offset.to_mm();
            let pad = 2.0 * GRID_MM * 2.0;
            frames.push(SheetFrame {
                name: f.name.clone(),
                min: [b.min[0] + ox - pad, b.min[1] + oy - pad],
                max: [b.max[0] + ox + pad, b.max[1] + oy + pad],
            });
        }

        let mut out = SchematicFile {
            name: self.name.clone(),
            description: self.description.clone(),
            board: self.board.clone(),
            parts: Vec::new(),
            nets: Vec::new(),
            no_connect: Vec::new(),
            sheets: Vec::new(),
        };
        let mut seen: HashMap<String, (usize, &str, usize)> = HashMap::new();
        for (f, offset) in &sources {
            for p in &f.parts {
                let mut p = p.clone();
                p.at = p.at + *offset;
                out.parts.push(p);
            }
            out.no_connect.extend(f.no_connect.iter().cloned());
            for n in &f.nets {
                let wires: Vec<Vec<Point>> =
                    n.wires.iter().map(|w| w.iter().map(|q| *q + *offset).collect()).collect();
                match seen.get_mut(&n.name) {
                    None => {
                        seen.insert(n.name.clone(), (out.nets.len(), f.name.as_str(), 1));
                        out.nets.push(NetFile { wires, ..n.clone() });
                    }
                    Some((i, first, count)) => {
                        let m = &mut out.nets[*i];
                        if let (Some(a), Some(b)) = (&m.class, &n.class)
                            && a != b
                        {
                            d.error(
                                format!("net {}", n.name),
                                format!("class {a} on sheet {first} but {b} on sheet {}", f.name),
                            );
                        }
                        if m.class.is_none() {
                            m.class = n.class.clone();
                        }
                        if n.style == Some(NetStyle::Power) {
                            m.style = Some(NetStyle::Power);
                        } else if m.style != Some(NetStyle::Power) {
                            m.style = Some(NetStyle::Label);
                        }
                        m.pins.extend(n.pins.iter().cloned());
                        m.wires.clear();
                        *count += 1;
                    }
                }
            }
        }
        (out, frames)
    }

    pub fn resolve(&self, lib: &Library, d: &mut Diags) -> Schematic {
        let mut parts = Vec::new();
        for (i, p) in self.parts.iter().enumerate() {
            let at = format!("parts[{i}] {}", p.reference);
            let Some(sym) = lib.symbols.get(short(&p.symbol)) else {
                d.error(&at, format!("symbol `{}` is not in this project", p.symbol));
                continue;
            };
            let rotation = p.rotation.unwrap_or(0.0);
            if rotation.rem_euclid(90.0) != 0.0 {
                d.error(&at, "schematic rotation must be 0, 90, 180 or 270");
            }
            let unit = p.unit.unwrap_or(1);
            if unit == 0 || unit > sym.units {
                d.error(&at, format!("`{}` has units 1 to {}", sym.name, sym.units));
            }
            let footprint = p
                .footprint
                .clone()
                .or_else(|| sym.footprint.clone())
                .map(|f| short(&f).to_string());
            parts.push(Part {
                reference: p.reference.clone(),
                value: p.value.clone().unwrap_or_else(|| sym.value.clone()),
                footprint,
                at: p.at,
                rotation: rotation.rem_euclid(360.0),
                mirror: p.mirror,
                unit,
                dnp: p.dnp,
                fields: p.fields.clone(),
                symbol: (*sym).clone(),
                symbol_name: sym.name.clone(),
            });
        }

        let find = |spec: &str, d: &mut Diags, at: &str| -> Option<PinRef> {
            let Some((r, pin)) = spec.rsplit_once('.') else {
                d.error(at, format!("`{spec}` should be REF.PIN, like U1.3"));
                return None;
            };
            let candidates: Vec<(usize, usize)> = parts
                .iter()
                .enumerate()
                .filter(|(_, p)| p.reference == r)
                .flat_map(|(pi, p)| p.pins().map(move |(ni, n)| (pi, ni, n)))
                .filter(|(_, _, n)| n.number == pin)
                .map(|(pi, ni, _)| (pi, ni))
                .collect();
            let by_name: Vec<(usize, usize)> = parts
                .iter()
                .enumerate()
                .filter(|(_, p)| p.reference == r)
                .flat_map(|(pi, p)| p.pins().map(move |(ni, n)| (pi, ni, n)))
                .filter(|(_, _, n)| n.name == pin)
                .map(|(pi, ni, _)| (pi, ni))
                .collect();
            match (candidates.first(), by_name.as_slice()) {
                (Some(&(part, pin)), _) => Some(PinRef { part, pin }),
                (None, [(part, pin)]) => Some(PinRef { part: *part, pin: *pin }),
                (None, []) if !parts.iter().any(|p| p.reference == r) => {
                    d.error(at, format!("no part `{r}`"));
                    None
                }
                (None, []) => {
                    d.error(at, format!("`{r}` has no pin `{pin}`"));
                    None
                }
                (None, _) => {
                    d.error(at, format!("`{r}` has several pins named `{pin}`, use the number"));
                    None
                }
            }
        };

        let mut nets = Vec::new();
        for (i, n) in self.nets.iter().enumerate() {
            let at = format!("nets[{i}] {}", n.name);
            let pins: Vec<PinRef> = n.pins.iter().filter_map(|s| find(s, d, &at)).collect();
            let class = n.class.clone().unwrap_or_else(|| "Default".into());
            if let Some(classes) = &lib.netclasses
                && !classes.contains(&class)
            {
                d.error(
                    &at,
                    format!(
                        "class `{class}` is not a netclass of the board ({})",
                        classes.join(", ")
                    ),
                );
            }
            nets.push(Net {
                name: n.name.clone(),
                class,
                style: n.style.unwrap_or_default(),
                pins,
                wires: n.wires.iter().map(|w| w.iter().map(|p| p.to_mm()).collect()).collect(),
                junctions: Vec::new(),
                drawn_by_hand: !n.wires.is_empty(),
            });
        }
        let no_connect = self.no_connect.iter().filter_map(|s| find(s, d, "no_connect")).collect();
        let mut s = Schematic {
            name: self.name.clone(),
            description: self.description.clone(),
            board: self.board.clone(),
            parts,
            nets,
            no_connect,
            sheets: Vec::new(),
            parent: None,
        };
        route(&mut s, d);
        for n in &mut s.nets {
            n.junctions = junctions(n, &s.parts);
        }
        s
    }
}

impl Schematic {
    pub fn net_of(&self, r: PinRef) -> Option<usize> {
        self.nets.iter().position(|n| n.pins.contains(&r))
    }

    pub fn pin_label(&self, r: PinRef) -> String {
        format!("{}.{}", self.parts[r.part].reference, self.parts[r.part].symbol.pins[r.pin].number)
    }

    pub fn bounds(&self) -> Bounds {
        let mut b = Bounds::EMPTY;
        for p in &self.parts {
            b.union(&p.bounds());
        }
        for n in &self.nets {
            n.wires.iter().flatten().for_each(|q| b.add(*q));
        }
        b
    }

    pub fn references(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.parts.iter().map(|p| p.reference.as_str()).collect();
        v.sort_by(|a, b| crate::footprint::natural_cmp(a, b));
        v.dedup();
        v
    }

    pub fn check(&self, lib: &Library, d: &mut Diags) {
        self.check_as(lib, d, false);
    }

    pub fn check_as(&self, lib: &Library, d: &mut Diags, sheet: bool) {
        let mut seen: HashMap<(&str, u32), usize> = HashMap::new();
        for (i, p) in self.parts.iter().enumerate() {
            let at = format!("part {}", p.reference);
            if let Some(j) = seen.insert((&p.reference, p.unit), i) {
                d.error(
                    &at,
                    format!("reference {} unit {} is used by parts[{j}] too", p.reference, p.unit),
                );
            }
            let grid = (GRID_MM * 1e6).round() as i64;
            if p.at.0.0 % grid != 0 || p.at.1.0 % grid != 0 {
                d.warn(
                    &at,
                    format!(
                        "placed at {}, off the 1.27 mm grid so its pins miss the wire grid",
                        p.at
                    ),
                );
            }
            match &p.footprint {
                None => d.warn(&at, "no footprint, the layout cannot place it"),
                Some(f) => match lib.footprints.get(f.as_str()) {
                    None => d.error(&at, format!("footprint `{f}` is not in this project")),
                    Some(fp) => {
                        let pads = fp.pad_numbers();
                        let missing: Vec<&str> = p
                            .symbol
                            .pins
                            .iter()
                            .map(|n| n.number.as_str())
                            .filter(|n| !pads.contains(n))
                            .collect();
                        if !missing.is_empty() {
                            d.error(
                                &at,
                                format!("pins {} have no pad on `{f}`", missing.join(", ")),
                            );
                        }
                    }
                },
            }
            for (j, q) in self.parts.iter().enumerate().skip(i + 1) {
                if p.body_bounds().overlaps(&q.body_bounds()) {
                    d.warn(&at, format!("body overlaps {} (parts[{j}])", q.reference));
                }
            }
        }

        let mut owner: HashMap<PinRef, usize> = HashMap::new();
        for (i, n) in self.nets.iter().enumerate() {
            let at = format!("net {}", n.name);
            if self.nets.iter().filter(|m| m.name == n.name).count() > 1 {
                d.error(&at, "net name is used twice, merge the pin lists");
            }
            if n.pins.len() == 1 && !sheet {
                d.warn(
                    &at,
                    format!("only one pin ({}), nothing to connect", self.pin_label(n.pins[0])),
                );
            }
            if n.class == "Default" && !sheet {
                let others: Vec<&str> = lib
                    .netclasses
                    .iter()
                    .flatten()
                    .map(String::as_str)
                    .filter(|c| *c != "Default")
                    .collect();
                let choose = if others.is_empty() {
                    "add a netclass for it to the board".to_string()
                } else {
                    format!("set `class` to one of {} or add one", others.join(", "))
                };
                d.warn(&at, format!("in the Default netclass, {choose}"));
            }
            for r in &n.pins {
                if let Some(j) = owner.insert(*r, i)
                    && j != i
                {
                    d.error(
                        &at,
                        format!("{} is also in net {}", self.pin_label(*r), self.nets[j].name),
                    );
                }
            }
            let drivers: Vec<String> = n
                .pins
                .iter()
                .filter(|r| {
                    matches!(
                        self.parts[r.part].symbol.pins[r.pin].kind,
                        PinType::Output | PinType::PowerOut
                    )
                })
                .map(|r| self.pin_label(*r))
                .collect();
            if drivers.len() > 1 {
                d.warn(&at, format!("several outputs drive it: {}", drivers.join(", ")));
            }
            if n.style == NetStyle::Wire && (n.drawn_by_hand || !n.wires.is_empty()) {
                self.check_wires(n, d);
            }
        }
        for r in &self.no_connect {
            if owner.contains_key(r) {
                d.error(
                    "no_connect",
                    format!("{} is marked no-connect but is in a net", self.pin_label(*r)),
                );
            }
        }
        for (pi, p) in self.parts.iter().enumerate() {
            let open: Vec<String> = p
                .pins()
                .filter(|(ni, pin)| {
                    let r = PinRef { part: pi, pin: *ni };
                    !owner.contains_key(&r)
                        && !self.no_connect.contains(&r)
                        && pin.kind != PinType::NoConnect
                        && !pin.hidden
                })
                .map(|(_, pin)| pin.number.clone())
                .collect();
            if !open.is_empty() {
                d.warn(
                    format!("part {}", p.reference),
                    format!(
                        "pins {} are not in any net, add them to a net or to `no_connect`",
                        open.join(", ")
                    ),
                );
            }
        }
    }

    fn check_wires(&self, n: &Net, d: &mut Diags) {
        let at = format!("net {}", n.name);
        let mut tips: Vec<(PinRef, P)> = Vec::new();
        for (pi, p) in self.parts.iter().enumerate() {
            for (ni, _) in p.pins() {
                tips.push((PinRef { part: pi, pin: ni }, p.pin_at(ni)));
            }
        }
        let segs: Vec<(P, P)> =
            n.wires.iter().flat_map(|w| w.windows(2).map(|s| (s[0], s[1]))).collect();
        let on = |p: P, s: &(P, P)| geom::point_segment_distance(p, s.0, s.1) < 1e-4;
        let mut uf = UnionFind::new(segs.len() + n.pins.len());
        for i in 0..segs.len() {
            for j in i + 1..segs.len() {
                let (a, b) = (&segs[i], &segs[j]);
                if on(a.0, b) || on(a.1, b) || on(b.0, a) || on(b.1, a) {
                    uf.union(i, j);
                }
            }
        }
        for (k, r) in n.pins.iter().enumerate() {
            let tip = self.parts[r.part].pin_at(r.pin);
            for (i, s) in segs.iter().enumerate() {
                if on(tip, s) {
                    uf.union(i, segs.len() + k);
                }
            }
        }
        for (r, tip) in &tips {
            if n.pins.contains(r) {
                continue;
            }
            if segs.iter().any(|s| on(*tip, s)) {
                d.error(
                    &at,
                    format!("a wire touches {}, which is not in this net", self.pin_label(*r)),
                );
            }
        }
        if n.pins.is_empty() {
            return;
        }
        let root = uf.find(segs.len());
        let loose: Vec<String> = n
            .pins
            .iter()
            .enumerate()
            .filter(|(k, _)| uf.find(segs.len() + k) != root)
            .map(|(_, r)| self.pin_label(*r))
            .collect();
        if !loose.is_empty() {
            d.error(&at, format!("wires do not reach {}", loose.join(", ")));
        }
    }
}

pub struct UnionFind {
    parent: Vec<usize>,
}

impl UnionFind {
    pub fn new(n: usize) -> Self {
        UnionFind { parent: (0..n).collect() }
    }

    pub fn find(&mut self, mut i: usize) -> usize {
        while self.parent[i] != i {
            self.parent[i] = self.parent[self.parent[i]];
            i = self.parent[i];
        }
        i
    }

    pub fn union(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            self.parent[a] = b;
        }
    }
}

type Cell = (i32, i32);

fn cell(p: P) -> Cell {
    ((p[0] / GRID_MM).round() as i32, (p[1] / GRID_MM).round() as i32)
}

fn point(c: Cell) -> P {
    [c.0 as f64 * GRID_MM, c.1 as f64 * GRID_MM]
}

const DIRS: [Cell; 4] = [(1, 0), (0, 1), (-1, 0), (0, -1)];

#[derive(Default)]
struct Grid {
    blocked: HashSet<Cell>,
    used: HashMap<Cell, (usize, u8)>,
    min: Cell,
    max: Cell,
}

impl Grid {
    fn mark(&mut self, net: usize, path: &[Cell]) {
        for w in path.windows(2) {
            let horizontal = w[0].1 == w[1].1;
            let axis = if horizontal { 1 } else { 2 };
            for c in [w[0], w[1]] {
                let e = self.used.entry(c).or_insert((net, 0));
                if e.0 == net {
                    e.1 |= axis;
                } else {
                    e.1 |= 4;
                }
            }
        }
    }
}

#[derive(PartialEq, Eq)]
struct Node {
    cost: u32,
    cell: Cell,
    dir: u8,
}

impl Ord for Node {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        o.cost.cmp(&self.cost).then_with(|| (self.cell, self.dir).cmp(&(o.cell, o.dir)))
    }
}

impl PartialOrd for Node {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}

fn search(
    g: &Grid,
    net: usize,
    start: Cell,
    out: Cell,
    targets: &HashSet<Cell>,
) -> Option<Vec<Cell>> {
    let start_dir = DIRS.iter().position(|d| *d == out).unwrap_or(0) as u8;
    let mut best: HashMap<(Cell, u8), u32> = HashMap::new();
    let mut prev: HashMap<(Cell, u8), (Cell, u8)> = HashMap::new();
    let mut heap = BinaryHeap::new();
    for (nd, d) in DIRS.iter().enumerate() {
        let nd = nd as u8;
        let first = (start.0 + d.0, start.1 + d.1);
        if targets.contains(&first) && nd == start_dir {
            return Some(vec![start, first]);
        }
        if g.blocked.contains(&first) && !targets.contains(&first) {
            continue;
        }
        if g.used.get(&first).is_some_and(|u| u.0 != net) {
            continue;
        }
        let cost = if nd == start_dir { 10 } else { 50 };
        best.insert((first, nd), cost);
        prev.insert((first, nd), (start, nd));
        heap.push(Node { cost, cell: first, dir: nd });
    }
    while let Some(Node { cost, cell: c, dir }) = heap.pop() {
        if best.get(&(c, dir)).is_some_and(|b| *b < cost) {
            continue;
        }
        if targets.contains(&c) {
            let mut path = vec![c];
            let mut k = (c, dir);
            while let Some(&p) = prev.get(&k) {
                path.push(p.0);
                if p.0 == start {
                    break;
                }
                k = p;
            }
            path.reverse();
            return Some(path);
        }
        for (nd, d) in DIRS.iter().enumerate() {
            let nd = nd as u8;
            if (nd + 2) % 4 == dir {
                continue;
            }
            let n = (c.0 + d.0, c.1 + d.1);
            if n.0 < g.min.0 || n.1 < g.min.1 || n.0 > g.max.0 || n.1 > g.max.1 {
                continue;
            }
            if g.blocked.contains(&n) && !targets.contains(&n) {
                continue;
            }
            let mut step = 10u32;
            if nd != dir {
                if g.used.get(&c).is_some_and(|u| u.0 != net) {
                    continue;
                }
                step += 40;
            }
            if let Some(&(owner, axes)) = g.used.get(&n)
                && owner != net
            {
                let along = if d.1 == 0 { 1 } else { 2 };
                if axes & along != 0 || axes & 4 != 0 || targets.contains(&n) {
                    continue;
                }
                step += 60;
            }
            let nc = cost + step;
            if best.get(&(n, nd)).is_none_or(|b| nc < *b) {
                best.insert((n, nd), nc);
                prev.insert((n, nd), (c, dir));
                heap.push(Node { cost: nc, cell: n, dir: nd });
            }
        }
    }
    None
}

fn simplify(path: &[Cell]) -> Vec<P> {
    let mut out: Vec<Cell> = Vec::new();
    for &c in path {
        if out.len() >= 2 {
            let (a, b) = (out[out.len() - 2], out[out.len() - 1]);
            if (a.0 == b.0 && b.0 == c.0) || (a.1 == b.1 && b.1 == c.1) {
                out.pop();
            }
        }
        out.push(c);
    }
    out.into_iter().map(point).collect()
}

fn route(s: &mut Schematic, d: &mut Diags) {
    let mut g = Grid::default();
    let mut all = Bounds::EMPTY;
    for p in s.parts.iter() {
        all.union(&p.bounds());
        let body = p.body_bounds();
        if !body.is_empty() {
            let (a, b) = (cell(body.min), cell(body.max));
            for x in a.0..=b.0 {
                for y in a.1..=b.1 {
                    g.blocked.insert((x, y));
                }
            }
        }
        for (ni, _) in p.pins() {
            let tip = cell(p.pin_at(ni));
            let end = cell(p.transform().apply(p.symbol.pins[ni].body_end().to_mm()));
            let (dx, dy) = ((end.0 - tip.0).signum(), (end.1 - tip.1).signum());
            let mut c = tip;
            while c != end {
                c = (c.0 + dx, c.1 + dy);
                g.blocked.insert(c);
            }
            g.blocked.insert(tip);
        }
    }
    if all.is_empty() {
        return;
    }
    let margin = 12;
    let (lo, hi) = (cell(all.min), cell(all.max));
    g.min = (lo.0 - margin, lo.1 - margin);
    g.max = (hi.0 + margin, hi.1 + margin);

    for (ni, n) in s.nets.iter().enumerate() {
        if n.drawn_by_hand {
            let cells: Vec<Vec<Cell>> =
                n.wires.iter().map(|w| w.iter().map(|p| cell(*p)).collect()).collect();
            for w in cells {
                let full = expand(&w);
                g.mark(ni, &full);
            }
        }
        if n.style != NetStyle::Wire {
            for r in &n.pins {
                let p = &s.parts[r.part];
                let tip = cell(p.pin_at(r.pin));
                let o = p.pin_outward(r.pin);
                let o = (o[0].round() as i32, o[1].round() as i32);
                for k in 1..=3 {
                    g.blocked.insert((tip.0 + o.0 * k, tip.1 + o.1 * k));
                }
            }
        }
    }

    for ni in 0..s.nets.len() {
        let n = &s.nets[ni];
        if n.style != NetStyle::Wire || n.drawn_by_hand || n.pins.len() < 2 {
            continue;
        }
        let pins: Vec<(Cell, Cell)> = n
            .pins
            .iter()
            .map(|r| {
                let p = &s.parts[r.part];
                let o = p.pin_outward(r.pin);
                (cell(p.pin_at(r.pin)), (o[0].round() as i32, o[1].round() as i32))
            })
            .collect();
        let mut tree: HashSet<Cell> = HashSet::from([pins[0].0]);
        let mut done = vec![false; pins.len()];
        done[0] = true;
        let mut wires = Vec::new();
        let mut failed = false;
        for _ in 1..pins.len() {
            let next = (0..pins.len())
                .filter(|i| !done[*i])
                .min_by_key(|i| {
                    tree.iter()
                        .map(|t| (t.0 - pins[*i].0.0).abs() + (t.1 - pins[*i].0.1).abs())
                        .min()
                        .unwrap_or(0)
                })
                .unwrap();
            done[next] = true;
            let (start, out) = pins[next];
            let found = search(&g, ni, start, out, &tree);
            match found {
                Some(path) => {
                    g.mark(ni, &path);
                    tree.extend(path.iter().copied());
                    wires.push(simplify(&path));
                }
                None => {
                    failed = true;
                    break;
                }
            }
        }
        let net = &mut s.nets[ni];
        if failed {
            d.info(
                format!("net {}", net.name),
                "no clear path for a wire, drawn with labels instead",
            );
            net.style = NetStyle::Label;
            net.wires.clear();
        } else {
            net.wires = wires;
        }
    }
}

fn expand(w: &[Cell]) -> Vec<Cell> {
    let mut out = Vec::new();
    for s in w.windows(2) {
        let (dx, dy) = ((s[1].0 - s[0].0).signum(), (s[1].1 - s[0].1).signum());
        let mut c = s[0];
        out.push(c);
        while c != s[1] {
            c = (c.0 + dx, c.1 + dy);
            out.push(c);
        }
    }
    out
}

fn junctions(n: &Net, parts: &[Part]) -> Vec<P> {
    if n.style != NetStyle::Wire {
        return Vec::new();
    }
    let segs: Vec<(P, P)> =
        n.wires.iter().flat_map(|w| w.windows(2).map(|s| (s[0], s[1]))).collect();
    let tips: Vec<P> = n.pins.iter().map(|r| parts[r.part].pin_at(r.pin)).collect();
    let mut points: Vec<P> = segs.iter().flat_map(|s| [s.0, s.1]).collect();
    points.sort_by(|a, b| a.partial_cmp(b).unwrap());
    points.dedup_by(|a, b| geom::dist(*a, *b) < 1e-6);
    points
        .into_iter()
        .filter(|p| {
            let ends = segs
                .iter()
                .filter(|s| geom::dist(s.0, *p) < 1e-6 || geom::dist(s.1, *p) < 1e-6)
                .count();
            let through = segs
                .iter()
                .filter(|s| {
                    geom::dist(s.0, *p) > 1e-6
                        && geom::dist(s.1, *p) > 1e-6
                        && geom::point_segment_distance(*p, s.0, s.1) < 1e-6
                })
                .count();
            let tip = tips.iter().any(|t| geom::dist(*t, *p) < 1e-6) as usize;
            ends + 2 * through + tip >= 3
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbol::SymbolFile;

    fn resistor() -> Symbol {
        let f: SymbolFile = toml::from_str(
            r#"
name = "R"
reference = "R"
[[graphics]]
kind = "rect"
start = [-1.016, -2.54]
end = [1.016, 2.54]
[[pins]]
number = "1"
at = [0, -3.81]
side = "top"
length = 1.27
[[pins]]
number = "2"
at = [0, 3.81]
side = "bottom"
length = 1.27
"#,
        )
        .unwrap();
        f.resolve(&mut Diags::new("R"))
    }

    #[test]
    fn auto_wires_join_every_pin_without_touching_others() {
        let r = resistor();
        let lib = Library {
            symbols: HashMap::from([("R", &r)]),
            footprints: HashMap::new(),
            netclasses: None,
        };
        let f: SchematicFile = toml::from_str(
            r#"
name = "divider"
[[parts]]
ref = "R1"
symbol = "R"
at = [10.16, 10.16]
[[parts]]
ref = "R2"
symbol = "R"
at = [20.32, 20.32]
[[parts]]
ref = "R3"
symbol = "R"
at = [30.48, 10.16]
[[nets]]
name = "MID"
pins = ["R1.2", "R2.1", "R3.2"]
[[nets]]
name = "TOP"
pins = ["R1.1", "R3.1"]
[[nets]]
name = "GND"
style = "power"
pins = ["R2.2"]
"#,
        )
        .unwrap();
        let mut d = Diags::new("divider");
        let s = f.resolve(&lib, &mut d);
        s.check(&lib, &mut d);
        let errors: Vec<_> =
            d.list.iter().filter(|x| x.severity == crate::diag::Severity::Error).collect();
        assert!(errors.is_empty(), "{errors:?}");
        assert!(!s.nets[0].wires.is_empty() && !s.nets[1].wires.is_empty());
        assert_eq!(s.nets[0].junctions.len(), 1, "{:?}", s.nets[0].wires);
    }

    #[test]
    fn sheets_join_nets_by_name_and_stack_below_each_other() {
        let r = resistor();
        let lib = Library {
            symbols: HashMap::from([("R", &r)]),
            footprints: HashMap::new(),
            netclasses: None,
        };
        let sheet = |name: &str, part: &str, class: &str| -> SchematicFile {
            toml::from_str(&format!(
                r#"
name = "{name}"
[[parts]]
ref = "{part}"
symbol = "R"
at = [10.16, 10.16]
[[nets]]
name = "MID"
class = "{class}"
pins = ["{part}.1"]
[[nets]]
name = "GND"
class = "Ground"
style = "power"
pins = ["{part}.2"]
"#
            ))
            .unwrap()
        };
        let a = sheet("a", "R1", "Signal");
        let b = sheet("b", "R2", "Signal");
        let top: SchematicFile = toml::from_str("name = \"top\"\nsheets = [\"a\", \"b\"]").unwrap();
        let mut d = Diags::new("a");
        let sa = a.resolve(&lib, &mut d);
        sa.check_as(&lib, &mut d, true);
        let noise = |d: &Diags| d.list.iter().filter(|x| !x.message.contains("footprint")).count();
        assert_eq!(noise(&d), 0, "{:?}", d.list);
        let bounds = |f: &SchematicFile| f.resolve(&lib, &mut Diags::new("x")).bounds();
        let mut d = Diags::new("top");
        let (whole, frames) = top.merge(&[(&a, bounds(&a)), (&b, bounds(&b))], &mut d);
        assert_eq!(frames.len(), 2);
        assert!(frames[1].min[1] > frames[0].max[1]);
        let whole = whole.resolve(&lib, &mut d);
        whole.check(&lib, &mut d);
        assert_eq!(noise(&d), 0, "{:?}", d.list);
        assert_eq!(whole.nets.len(), 2);
        assert_eq!(whole.nets[0].pins.len(), 2);
        assert_eq!(whole.nets[0].style, NetStyle::Label);
        assert!(whole.parts[1].at.to_mm()[1] > whole.parts[0].at.to_mm()[1] + SHEET_GAP_MM);

        let c = sheet("c", "R3", "Power");
        let mut d = Diags::new("top");
        top.merge(&[(&a, bounds(&a)), (&c, bounds(&c))], &mut d);
        assert!(d.list.iter().any(|x| x.message.contains("class Signal on sheet a but Power")));
    }

    #[test]
    fn a_net_left_in_default_is_a_warning_on_the_whole_design_only() {
        let r = resistor();
        let classes = vec!["Default".to_string(), "Signal".to_string()];
        let lib = Library {
            symbols: HashMap::from([("R", &r)]),
            footprints: HashMap::new(),
            netclasses: Some(classes),
        };
        let file: SchematicFile = toml::from_str(
            "name = \"s\"\n[[parts]]\nref = \"R1\"\nsymbol = \"R\"\nat = [10.16, 10.16]\n\
             [[nets]]\nname = \"A\"\npins = [\"R1.1\"]\n\
             [[nets]]\nname = \"B\"\nclass = \"Default\"\npins = [\"R1.2\"]\n",
        )
        .unwrap();
        let s = file.resolve(&lib, &mut Diags::new("s"));
        let default_warnings =
            |d: &Diags| d.list.iter().filter(|x| x.message.contains("Default netclass")).count();
        let mut d = Diags::new("s");
        s.check_as(&lib, &mut d, false);
        assert_eq!(default_warnings(&d), 2, "{:?}", d.list);
        assert!(d.list.iter().any(|x| x.message.contains("one of Signal")));
        let mut d = Diags::new("s");
        s.check_as(&lib, &mut d, true);
        assert_eq!(default_warnings(&d), 0);
    }

    #[test]
    fn a_hand_wire_that_misses_a_pin_is_reported() {
        let r = resistor();
        let lib = Library {
            symbols: HashMap::from([("R", &r)]),
            footprints: HashMap::new(),
            netclasses: None,
        };
        let f: SchematicFile = toml::from_str(
            r#"
name = "x"
[[parts]]
ref = "R1"
symbol = "R"
at = [10.16, 10.16]
[[parts]]
ref = "R2"
symbol = "R"
at = [20.32, 10.16]
[[nets]]
name = "A"
pins = ["R1.1", "R2.1"]
wires = [[[10.16, 6.35], [10.16, 3.81], [19.05, 3.81]]]
"#,
        )
        .unwrap();
        let mut d = Diags::new("x");
        let s = f.resolve(&lib, &mut d);
        s.check(&lib, &mut d);
        assert!(d.list.iter().any(|x| x.message.contains("do not reach R2.1")), "{:?}", d.list);
    }
}
