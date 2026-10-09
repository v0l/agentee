use agentee_core::Project;
use agentee_core::layout::{LayoutFile, Placed};
use agentee_core::place::{self, PlaceInput, PlaceOptions};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

type P = [f64; 2];

const SEEDS: [u64; 10] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data").join(name)
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &to.join(e.file_name()));
        } else {
            std::fs::copy(e.path(), to.join(e.file_name())).unwrap();
        }
    }
}

struct Laid {
    parts: Vec<Placed>,
    nets: Vec<String>,
    outline: Vec<P>,
    shadows: Vec<String>,
    sheets: HashMap<String, String>,
}

fn placed(name: &str, seed: u64) -> Vec<Placed> {
    laid(name, seed).parts
}

fn sheets_of(dir: &Path) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for e in std::fs::read_dir(dir).unwrap() {
        let path = e.unwrap().path();
        let file = path.file_name().unwrap().to_string_lossy().to_string();
        let Some(sheet) = file.strip_suffix(".sch.toml") else { continue };
        for line in std::fs::read_to_string(&path).unwrap().lines() {
            if let Some(r) = line.strip_prefix("ref = \"").and_then(|r| r.strip_suffix('"')) {
                out.insert(r.to_string(), sheet.to_string());
            }
        }
    }
    out
}

