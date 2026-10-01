use agentee_core::board::Board;
use agentee_core::geom::{self, P};
use agentee_core::graphic::Bounds;
use agentee_core::layout::Layout;
use agentee_core::place;
use std::collections::{BTreeMap, HashMap};

pub const BUS_CROSS: f64 = 4.0;

#[derive(Clone, Debug)]
pub struct TNet {
    pub name: String,
    pub power: bool,
    pub weight: f64,
    pub bundle: Option<usize>,
    pub unit: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Group {
    Bundle(usize),
    Link(usize, usize),
}

#[derive(Clone, Debug)]
struct Link {
    net: usize,
    a: P,
    b: P,
    group: Group,
    alive: bool,
}

pub struct Tangle {
    pub nets: Vec<Option<TNet>>,
    links: Vec<Link>,
    of_net: Vec<Vec<usize>>,
    counts: HashMap<(Group, Group), u32>,
    weight: HashMap<Group, f64>,
    intra: f64,
    cost: f64,
    free: Vec<usize>,
}

pub fn class_weight(board: &Board, class: &str, user: &BTreeMap<String, f64>) -> f64 {
    if let Some(w) = user.get(class) {
        return *w;
    }
    let c = board.netclasses.iter().find(|c| c.name == class);
    match c {
        Some(c) if c.current.is_some() => 0.25,
        Some(c) if c.diff_gap.is_some() || c.impedance.is_some() => 3.0,
        _ if place::is_rf_class(board, class) => 3.0,
        _ => 1.0,
    }
}

pub fn nets_of(
    layout: &Layout,
    board: &Board,
    planes: &[String],
    user: &BTreeMap<String, f64>,
) -> Vec<Option<TNet>> {
    let mut bundle_of: HashMap<&str, usize> = HashMap::new();
    for (bi, iface) in layout.interfaces.iter().enumerate() {
        for lane in &iface.lanes {
            for n in &lane.nets {
                bundle_of.entry(n.as_str()).or_insert(bi);
            }
        }
    }
    let mut unit: Vec<usize> = (0..layout.nets.len()).collect();
    for pr in &layout.pairs {
        unit[pr.n] = unit[pr.p];
    }
    layout
        .nets
        .iter()
        .enumerate()
        .map(|(i, n)| {
            if place::is_ground(&n.name) {
                return None;
            }
            let power = planes.contains(&n.name)
                || board.netclasses.iter().any(|c| c.name == n.class && c.current.is_some());
            Some(TNet {
                name: n.name.clone(),
                power,
                weight: class_weight(board, &n.class, user),
                bundle: bundle_of.get(n.name.as_str()).copied(),
                unit: unit[i],
            })
        })
        .collect()
}

pub fn pins_of(layout: &Layout) -> Vec<Vec<P>> {
    let mut pins = vec![Vec::new(); layout.nets.len()];
    for part in &layout.parts {
        for pad in &part.pads {
            if let Some(n) = pad.net {
                let mut bb = Bounds::EMPTY;
                pad.outlines.iter().flatten().for_each(|q| bb.add(*q));
                if !bb.is_empty() {
                    pins[n].push(bb.center());
                }
            }
        }
    }
    pins
}

pub fn mst(pins: &[P]) -> Vec<(P, P)> {
    let n = pins.len();
    if n < 2 {
        return Vec::new();
    }
    let mut in_tree = vec![false; n];
    let mut best = vec![(f64::MAX, 0usize); n];
    in_tree[0] = true;
    for j in 1..n {
        best[j] = (geom::dist(pins[0], pins[j]), 0);
    }
    let mut out = Vec::new();
    for _ in 1..n {
        let Some(j) =
            (0..n).filter(|&j| !in_tree[j]).min_by(|&a, &b| best[a].0.total_cmp(&best[b].0))
        else {
            break;
        };
        in_tree[j] = true;
        out.push((pins[best[j].1], pins[j]));
        for k in 0..n {
            if !in_tree[k] {
                let d = geom::dist(pins[j], pins[k]);
                if d < best[k].0 {
                    best[k] = (d, j);
                }
            }
        }
    }
    out
}

fn cross(a: &Link, b: &Link) -> bool {
    let shared = |p: P, q: P| geom::dist(p, q) < 1e-6;
    if shared(a.a, b.a) || shared(a.a, b.b) || shared(a.b, b.a) || shared(a.b, b.b) {
        return false;
    }
    geom::segments_intersect(a.a, a.b, b.a, b.b)
}

impl Tangle {
    pub fn new(nets: Vec<Option<TNet>>, pins: &[Vec<P>], rats: &[(P, P, usize)]) -> Tangle {
        let mut t = Tangle {
            of_net: vec![Vec::new(); nets.len()],
            nets,
            links: Vec::new(),
            counts: HashMap::new(),
            weight: HashMap::new(),
            intra: 0.0,
            cost: 0.0,
            free: Vec::new(),
        };
        for n in 0..t.nets.len() {
            if t.nets[n].as_ref().is_some_and(|x| x.power) {
                let segs: Vec<(P, P)> =
                    rats.iter().filter(|r| r.2 == n).map(|r| (r.0, r.1)).collect();
                t.set_links(n, segs);
            } else {
                t.set_net(n, &pins[n]);
            }
        }
        t
    }

