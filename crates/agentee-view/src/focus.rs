use agentee_core::graphic::{Bounds, Shape};
use agentee_core::layout::Layout;
use agentee_core::schematic::Schematic;
use egui::Painter;
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Context {
    #[default]
    Dim,
    Hide,
    Show,
}

impl std::str::FromStr for Context {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "dim" => Ok(Context::Dim),
            "hide" => Ok(Context::Hide),
            "show" => Ok(Context::Show),
            _ => Err(format!("context is dim, hide or show, not {s}")),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Focus {
    pub parts: HashSet<usize>,
    pub nets: HashSet<usize>,
    pub context: Context,
}

const DIM: f32 = 0.16;

impl Focus {
    pub fn part(&self, i: usize) -> bool {
        self.parts.contains(&i)
    }

    pub fn net(&self, n: usize) -> bool {
        self.nets.contains(&n)
    }
}

pub fn pick(p: &Painter, focus: Option<&Focus>, lit: bool) -> Option<Painter> {
    match focus.map(|f| f.context) {
        None | Some(Context::Show) => Some(p.clone()),
        _ if lit => Some(p.clone()),
        Some(Context::Dim) => {
            let mut d = p.clone();
            d.multiply_opacity(DIM);
            Some(d)
        }
        Some(Context::Hide) => None,
    }
}

pub fn matches(pattern: &str, name: &str) -> bool {
    let Some((head, rest)) = pattern.split_once('*') else { return pattern == name };
    let Some(mut tail) = name.strip_prefix(head) else { return false };
    let parts: Vec<&str> = rest.split('*').collect();
    let (last, middle) = parts.split_last().unwrap();
    for m in middle {
        match tail.find(m) {
            Some(k) => tail = &tail[k + m.len()..],
            None => return false,
        }
    }
    tail.len() >= last.len() && tail.ends_with(last)
}

fn grow(b: Bounds, pad: f64, min: f64) -> Bounds {
    let [w, h] = b.size();
    let c = b.center();
    let half = |s: f64| (s / 2.0 + pad + 0.1 * w.max(h)).max(min / 2.0);
    let (hx, hy) = (half(w), half(h));
    Bounds { min: [c[0] - hx, c[1] - hy], max: [c[0] + hx, c[1] + hy] }
}

fn unknown(names: &[String], hit: &[bool], what: &str) -> Result<(), String> {
    let missing: Vec<&str> =
        names.iter().zip(hit).filter(|(_, h)| !**h).map(|(n, _)| n.as_str()).collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!("no part, net or pin named {} in {what}", missing.join(", ")))
    }
}

pub fn schematic(
    s: &Schematic,
    names: &[String],
    context: Context,
) -> Result<(Focus, Bounds), String> {
    let mut f = Focus { context, ..Default::default() };
    let mut b = Bounds::EMPTY;
    let mut hit = vec![false; names.len()];
    for (k, name) in names.iter().enumerate() {
        for (ni, n) in s.nets.iter().enumerate().filter(|(_, n)| matches(name, &n.name)) {
            hit[k] = true;
            f.nets.insert(ni);
            n.wires.iter().flatten().for_each(|q| b.add(*q));
            for r in &n.pins {
                f.parts.insert(r.part);
                b.add(s.parts[r.part].pin_at(r.pin));
            }
        }
        for (pi, p) in s.parts.iter().enumerate().filter(|(_, p)| matches(name, &p.reference)) {
            hit[k] = true;
            f.parts.insert(pi);
            b.union(&p.bounds());
        }
        if let Some((reference, number)) = name.rsplit_once('.') {
            for (pi, p) in s.parts.iter().enumerate().filter(|(_, p)| p.reference == reference) {
                for (i, _) in p.pins().filter(|(_, pin)| pin.number == number) {
                    hit[k] = true;
                    f.parts.insert(pi);
                    b.add(p.pin_at(i));
                    let r = agentee_core::schematic::PinRef { part: pi, pin: i };
                    if let Some(ni) = s.net_of(r) {
                        f.nets.insert(ni);
                        s.nets[ni].wires.iter().flatten().for_each(|q| b.add(*q));
                    }
                }
            }
        }
    }
    unknown(names, &hit, &s.name)?;
    Ok((f, grow(b, 5.0, 15.0)))
}

pub fn layout(l: &Layout, names: &[String], context: Context) -> Result<(Focus, Bounds), String> {
    let mut f = Focus { context, ..Default::default() };
    let mut b = Bounds::EMPTY;
    let mut hit = vec![false; names.len()];
    for (k, name) in names.iter().enumerate() {
        for ni in (0..l.nets.len()).filter(|n| matches(name, &l.nets[*n].name)) {
            hit[k] = true;
            f.nets.insert(ni);
            l.tracks.iter().filter(|t| t.net == ni).flat_map(|t| &t.points).for_each(|q| b.add(*q));
            for v in l.vias.iter().filter(|v| v.net == ni) {
                b.add_circle(v.at, v.diameter / 2.0);
            }
            let pads = l.parts.iter().flat_map(|p| &p.pads).filter(|q| q.net == Some(ni));
            pads.flat_map(|q| q.outlines.iter().flatten()).for_each(|q| b.add(*q));
        }
        for (pi, p) in l.parts.iter().enumerate().filter(|(_, p)| matches(name, &p.reference)) {
            hit[k] = true;
            f.parts.insert(pi);
            b.union(&placed_bounds(p));
        }
        if let Some((reference, number)) = name.rsplit_once('.') {
            for (pi, p) in l.parts.iter().enumerate().filter(|(_, p)| p.reference == reference) {
                for pad in p.pads.iter().filter(|q| q.number == number) {
                    hit[k] = true;
                    f.parts.insert(pi);
                    pad.outlines.iter().flatten().for_each(|q| b.add(*q));
                    f.nets.extend(pad.net);
                }
            }
        }
    }
    unknown(names, &hit, &l.name)?;
    Ok((f, grow(b, 1.0, 4.0)))
}

fn placed_bounds(p: &agentee_core::layout::Placed) -> Bounds {
    let mut b = Bounds::EMPTY;
    p.pads.iter().flat_map(|q| q.outlines.iter().flatten()).for_each(|q| b.add(*q));
    let t = p.transform();
    for g in p.footprint.graphics.iter().filter(|g| !matches!(g.shape, Shape::Text { .. })) {
        let gb = g.bounds();
        if !gb.is_empty() {
            for c in [gb.min, gb.max, [gb.min[0], gb.max[1]], [gb.max[0], gb.min[1]]] {
                b.add(t.apply(c));
            }
        }
    }
    b
}

pub fn part_lit(l: &Layout, f: &Focus, pi: usize) -> bool {
    f.part(pi) || l.parts[pi].pads.iter().any(|q| q.net.is_some_and(|n| f.net(n)))
}

#[cfg(test)]
mod tests {
    use super::matches;

    #[test]
    fn globs_match_prefixes_suffixes_and_middles() {
        assert!(matches("R*", "R12"));
        assert!(matches("*_CLK", "SPI_CLK"));
        assert!(matches("SPI_*_N", "SPI_MOSI_N"));
        assert!(matches("*", "anything"));
        assert!(!matches("R*", "C1"));
        assert!(!matches("AB*BA", "ABA"));
        assert!(matches("U1", "U1"));
        assert!(!matches("U1", "U10"));
    }
}