fn laid(name: &str, seed: u64) -> Laid {
    let dir =
        std::env::temp_dir().join(format!("agentee-rules-{name}-{seed}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    copy_dir(&fixture(name), &dir);
    let p = Project::load(&dir).unwrap();
    let entry = &p.layouts[0];
    let layout = &entry.item;
    let board = &p.boards[0].item;
    let sch = &p.schematics.iter().find(|s| s.name == layout.schematic).unwrap().item;
    let file: LayoutFile =
        agentee_core::project::parse(&std::fs::read_to_string(&entry.path).unwrap()).unwrap();
    let footprints: HashMap<&str, &agentee_core::footprint::Footprint> =
        p.footprints.iter().map(|e| (e.name.as_str(), &e.item)).collect();
    let spec = file.place.clone().unwrap_or_default();
    let mut fast: Vec<String> = file.interfaces.iter().flat_map(|f| f.nets.clone()).collect();
    for pr in &file.pairs {
        fast.push(pr.p.clone());
        fast.push(pr.n.clone());
    }
    let input = PlaceInput {
        board,
        outline: &layout.outline,
        cutouts: &layout.board_cutouts,
        schematic: sch,
        footprints: &footprints,
        placements: &file.footprints,
        spec: &spec,
        fast_nets: fast,
        heat: Vec::new(),
        silk: place::board_silk(&layout.graphics, &layout.artwork),
        texts: place::movable_texts(&layout.graphics),
        tracks: &file.tracks,
        vias: &file.vias,
        budgets: place::budgets(&file.interfaces),
    };
    let r = place::place(&input, &PlaceOptions { seed, ..Default::default() }).unwrap();
    assert!(r.failed.is_empty(), "seed {seed}: {:?}", r.failed);
    assert_eq!(r.after.overlaps, 0, "seed {seed}");
    let mut text = format!("name = \"{name}\"\nboard = \"{name}\"\nschematic = \"{name}\"\n");
    for pm in &r.placements {
        text += &format!(
            "\n[[footprints]]\nref = \"{}\"\nat = [{}, {}]\nrotation = {}\n{}",
            pm.reference,
            pm.at[0],
            pm.at[1],
            pm.rotation,
            if pm.bottom { "side = \"bottom\"\n" } else { "" }
        );
    }
    std::fs::write(&entry.path, text).unwrap();
    let after = Project::load(&dir).unwrap();
    let layout = &after.layouts[0];
    let out = Laid {
        parts: layout.item.parts.clone(),
        nets: layout.item.nets.iter().map(|n| n.name.clone()).collect(),
        outline: layout.item.outline.clone(),
        shadows: layout
            .diags
            .iter()
            .filter(|d| d.rule.as_deref() == Some("tall-part-shadow"))
            .map(|d| d.at.trim_start_matches("part ").to_string())
            .collect(),
        sheets: sheets_of(&dir),
    };
    if let Ok(keep) = std::env::var("AGENTEE_PLACE_KEEP") {
        let to = Path::new(&keep).join(format!("{name}-{seed}"));
        let _ = std::fs::remove_dir_all(&to);
        copy_dir(&dir, &to);
    }
    let _ = std::fs::remove_dir_all(&dir);
    out
}

struct Board {
    parts: Vec<Placed>,
    nets: Vec<String>,
    outline: Vec<P>,
    shadows: Vec<String>,
    sheets: HashMap<String, String>,
    broken: Vec<String>,
}

fn seg_dist(p: P, a: P, b: P) -> f64 {
    let d = [b[0] - a[0], b[1] - a[1]];
    let l = d[0] * d[0] + d[1] * d[1];
    let t = if l == 0.0 {
        0.0
    } else {
        (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / l).clamp(0.0, 1.0)
    };
    ((p[0] - a[0] - t * d[0]).powi(2) + (p[1] - a[1] - t * d[1]).powi(2)).sqrt()
}

fn ring_gap(a: &[P], b: &[P]) -> f64 {
    let mut best = f64::MAX;
    for (x, y) in [(a, b), (b, a)] {
        for &p in x {
            for k in 0..y.len() {
                best = best.min(seg_dist(p, y[k], y[(k + 1) % y.len()]));
            }
        }
    }
    best
}

impl Board {
    fn part(&self, r: &str) -> &Placed {
        self.parts.iter().find(|p| p.reference == r).unwrap_or_else(|| panic!("no part {r}"))
    }

    fn pad(&self, pin: &str) -> &[P] {
        let (r, n) = pin.split_once('.').unwrap();
        let pad = self.part(r).pads.iter().find(|q| q.number == n);
        &pad.unwrap_or_else(|| panic!("no pad {pin}")).outlines[0]
    }

    fn centre(&self, r: &str) -> P {
        self.part(r).at.to_mm()
    }

    fn gap(&self, a: &str, b: &str) -> f64 {
        ring_gap(self.pad(a), self.pad(b))
    }

    fn apart(&self, a: &str, b: &str) -> f64 {
        let (p, q) = (self.centre(a), self.centre(b));
        ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt()
    }

    fn pad_near(&mut self, why: &str, pin: &str, any_of: &[&str], mm: f64) {
        let g = any_of.iter().map(|o| self.gap(pin, o)).fold(f64::MAX, f64::min);
        if g > mm {
            self.broken.push(format!("{why}: {pin} is {g:.1} mm from {any_of:?}, at most {mm}"));
        }
    }

    fn centres_within(&mut self, why: &str, a: &str, b: &str, mm: f64) {
        let d = self.apart(a, b);
        if d > mm {
            self.broken.push(format!("{why}: {a} is {d:.1} mm from {b}, at most {mm}"));
        }
    }

    fn apart_at_least(&mut self, why: &str, a: &str, b: &str, mm: f64) {
        let d = self.apart(a, b);
        if d < mm {
            self.broken.push(format!("{why}: {a} is {d:.1} mm from {b}, at least {mm}"));
        }
    }

    fn pads_apart(&mut self, why: &str, a: &str, b: &str, mm: f64) {
        let (pa, pb) = (self.part(a), self.part(b));
        let mut g = f64::MAX;
        for x in &pa.pads {
            for y in &pb.pads {
                if let (Some(r), Some(t)) = (x.outlines.first(), y.outlines.first()) {
                    g = g.min(ring_gap(r, t));
                }
            }
        }
        if g < mm {
            self.broken.push(format!("{why}: {a} pads are {g:.1} mm from {b}, at least {mm}"));
        }
    }

    fn by_edge(&mut self, why: &str, r: &str, mm: f64) {
        let o = self.outline.clone();
        let mut g = f64::MAX;
        for pad in &self.part(r).pads {
            for ring in &pad.outlines {
                for &p in ring {
                    for k in 0..o.len() {
                        g = g.min(seg_dist(p, o[k], o[(k + 1) % o.len()]));
                    }
                }
            }
        }
        if g > mm {
            self.broken.push(format!("{why}: {r} is {g:.1} mm from the board edge, at most {mm}"));
        }
    }

    fn net_span(&self, net: &str) -> f64 {
        let n = self.nets.iter().position(|x| x == net).unwrap_or_else(|| panic!("no net {net}"));
        let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
        for p in &self.parts {
            for q in p.pads.iter().filter(|q| q.net == Some(n)) {
                for v in q.outlines.iter().flatten() {
                    lo = [lo[0].min(v[0]), lo[1].min(v[1])];
                    hi = [hi[0].max(v[0]), hi[1].max(v[1])];
                }
            }
        }
        (hi[0] - lo[0]) + (hi[1] - lo[1])
    }

    fn within_budget(&mut self, why: &str, net: &str, mm: f64) {
        let l = 1.1 * self.net_span(net);
        if l > mm {
            self.broken.push(format!("{why}: {net} spans {l:.1} mm with detours, at most {mm}"));
        }
    }

    fn sheets_hold(&mut self, share: f64) {
        let mut centre: HashMap<String, (P, usize)> = HashMap::new();
        let skip = |r: &str| r.starts_with('H');
        for p in self.parts.iter().filter(|p| !skip(&p.reference)) {
            let Some(s) = self.sheets.get(&p.reference) else { continue };
            let e = centre.entry(s.clone()).or_insert(([0.0, 0.0], 0));
            let a = p.at.to_mm();
            e.0 = [e.0[0] + a[0], e.0[1] + a[1]];
            e.1 += 1;
        }
        let centre: HashMap<String, P> =
            centre.into_iter().map(|(k, (c, n))| (k, [c[0] / n as f64, c[1] / n as f64])).collect();
        let mut per: HashMap<String, (usize, usize)> = HashMap::new();
        for p in self.parts.iter().filter(|p| !skip(&p.reference)) {
            let Some(s) = self.sheets.get(&p.reference) else { continue };
            let a = p.at.to_mm();
            let d = |c: &P| (a[0] - c[0]).hypot(a[1] - c[1]);
            let own = d(&centre[s]);
            let home = centre.iter().all(|(k, c)| k == s || d(c) >= own);
            let e = per.entry(s.clone()).or_default();
            e.0 += home as usize;
            e.1 += 1;
        }
        for (s, (home, all)) in per {
            if all >= 6 && (home as f64) < share * all as f64 {
                self.broken.push(format!(
                    "parts stay with their sheet: {home} of {all} parts of {s} are nearest its own centre, at least {:.0}%",
                    share * 100.0
                ));
            }
        }
    }

    fn in_order(&mut self, why: &str, from: &str, chain: &[&str]) {
        let d: Vec<f64> = chain.iter().map(|r| self.apart(from, r)).collect();
        if d.windows(2).any(|w| w[0] >= w[1]) {
            let got: Vec<String> =
                chain.iter().zip(&d).map(|(r, d)| format!("{r} {d:.1}")).collect();
            self.broken.push(format!("{why}: from {from} the chain runs {}", got.join(", ")));
        }
    }

    fn closer(&mut self, why: &str, a: &str, near: &str, far: &str) {
        let (n, f) = (self.apart(a, near), self.apart(a, far));
        if n >= f {
            self.broken
                .push(format!("{why}: {a} is {n:.1} mm from {near} but {f:.1} mm from {far}"));
        }
    }
}

fn stepper_rules(b: &mut Board) {
    for (u, j, other, c, r) in [("U3", "J2", "J3", 30, 30), ("U4", "J3", "J2", 40, 40)] {
        let pin = |n: u32| format!("{u}.{n}");
        let cap = |k: u32, n: u32| format!("C{}.{n}", c + k);
        let vs = [pin(22), pin(28)];
        let vs: Vec<&str> = vs.iter().map(|s| s.as_str()).collect();
        let why = format!("{u} datasheet 21, filter caps close to the pins");
        b.pad_near(&why, &cap(1, 1), &[&pin(4)], 5.0);
        b.pad_near(&why, &cap(1, 2), &[&pin(5)], 5.0);
        b.pad_near(&why, &cap(2, 1), &[&pin(6)], 5.0);
        b.pad_near(&why, &cap(3, 1), &[&pin(8)], 5.0);
        b.pad_near(&why, &cap(0, 1), &vs, 5.0);
        b.pad_near(&why, &cap(4, 1), &[&pin(15)], 5.0);
        let why = format!("{u} datasheet 21.2, sense resistors close to BRA and BRB");
        b.pad_near(&why, &format!("R{r}.1"), &[&pin(23)], 8.0);
        b.pad_near(&why, &format!("R{}.1", r + 1), &[&pin(27)], 8.0);
        let why = format!("{u} datasheet 21.3, VS electrolytic close to the device");
        b.pad_near(&why, &cap(5, 1), &vs, 8.0);
        let why = format!("{u} drives {j}: the coil current loop stays short");
        b.centres_within(&why, u, j, 18.0);
        b.closer(&why, u, j, other);
    }
    let why = "AP63203 layout 4, VIN capacitor as close as possible";
    b.pad_near(why, "C1.1", &["U1.3"], 2.0);
    b.pad_near(why, "C1.2", &["U1.4"], 5.0);
    let why = "AP63203 table 2, the switch loop: BST cap and inductor on SW";
    b.pad_near(why, "C2.1", &["U1.6"], 3.0);
    b.pad_near(why, "C2.2", &["U1.5"], 3.0);
    b.pad_near(why, "L1.1", &["U1.5"], 3.0);
    b.pad_near(why, "C3.1", &["L1.2", "C4.1"], 4.0);
    b.pad_near(why, "C4.1", &["L1.2", "C3.1"], 4.0);
    let why = "AN5711, a small ceramic on every VDD, VDDA and VBAT pin";
    let decaps = ["C10.1", "C11.1", "C12.1", "C13.1", "C15.1", "C16.1", "C17.1"];
    for pin in ["U2.1", "U2.13", "U2.19", "U2.32", "U2.48", "U2.64"] {
        b.pad_near(why, pin, &decaps, 5.0);
    }
    let why = "AN5711, 2.2 uF on each VCAP pin";
    b.pad_near(why, "U2.30", &["C18.1", "C19.1"], 5.0);
    b.pad_near(why, "U2.62", &["C18.1", "C19.1"], 5.0);
    let why = "LSE crystal next to OSC32";
    b.pad_near(why, "Y1.1", &["U2.3"], 6.0);
    b.pad_near(why, "Y1.2", &["U2.4"], 6.0);
    b.pad_near(why, "C21.1", &["Y1.1"], 4.0);
    b.pad_near(why, "C22.1", &["Y1.2"], 4.0);
}

fn controller_rules(b: &mut Board) {
    let why =
        "power entry: fuse at the connector, then the reverse diode, the TVS on the protected node";
    b.pad_near(why, "F1.1", &["J1.1"], 4.0);
    b.pad_near(why, "D1.2", &["F1.2"], 5.0);
    b.pad_near(why, "D2.1", &["D1.1"], 8.0);
    b.in_order(why, "J1", &["F1", "D1", "U1"]);
    let why = "USBLC6-2 datasheet: the ESD array as close as possible to the connector";
    b.pad_near(why, "U3.1", &["J2.A6", "J2.B6"], 6.0);
    b.pad_near(why, "U3.3", &["J2.A7", "J2.B7"], 6.0);
    let why = "USB-C CC pull-downs at the connector";
    b.pad_near(why, "R5.1", &["J2.A5"], 5.0);
    b.pad_near(why, "R6.1", &["J2.B5"], 5.0);
    let why = "NTC bias and input resistor at the sensor connector";
    b.pad_near(why, "R9.2", &["J4.1"], 4.0);
    b.pad_near(why, "R10.1", &["J4.1"], 4.0);
    let why = "AP63200 layout 5: feedback components as close to FB as possible";
    b.pad_near(why, "R1.2", &["U1.1"], 4.0);
    b.pad_near(why, "R2.1", &["U1.1"], 4.0);
    b.pad_near(why, "C6.2", &["U1.1"], 4.0);
    let why = "gate resistor and pull-down at the MOSFET gate";
    b.pad_near(why, "R7.2", &["Q1.1"], 4.0);
    b.pad_near(why, "R8.1", &["Q1.1"], 4.0);
    let why = "the fan switch loop stays short (inferred current, no class current)";
    b.centres_within(why, "Q1", "J3", 18.0);
    b.centres_within(why, "D4", "J3", 10.0);
    let why = "MCP6001: feedback network at the inverting input";
    b.pad_near(why, "R11.2", &["U4.4"], 4.0);
    b.pad_near(why, "R12.1", &["U4.4"], 4.0);
    let why = "MCP6001 datasheet 4.4: VDD bypass within 2 mm";
    b.pad_near(why, "C23.1", &["U4.5"], 2.0);
    let why = "ADC filter cap at the ADC pin";
    b.pad_near(why, "C24.1", &["U2.14"], 5.0);
    let why = "analog interface max_length 25 mm";
    for net in ["NTC", "AIN_P", "AIN_N", "AIN_OUT", "AIN"] {
        b.within_budget(why, net, 25.0);
    }
    let why = "noisy parts 8 mm from sensitive ones";
    for loud in ["U1", "L1", "Q1"] {
        b.apart_at_least(why, "U4", loud, 8.0);
        b.apart_at_least(why, "Y1", loud, 8.0);
    }
    let why = "electrolytic 3 mm from a hot part (life halves per 10 C)";
    b.pads_apart(why, "C1", "U2", 3.0);
    let why = "user-facing parts at the board edge";
    b.by_edge(why, "SW1", 5.0);
    b.by_edge(why, "D3", 5.0);
    let why = "tall-part-shadow, an info-level rule the placer weighs below the decap pulls: at most one small chip a tall part's height from it, besides the buck's own parts that AP63200 layout 4 and 5 put at the device";
    let own = ["C3", "R1", "R2", "C6"];
    let shaded: Vec<String> =
        b.shadows.iter().filter(|r| !own.iter().any(|o| r.ends_with(o))).cloned().collect();
    if shaded.len() > 1 {
        b.broken.push(format!("{why}: {shaded:?}"));
    }
    b.sheets_hold(0.5);
}

fn check(name: &str, rules: fn(&mut Board)) {
    let mut report = Vec::new();
    for seed in SEEDS {
        let l = laid(name, seed);
        let mut b = Board {
            parts: l.parts,
            nets: l.nets,
            outline: l.outline,
            shadows: l.shadows,
            sheets: l.sheets,
            broken: Vec::new(),
        };
        rules(&mut b);
        for x in b.broken {
            report.push(format!("seed {seed}: {x}"));
        }
    }
    assert!(report.is_empty(), "{} placement rules broken:\n{}", report.len(), report.join("\n"));
}

#[test]
fn stepper_placement_follows_the_datasheets() {
    check("stepper", stepper_rules);
}

#[test]
fn stepper_placement_is_deterministic() {
    let key = |v: Vec<Placed>| {
        v.iter()
            .map(|p| format!("{} {:?} {}", p.reference, p.at.to_mm(), p.rotation))
            .collect::<Vec<_>>()
    };
    assert_eq!(key(placed("stepper", 7)), key(placed("stepper", 7)));
}

#[test]
fn controller_placement_follows_the_datasheets() {
    check("controller", controller_rules);
}
