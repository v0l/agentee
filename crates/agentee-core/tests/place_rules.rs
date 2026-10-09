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

fn placed(name: &str, seed: u64) -> Vec<Placed> {
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
    let input = PlaceInput {
        board,
        outline: &layout.outline,
        cutouts: &layout.board_cutouts,
        schematic: sch,
        footprints: &footprints,
        placements: &file.footprints,
        spec: &spec,
        fast_nets: Vec::new(),
        heat: Vec::new(),
        silk: place::board_silk(&layout.graphics, &layout.artwork),
        texts: place::movable_texts(&layout.graphics),
        tracks: &file.tracks,
        vias: &file.vias,
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
    let parts = Project::load(&dir).unwrap().layouts[0].item.parts.clone();
    if let Ok(keep) = std::env::var("AGENTEE_PLACE_KEEP") {
        let to = Path::new(&keep).join(format!("{name}-{seed}"));
        let _ = std::fs::remove_dir_all(&to);
        copy_dir(&dir, &to);
    }
    let _ = std::fs::remove_dir_all(&dir);
    parts
}

struct Board {
    parts: Vec<Placed>,
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
        b.pad_near(&why, &cap(1, 1), &[&pin(4)], 4.0);
        b.pad_near(&why, &cap(1, 2), &[&pin(5)], 4.0);
        b.pad_near(&why, &cap(2, 1), &[&pin(6)], 4.0);
        b.pad_near(&why, &cap(3, 1), &[&pin(8)], 4.0);
        b.pad_near(&why, &cap(0, 1), &vs, 4.0);
        b.pad_near(&why, &cap(4, 1), &[&pin(15)], 4.0);
        let why = format!("{u} datasheet 21.2, sense resistors close to BRA and BRB");
        b.pad_near(&why, &format!("R{r}.1"), &[&pin(23)], 7.0);
        b.pad_near(&why, &format!("R{}.1", r + 1), &[&pin(27)], 7.0);
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
        b.pad_near(why, pin, &decaps, 4.0);
    }
    let why = "AN5711, 2.2 uF on each VCAP pin";
    b.pad_near(why, "U2.30", &["C18.1", "C19.1"], 4.0);
    b.pad_near(why, "U2.62", &["C18.1", "C19.1"], 4.0);
    let why = "LSE crystal next to OSC32";
    b.pad_near(why, "Y1.1", &["U2.3"], 6.0);
    b.pad_near(why, "Y1.2", &["U2.4"], 6.0);
    b.pad_near(why, "C21.1", &["Y1.1"], 4.0);
    b.pad_near(why, "C22.1", &["Y1.2"], 4.0);
}

fn check(name: &str, rules: fn(&mut Board)) {
    let mut report = Vec::new();
    for seed in SEEDS {
        let mut b = Board { parts: placed(name, seed), broken: Vec::new() };
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