    pub fn cost(&self) -> f64 {
        self.cost
    }

    fn pair_weight(&self, g: Group, h: Group) -> f64 {
        match (g, h) {
            (Group::Bundle(_), Group::Bundle(_)) => BUS_CROSS,
            _ => self.weight[&g].min(self.weight[&h]),
        }
    }

    fn touch(&mut self, i: usize, j: usize, add: bool) {
        let (li, lj) = (&self.links[i], &self.links[j]);
        let (ni, nj) = (li.net, lj.net);
        let (ti, tj) = (self.nets[ni].as_ref().unwrap(), self.nets[nj].as_ref().unwrap());
        if ti.unit == tj.unit {
            return;
        }
        if ti.bundle.is_some() && ti.bundle == tj.bundle {
            self.intra += if add { BUS_CROSS } else { -BUS_CROSS };
            self.cost += if add { BUS_CROSS } else { -BUS_CROSS };
            return;
        }
        let (g, h) = if li.group < lj.group { (li.group, lj.group) } else { (lj.group, li.group) };
        let w = self.pair_weight(g, h);
        let c = self.counts.entry((g, h)).or_insert(0);
        if add {
            *c += 1;
            if *c == 1 {
                self.cost += w;
            }
        } else {
            *c -= 1;
            if *c == 0 {
                self.cost -= w;
            }
        }
    }

    pub fn set_net(&mut self, n: usize, pins: &[P]) {
        self.set_links(n, mst(pins));
    }

    fn set_links(&mut self, n: usize, segs: Vec<(P, P)>) {
        let Some(tn) = self.nets[n].clone() else { return };
        for li in std::mem::take(&mut self.of_net[n]) {
            for j in 0..self.links.len() {
                if j != li
                    && self.links[j].alive
                    && self.links[j].net != n
                    && cross(&self.links[li], &self.links[j])
                {
                    self.touch(li, j, false);
                }
            }
            self.links[li].alive = false;
            self.free.push(li);
        }
        for (k, (a, b)) in segs.into_iter().enumerate() {
            let group = match tn.bundle {
                Some(bi) => Group::Bundle(bi),
                None => Group::Link(tn.unit, k),
            };
            let w = self.weight.entry(group).or_insert(0.0);
            *w = w.max(tn.weight);
            let link = Link { net: n, a, b, group, alive: true };
            let li = match self.free.pop() {
                Some(i) => {
                    self.links[i] = link;
                    i
                }
                None => {
                    self.links.push(link);
                    self.links.len() - 1
                }
            };
            self.of_net[n].push(li);
            for j in 0..self.links.len() {
                if j == li {
                    continue;
                }
                if self.links[j].alive
                    && self.links[j].net != n
                    && cross(&self.links[li], &self.links[j])
                {
                    self.touch(li, j, true);
                }
            }
        }
    }

    pub fn worst(&self) -> Vec<(String, f64)> {
        let mut by_net: BTreeMap<usize, f64> = BTreeMap::new();
        for i in 0..self.links.len() {
            if !self.links[i].alive {
                continue;
            }
            for j in i + 1..self.links.len() {
                let (ni, nj) = (self.links[i].net, self.links[j].net);
                if self.links[j].alive && ni != nj && cross(&self.links[i], &self.links[j]) {
                    let w =
                        self.weight[&self.links[i].group].min(self.weight[&self.links[j].group]);
                    *by_net.entry(ni).or_default() += w;
                    *by_net.entry(nj).or_default() += w;
                }
            }
        }
        by_net
            .into_iter()
            .filter_map(|(n, c)| self.nets[n].as_ref().map(|t| (t.name.clone(), c)))
            .collect()
    }
}
