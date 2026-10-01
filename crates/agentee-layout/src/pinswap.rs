use crate::tangle::{self, Tangle};
use agentee_core::board::Board;
use agentee_core::geom::P;
use agentee_core::graphic::Bounds;
use agentee_core::layout::{Layout, LayoutFile};
use agentee_core::schematic::Schematic;
use serde::Serialize;
use std::collections::HashMap;

#[derive(Clone, Debug, Serialize)]
pub struct Swap {
    pub net: String,
    pub from: String,
    pub to: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct PinSwap {
    pub part: String,
    pub movable_nets: usize,
    pub pairs: usize,
    pub before: f64,
    pub after: f64,
    pub swaps: Vec<Swap>,
}

#[derive(Clone, Debug)]
struct Ball {
    number: String,
    at: P,
    bank: String,
    lane: Option<(u32, bool)>,
    clock: Option<&'static str>,
}

fn parse(name: &str) -> Option<(String, Option<(u32, bool)>, Option<&'static str>)> {
    if !name.starts_with("IO_") {
        return None;
    }
    let bank = name.rsplit('_').next()?.to_string();
    let lane = name.strip_prefix("IO_L").and_then(|rest| {
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        let pol = rest[digits.len()..].chars().next()?;
        Some((digits.parse().ok()?, pol == 'P'))
    });
    let clock = if name.contains("MRCC") {
        Some("MRCC")
    } else if name.contains("SRCC") {
        Some("SRCC")
    } else {
        None
    };
    Some((bank, lane, clock))
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn pick(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Held {
    Free,
    Single(usize),
    Pair(usize, usize),
    Fixed,
}

pub fn run(
    layout: &Layout,
    board: &Board,
    schematic: &Schematic,
    file: &LayoutFile,
    part: &str,
    seed: u64,
) -> Result<PinSwap, String> {
    let placed = layout
        .parts
        .iter()
        .find(|p| p.reference == part)
        .ok_or_else(|| format!("no part `{part}` in the layout"))?;
    let mut names: HashMap<&str, &str> = HashMap::new();
    for sp in schematic.parts.iter().filter(|p| p.reference == part) {
        for (_, pin) in sp.pins() {
            names.insert(pin.number.as_str(), pin.name.as_str());
        }
    }
    let mut balls: Vec<Ball> = Vec::new();
    let mut net_at: Vec<Option<usize>> = Vec::new();
    for pad in &placed.pads {
        let Some(name) = names.get(pad.number.as_str()) else { continue };
        let Some((bank, lane, clock)) = parse(name) else { continue };
        let mut b = Bounds::EMPTY;
        pad.outlines.iter().flatten().for_each(|q| b.add(*q));
        balls.push(Ball { number: pad.number.clone(), at: b.center(), bank, lane, clock });
        net_at.push(pad.net);
    }
    let on_part = |n: usize| placed.pads.iter().filter(|q| q.net == Some(n)).count();
    let mut pins = tangle::pins_of(layout);
    let planes: Vec<String> = file.zones.iter().map(|z| z.net.clone()).collect();
    let user = layout.engine.tangle.clone().map(|t| t.weights).unwrap_or_default();
    let tnets = tangle::nets_of(layout, board, &planes, &user);
    let is_clock = |n: usize| {
        layout.nets[n].class.to_ascii_lowercase().contains("clock")
            || layout.nets[n].name.contains("CLK")
    };
    let mut partner: HashMap<usize, usize> = HashMap::new();
    for pr in &layout.pairs {
        partner.insert(pr.p, pr.n);
        partner.insert(pr.n, pr.p);
    }
    let config = |n: &str| {
        ["_D00", "_D01", "_D02", "_D03", "FCS_B", "PUDC_B", "EMCCLK", "MOSI", "_DIN"]
            .iter()
            .any(|t| n.contains(t))
    };
    let mut held: Vec<Held> = balls
        .iter()
        .map(|b| if config(names[b.number.as_str()]) { Held::Fixed } else { Held::Free })
        .collect();
    let mut pairs = 0;
    let mut singles = 0;
    for (i, b) in balls.iter().enumerate() {
        let Some(n) = net_at[i] else { continue };
        if held[i] == Held::Fixed {
            continue;
        }
        let movable = on_part(n) == 1 && pins[n].len() >= 2 && tnets[n].is_some();
        if !movable {
            held[i] = Held::Fixed;
            continue;
        }
        if let Some(&m) = partner.get(&n) {
            let mate = balls.iter().position(|c| {
                c.bank == b.bank
                    && c.lane.map(|l| l.0) == b.lane.map(|l| l.0)
                    && c.number != b.number
            });
            match (b.lane, mate) {
                (Some((_, true)), Some(j)) if net_at[j] == Some(m) => {
                    held[i] = Held::Pair(n, m);
                    pairs += 1;
                }
                (Some((_, false)), Some(j)) if net_at[j] == Some(m) => held[i] = Held::Pair(m, n),
                _ => held[i] = Held::Fixed,
            }
            continue;
        }
        held[i] = Held::Single(n);
        singles += 1;
    }
    let pin_index = |pins: &Vec<Vec<P>>, n: usize, at: P| {
        pins[n].iter().position(|q| agentee_core::geom::dist(*q, at) < 1e-6)
    };
    let mut tangle = Tangle::new(tnets, &pins, &layout.ratsnest);
    let before = tangle.cost();
    let original: Vec<Held> = held.clone();
    let mut rng = Rng(seed.max(1).wrapping_mul(0x9e37_79b9_7f4a_7c15));
    let movers: Vec<usize> = (0..balls.len())
        .filter(|&i| {
            matches!(held[i], Held::Single(_))
                || matches!((held[i], balls[i].lane), (Held::Pair(..), Some((_, true))))
        })
        .collect();
    if movers.is_empty() {
        return Ok(PinSwap {
            part: part.into(),
            movable_nets: 0,
            pairs: 0,
            before,
            after: before,
            swaps: Vec::new(),
        });
    }
    let mate_of = |i: usize| -> Option<usize> {
        let (l, p) = balls[i].lane?;
        balls.iter().position(|c| c.bank == balls[i].bank && c.lane == Some((l, !p)))
    };
    let clock_ok = |from: usize, to: usize| {
        let capable = balls[from].clock.is_some() && balls[from].lane.is_some_and(|l| l.1);
        !capable || (balls[to].clock == balls[from].clock && balls[to].lane.is_some_and(|l| l.1))
    };
    let steps = 60_000usize;
    let (t0, t1) = (2.0f64, 0.02f64);
    let mut cost = before;
    for step in 0..steps {
        let temp = t0 * (t1 / t0).powf(step as f64 / steps as f64);
        let a = movers[rng.pick(movers.len())];
        let b = rng.pick(balls.len());
        if a == b || balls[a].bank != balls[b].bank {
            continue;
        }
        let mut moves: Vec<(usize, usize)> = Vec::new();
        match held[a] {
            Held::Single(n) => {
                if !matches!(held[b], Held::Free | Held::Single(_)) {
                    continue;
                }
                if is_clock(n) && !clock_ok(a, b) {
                    continue;
                }
                if let Held::Single(m) = held[b]
                    && is_clock(m)
                    && !clock_ok(b, a)
                {
                    continue;
                }
                moves.push((a, b));
            }
            Held::Pair(p, _) => {
                let (Some(am), Some(bp)) = (mate_of(a), Some(b)) else { continue };
                if balls[bp].lane.map(|l| l.1) != Some(true) {
                    continue;
                }
                let Some(bm) = mate_of(bp) else { continue };
                if !matches!(held[bp], Held::Free | Held::Single(_) | Held::Pair(..))
                    || !matches!(held[bm], Held::Free | Held::Single(_) | Held::Pair(..))
                {
                    continue;
                }
                if is_clock(p) && !clock_ok(a, bp) {
                    continue;
                }
                moves.push((a, bp));
                moves.push((am, bm));
            }
            _ => continue,
        }
        let mut touched: Vec<usize> = Vec::new();
        let mut undo: Vec<(usize, Vec<P>)> = Vec::new();
        for &(x, y) in &moves {
            for (from, to) in [(x, y), (y, x)] {
                let n = match held[from] {
                    Held::Single(n) => n,
                    Held::Pair(p, q) => {
                        if balls[from].lane.map(|l| l.1) == Some(true) {
                            p
                        } else {
                            q
                        }
                    }
                    _ => continue,
                };
                if !undo.iter().any(|u| u.0 == n) {
                    undo.push((n, pins[n].clone()));
                }
                if let Some(k) = pin_index(&pins, n, balls[from].at) {
                    pins[n][k] = balls[to].at;
                }
                if !touched.contains(&n) {
                    touched.push(n);
                }
            }
        }
        for &n in &touched {
            tangle.set_net(n, &pins[n]);
        }
        let delta = tangle.cost() - cost;
        if delta <= 0.0 || rng.unit() < (-delta / temp).exp() {
            cost = tangle.cost();
            for &(x, y) in &moves {
                held.swap(x, y);
            }
        } else {
            for (n, old) in undo {
                pins[n] = old;
                tangle.set_net(n, &pins[n]);
            }
        }
    }
    let mut swaps = Vec::new();
    for i in 0..balls.len() {
        let net = match held[i] {
            Held::Single(n) => Some(n),
            Held::Pair(p, q) => Some(if balls[i].lane.map(|l| l.1) == Some(true) { p } else { q }),
            _ => None,
        };
        let Some(n) = net else { continue };
        let from = (0..balls.len()).find(|&j| match original[j] {
            Held::Single(m) => m == n,
            Held::Pair(p, q) => (if balls[j].lane.map(|l| l.1) == Some(true) { p } else { q }) == n,
            _ => false,
        });
        if let Some(j) = from
            && j != i
        {
            swaps.push(Swap {
                net: layout.nets[n].name.clone(),
                from: balls[j].number.clone(),
                to: balls[i].number.clone(),
            });
        }
    }
    swaps.sort_by(|a, b| a.net.cmp(&b.net));
    Ok(PinSwap {
        part: part.into(),
        movable_nets: singles + 2 * pairs,
        pairs,
        before,
        after: cost,
        swaps,
    })
}
