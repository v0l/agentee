use crate::diag::Diags;
use crate::schematic::{PinRef, Schematic};
use crate::symbol::{Levels, PinType, Threshold};
use std::collections::{BTreeMap, HashMap, HashSet};

pub const MARGIN: f64 = 0.1;
pub const RESISTOR_TOLERANCE: f64 = 0.01;
const MAX_STATES: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq)]
enum End {
    Node(usize),
    Rail(usize),
}

struct Resistor {
    a: End,
    b: End,
    var: usize,
}

struct Switch {
    label: String,
    pins: Vec<(String, End)>,
}

struct Input {
    pin: PinRef,
    at: End,
    levels: Levels,
    supply: Option<usize>,
    leak: Option<usize>,
    analog: bool,
    bidirectional: bool,
}

struct OpenCollector {
    label: String,
    at: End,
}

#[derive(Default)]
struct Cluster {
    nets: Vec<usize>,
    vars: Vec<[f64; 2]>,
    nominal: Vec<f64>,
    rails: Vec<usize>,
    rail_vars: HashMap<usize, usize>,
    zero: Option<usize>,
    leaks: Vec<usize>,
    resistors: Vec<Resistor>,
    switches: Vec<Switch>,
    pulls: Vec<OpenCollector>,
    inputs: Vec<Input>,
}

impl Cluster {
    fn var(&mut self, range: [f64; 2], nominal: f64) -> usize {
        self.vars.push(range);
        self.nominal.push(nominal);
        self.vars.len() - 1
    }

    fn rail(&mut self, net: usize, range: [f64; 2]) -> usize {
        if let Some(&v) = self.rail_vars.get(&net) {
            return v;
        }
        let v = self.var(range, (range[0] + range[1]) / 2.0);
        self.rails.push(v);
        self.rail_vars.insert(net, v);
        v
    }

    fn zero(&mut self) -> usize {
        if let Some(z) = self.zero {
            return z;
        }
        let v = self.var([0.0, 0.0], 0.0);
        self.rails.push(v);
        self.zero = Some(v);
        v
    }
}

enum Role {
    Ignore,
    Resistor(f64, f64),
    Switch,
    Pins,
}

fn role(s: &Schematic, part: usize) -> Role {
    let p = &s.parts[part];
    if p.dnp || p.symbol.power {
        return Role::Ignore;
    }
    let two = p.symbol.pins.len() == 2;
    match p.symbol.reference.as_str() {
        "C" | "TP" | "H" | "MH" => Role::Ignore,
        "R" if two => match crate::sim::parse_value(&p.value) {
            Some(r) if r > 0.0 => {
                let tol = p
                    .fields
                    .get("tolerance")
                    .and_then(|t| crate::units::Percent::parse(t).ok())
                    .map(|t| t.0 / 100.0)
                    .unwrap_or(RESISTOR_TOLERANCE);
                Role::Resistor(r, tol)
            }
            _ => Role::Pins,
        },
        "L" | "FB" if two => Role::Resistor(1e-3, 0.0),
        "SW" => Role::Switch,
        _ => Role::Pins,
    }
}

fn pin_levels(s: &Schematic, r: PinRef) -> Option<Levels> {
    let sym = &s.parts[r.part].symbol;
    let pin = &sym.pins[r.pin];
    let base = sym.levels.clone().unwrap_or_default();
    match &pin.levels {
        Some(own) => Some(own.over(&base)),
        None if sym.levels.is_some()
            && matches!(pin.kind, PinType::Input | PinType::Bidirectional) =>
        {
            Some(base)
        }
        None => None,
    }
}

fn supply_net(
    s: &Schematic,
    r: PinRef,
    name: &str,
    net_of: &HashMap<PinRef, usize>,
) -> Option<usize> {
    let reference = &s.parts[r.part].reference;
    s.parts.iter().enumerate().filter(|(_, p)| &p.reference == reference).find_map(|(pi, p)| {
        p.symbol
            .pins
            .iter()
            .enumerate()
            .filter(|(_, q)| q.number == name || q.name == name)
            .find_map(|(ni, _)| net_of.get(&PinRef { part: pi, pin: ni }).copied())
    })
}

struct Uf(Vec<usize>);

impl Uf {
    fn find(&mut self, mut i: usize) -> usize {
        while self.0[i] != i {
            self.0[i] = self.0[self.0[i]];
            i = self.0[i];
        }
        i
    }

    fn union(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            self.0[a] = b;
        }
    }
}

pub fn check(s: &Schematic, d: &mut Diags) {
    if s.rails.is_empty() {
        return;
    }
    let mut net_of: HashMap<PinRef, usize> = HashMap::new();
    for (i, n) in s.nets.iter().enumerate() {
        for r in &n.pins {
            net_of.insert(*r, i);
        }
    }
    let rail_of = |net: usize| s.rails.get(&s.nets[net].name).copied();
    let roles: Vec<Role> = (0..s.parts.len()).map(|p| role(s, p)).collect();
    let mut uf = Uf((0..s.nets.len()).collect());
    for (pi, r) in roles.iter().enumerate() {
        if !matches!(r, Role::Resistor(..) | Role::Switch) {
            continue;
        }
        let nets: Vec<usize> = (0..s.parts[pi].symbol.pins.len())
            .filter_map(|ni| net_of.get(&PinRef { part: pi, pin: ni }).copied())
            .filter(|&n| rail_of(n).is_none())
            .collect();
        for w in nets.windows(2) {
            uf.union(w[0], w[1]);
        }
    }
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for n in 0..s.nets.len() {
        if rail_of(n).is_none() {
            groups.entry(uf.find(n)).or_default().push(n);
        }
    }
    let mut clusters: Vec<Vec<usize>> = groups.into_values().collect();
    clusters.extend((0..s.nets.len()).filter(|&n| rail_of(n).is_some()).map(|n| vec![n]));
    let analog: HashSet<PinRef> = s.analog.iter().copied().collect();
    for nets in clusters {
        let Some(mut c) = build(s, &nets, &roles, &net_of, &analog, d) else { continue };
        evaluate(s, &mut c, d);
    }
}

fn build(
    s: &Schematic,
    nets: &[usize],
    roles: &[Role],
    net_of: &HashMap<PinRef, usize>,
    analog: &HashSet<PinRef>,
    d: &mut Diags,
) -> Option<Cluster> {
    let rail_of = |net: usize| s.rails.get(&s.nets[net].name).copied();
    let only_rail = nets.len() == 1 && rail_of(nets[0]).is_some();
    let mut c = Cluster { nets: nets.to_vec(), ..Default::default() };
    let local: HashMap<usize, usize> = if only_rail {
        HashMap::new()
    } else {
        nets.iter().enumerate().map(|(i, &n)| (n, i)).collect()
    };
    let end = |c: &mut Cluster, net: usize| -> End {
        match rail_of(net) {
            Some(range) => End::Rail(c.rail(net, range)),
            None => End::Node(local[&net]),
        }
    };
    let mut seen_parts: HashSet<usize> = HashSet::new();
    for &net in nets {
        for &r in &s.nets[net].pins {
            match roles[r.part] {
                Role::Ignore => {}
                Role::Resistor(ohms, tol) if !only_rail => {
                    if !seen_parts.insert(r.part) {
                        continue;
                    }
                    let ends: Vec<usize> = (0..2)
                        .filter_map(|ni| net_of.get(&PinRef { part: r.part, pin: ni }).copied())
                        .collect();
                    if ends.len() == 2 && ends[0] != ends[1] {
                        let (a, b) = (end(&mut c, ends[0]), end(&mut c, ends[1]));
                        let var = c.var([ohms * (1.0 - tol), ohms * (1.0 + tol)], ohms);
                        c.resistors.push(Resistor { a, b, var });
                    }
                }
                Role::Switch if !only_rail => {
                    if !seen_parts.insert(r.part) {
                        continue;
                    }
                    let p = &s.parts[r.part];
                    let mut pins = Vec::new();
                    for (ni, pin) in p.symbol.pins.iter().enumerate() {
                        if let Some(&n) = net_of.get(&PinRef { part: r.part, pin: ni }) {
                            pins.push((pin.number.clone(), end(&mut c, n)));
                        }
                    }
                    c.switches.push(Switch { label: p.reference.clone(), pins });
                }
                Role::Resistor(..) | Role::Switch => {}
                Role::Pins => {
                    let pin = &s.parts[r.part].symbol.pins[r.pin];
                    if let Some(levels) = pin_levels(s, r) {
                        let at = end(&mut c, net);
                        let supply = match &levels.supply {
                            Some(name) => match supply_net(s, r, name, net_of) {
                                Some(n) => match rail_of(n) {
                                    Some(range) => Some(c.rail(n, range)),
                                    None if levels.needs_supply() => {
                                        d.info(
                                            format!("net {}", s.nets[net].name),
                                            format!(
                                                "{} not checked: its supply {name} is on net {}, which is not in [rails]",
                                                s.pin_label(r),
                                                s.nets[n].name
                                            ),
                                        );
                                        if !only_rail {
                                            return None;
                                        }
                                        continue;
                                    }
                                    None => None,
                                },
                                None => None,
                            },
                            None => None,
                        };
                        if levels.needs_supply() && supply.is_none() {
                            if !only_rail {
                                return None;
                            }
                            continue;
                        }
                        if let (Some(up), Some(sv), false) = (levels.pull_up, supply, only_rail) {
                            let var = c.var([up, up], up);
                            c.resistors.push(Resistor { a: at, b: End::Rail(sv), var });
                        }
                        if let (Some(down), false) = (levels.pull_down, only_rail) {
                            let z = c.zero();
                            let var = c.var([down, down], down);
                            c.resistors.push(Resistor { a: at, b: End::Rail(z), var });
                        }
                        let leak = levels.leakage.filter(|_| !only_rail).map(|i| {
                            let v = c.var([-i, i], 0.0);
                            c.leaks.push(v);
                            v
                        });
                        c.inputs.push(Input {
                            pin: r,
                            at,
                            levels,
                            supply,
                            leak,
                            analog: analog.contains(&r),
                            bidirectional: pin.kind == PinType::Bidirectional,
                        });
                        continue;
                    }
                    if only_rail {
                        continue;
                    }
                    match pin.kind {
                        PinType::Input | PinType::NoConnect => {}
                        PinType::OpenCollector => {
                            let at = end(&mut c, net);
                            c.zero();
                            c.pulls.push(OpenCollector { label: s.pin_label(r), at });
                        }
                        _ => return None,
                    }
                }
            }
        }
    }
    (!c.inputs.is_empty()).then_some(c)
}

#[derive(Clone)]
struct State {
    closed: Vec<Option<(usize, usize)>>,
    pulled: Vec<bool>,
}

fn states(c: &Cluster) -> Option<Vec<State>> {
    let mut options: Vec<Vec<Option<(usize, usize)>>> = Vec::new();
    for sw in &c.switches {
        let mut o = vec![None];
        for i in 0..sw.pins.len() {
            for j in i + 1..sw.pins.len() {
                o.push(Some((i, j)));
            }
        }
        options.push(o);
    }
    let total = options.iter().map(Vec::len).product::<usize>() << c.pulls.len();
    if total > MAX_STATES {
        return None;
    }
    let mut out = vec![State { closed: Vec::new(), pulled: Vec::new() }];
    for o in options {
        out = out
            .into_iter()
            .flat_map(|st| {
                o.iter().map(move |choice| {
                    let mut st = st.clone();
                    st.closed.push(*choice);
                    st
                })
            })
            .collect();
    }
    for _ in &c.pulls {
        out = out
            .into_iter()
            .flat_map(|st| {
                [false, true].map(|p| {
                    let mut st = st.clone();
                    st.pulled.push(p);
                    st
                })
            })
            .collect();
    }
    Some(out)
}

struct Solved {
    set_of: Vec<usize>,
    fixed: HashMap<usize, usize>,
    free: HashMap<usize, usize>,
    floating: HashSet<usize>,
}

fn terminal(c: &Cluster, e: End) -> usize {
    match e {
        End::Node(i) => i,
        End::Rail(v) => c.nets.len() + v,
    }
}

fn arrange(c: &Cluster, st: &State) -> Option<Solved> {
    let n = c.nets.len() + c.vars.len();
    let mut uf = Uf((0..n).collect());
    for (sw, choice) in c.switches.iter().zip(&st.closed) {
        if let Some((i, j)) = choice {
            uf.union(terminal(c, sw.pins[*i].1), terminal(c, sw.pins[*j].1));
        }
    }
    for (p, on) in c.pulls.iter().zip(&st.pulled) {
        if *on {
            uf.union(terminal(c, p.at), terminal(c, End::Rail(c.zero?)));
        }
    }
    let set_of: Vec<usize> = (0..n).map(|t| uf.find(t)).collect();
    let mut fixed: HashMap<usize, usize> = HashMap::new();
    for &v in &c.rails {
        let set = set_of[c.nets.len() + v];
        if let Some(&other) = fixed.get(&set)
            && c.vars[other] != c.vars[v]
        {
            return None;
        }
        fixed.insert(set, v);
    }
    let mut reached: HashSet<usize> = fixed.keys().copied().collect();
    loop {
        let before = reached.len();
        for r in &c.resistors {
            let (a, b) = (set_of[terminal(c, r.a)], set_of[terminal(c, r.b)]);
            if reached.contains(&a) || reached.contains(&b) {
                reached.insert(a);
                reached.insert(b);
            }
        }
        if reached.len() == before {
            break;
        }
    }
    let mut free = HashMap::new();
    let mut floating = HashSet::new();
    for &set in &set_of[..c.nets.len()] {
        if fixed.contains_key(&set) {
            continue;
        }
        if reached.contains(&set) {
            let k = free.len();
            free.entry(set).or_insert(k);
        } else {
            floating.insert(set);
        }
    }
    Some(Solved { set_of, fixed, free, floating })
}

fn voltages(c: &Cluster, sv: &Solved, x: &[f64]) -> HashMap<usize, f64> {
    let m = sv.free.len();
    let mut g = vec![vec![0.0; m + 1]; m];
    let potential = |set: usize| sv.fixed.get(&set).map(|&v| x[v]);
    for r in &c.resistors {
        let (a, b) = (sv.set_of[terminal(c, r.a)], sv.set_of[terminal(c, r.b)]);
        if a == b {
            continue;
        }
        let y = 1.0 / x[r.var];
        match (sv.free.get(&a), sv.free.get(&b)) {
            (Some(&i), Some(&j)) => {
                g[i][i] += y;
                g[j][j] += y;
                g[i][j] -= y;
                g[j][i] -= y;
            }
            (Some(&i), None) => {
                if let Some(v) = potential(b) {
                    g[i][i] += y;
                    g[i][m] += y * v;
                }
            }
            (None, Some(&j)) => {
                if let Some(v) = potential(a) {
                    g[j][j] += y;
                    g[j][m] += y * v;
                }
            }
            (None, None) => {}
        }
    }
    for inp in &c.inputs {
        if let (Some(l), Some(&i)) = (inp.leak, sv.free.get(&sv.set_of[terminal(c, inp.at)])) {
            g[i][m] += x[l];
        }
    }
    for col in 0..m {
        let pivot =
            (col..m).max_by(|&a, &b| g[a][col].abs().total_cmp(&g[b][col].abs())).unwrap_or(col);
        g.swap(col, pivot);
        let p = g[col][col];
        if p.abs() < 1e-300 {
            continue;
        }
        let pivot_row = g[col].clone();
        for (row, r) in g.iter_mut().enumerate() {
            let k = r[col] / p;
            if row != col && k != 0.0 {
                for (v, q) in r[col..].iter_mut().zip(&pivot_row[col..]) {
                    *v -= k * q;
                }
            }
        }
    }
    let mut out: HashMap<usize, f64> = sv.fixed.iter().map(|(&set, &v)| (set, x[v])).collect();
    let floor = c.rails.iter().map(|&v| x[v]).fold(f64::INFINITY, f64::min);
    let ceiling = c.rails.iter().map(|&v| x[v]).fold(f64::NEG_INFINITY, f64::max);
    for (&set, &i) in &sv.free {
        out.insert(set, (g[i][m] / g[i][i]).clamp(floor, ceiling));
    }
    out
}

fn minimise(ranges: &[[f64; 2]], nominal: &[f64], f: &dyn Fn(&[f64]) -> f64) -> (f64, Vec<f64>) {
    let mut x = nominal.to_vec();
    let mut best = f(&x);
    for _ in 0..4 {
        let mut moved = false;
        for k in 0..x.len() {
            let [lo, hi] = ranges[k];
            if lo == hi {
                continue;
            }
            for v in [lo, hi] {
                let keep = x[k];
                x[k] = v;
                let y = f(&x);
                if y < best - 1e-12 {
                    best = y;
                    moved = true;
                } else {
                    x[k] = keep;
                }
            }
        }
        if !moved {
            break;
        }
    }
    (best, x)
}

fn describe(c: &Cluster, st: &State) -> String {
    let mut parts = Vec::new();
    for (sw, choice) in c.switches.iter().zip(&st.closed) {
        parts.push(match choice {
            None => format!("{} open", sw.label),
            Some(_) if sw.pins.len() == 2 => format!("{} closed", sw.label),
            Some((i, j)) => format!("{} {}-{}", sw.label, sw.pins[*i].0, sw.pins[*j].0),
        });
    }
    for (p, on) in c.pulls.iter().zip(&st.pulled) {
        parts.push(format!("{} {}", p.label, if *on { "low" } else { "off" }));
    }
    if parts.is_empty() { String::new() } else { format!(" with {}", parts.join(", ")) }
}

fn mv(v: f64) -> String {
    format!("{:.0} mV", v * 1000.0)
}

fn volts(v: f64) -> String {
    format!("{v:.2} V")
}

fn amps(a: f64) -> String {
    if a >= 1e-3 {
        format!("{:.3} mA", a * 1e3)
    } else if a >= 1e-6 {
        format!("{:.3} uA", a * 1e6)
    } else {
        format!("{:.0} nA", a * 1e9)
    }
}

fn evaluate(s: &Schematic, c: &mut Cluster, d: &mut Diags) {
    let Some(all) = states(c) else {
        d.info(
            format!("net {}", s.nets[c.nets[0]].name),
            "levels not checked: too many switch states",
        );
        return;
    };
    let wants_pull = !c.switches.is_empty() || !c.pulls.is_empty();
    let bidirectional: Vec<usize> =
        c.inputs.iter().enumerate().filter(|(_, i)| i.bidirectional).map(|(k, _)| k).collect();
    let mut reported: HashSet<(usize, &str)> = HashSet::new();
    let quiet: Vec<[f64; 2]> = c
        .vars
        .iter()
        .enumerate()
        .map(|(i, r)| if c.leaks.contains(&i) { [0.0, 0.0] } else { *r })
        .collect();
    for st in &all {
        let Some(sv) = arrange(c, st) else { continue };
        let state = describe(c, st);
        for (k, inp) in c.inputs.iter().enumerate() {
            if !wants_pull && bidirectional.iter().any(|&j| j != k) {
                continue;
            }
            let label = {
                let pin = &s.parts[inp.pin.part].symbol.pins[inp.pin.pin];
                let mut l = s.pin_label(inp.pin);
                if !pin.name.is_empty() && pin.name != pin.number {
                    l = format!("{l} ({})", pin.name);
                }
                l
            };
            let net_name = match inp.at {
                End::Node(i) => s.nets[c.nets[i]].name.clone(),
                End::Rail(_) => s.nets[c.nets[0]].name.clone(),
            };
            let at = format!("net {net_name}");
            let set = sv.set_of[terminal(c, inp.at)];
            if sv.floating.contains(&set) {
                if (inp.bidirectional && !wants_pull) || !reported.insert((k, "float")) {
                    continue;
                }
                d.error(
                    &at,
                    format!(
                        "{label} floats{state}: nothing ties it to a rail, add a pull resistor"
                    ),
                );
                continue;
            }
            let node = |x: &[f64]| voltages(c, &sv, x)[&set];
            let supply = |x: &[f64]| inp.supply.map(|v| x[v]).unwrap_or(0.0);
            let level = |t: Threshold, x: &[f64]| t.at(supply(x));
            let (lo, _) = minimise(&c.vars, &c.nominal, &|x| node(x));
            let (neg_hi, _) = minimise(&c.vars, &c.nominal, &|x| -node(x));
            let hi = -neg_hi;
            let worst = |t: Threshold, high: bool| -> f64 {
                let (v, _) = minimise(&c.vars, &c.nominal, &|x| {
                    if high { -level(t, x) } else { level(t, x) }
                });
                if high { -v } else { v }
            };
            if let Some(max) = inp.levels.max {
                let (m, _) = minimise(&c.vars, &c.nominal, &|x| level(max, x) - node(x));
                if m < 0.0 && reported.insert((k, "max")) {
                    d.error(
                        &at,
                        format!(
                            "{label} is overdriven{state}: up to {}, above its {} limit",
                            volts(hi),
                            volts(worst(max, false))
                        ),
                    );
                }
            }
            if let Some(min) = inp.levels.min {
                let (m, _) = minimise(&c.vars, &c.nominal, &|x| node(x) - level(min, x));
                if m < 0.0 && reported.insert((k, "min")) {
                    d.error(
                        &at,
                        format!(
                            "{label} is driven below its {} limit{state}: down to {}",
                            volts(worst(min, true)),
                            volts(lo)
                        ),
                    );
                }
            }
            let (Some(vih), Some(vil)) = (inp.levels.vih, inp.levels.vil) else { continue };
            if inp.analog {
                continue;
            }
            let high = |r: &[[f64; 2]]| minimise(r, &c.nominal, &|x| node(x) - level(vih, x)).0;
            let low = |r: &[[f64; 2]]| minimise(r, &c.nominal, &|x| level(vil, x) - node(x)).0;
            let (h, l) = (high(&c.vars), low(&c.vars));
            if h >= 0.0 {
                if h < MARGIN && reported.insert((k, "margin")) {
                    d.warn(
                        &at,
                        format!(
                            "{label} is only {} above VIH {}{state}, at {}",
                            mv(h),
                            volts(worst(vih, true)),
                            volts(lo)
                        ),
                    );
                }
                continue;
            }
            if l >= 0.0 {
                if l < MARGIN && reported.insert((k, "margin")) {
                    d.warn(
                        &at,
                        format!(
                            "{label} is only {} below VIL {}{state}, at {}",
                            mv(l),
                            volts(worst(vil, false)),
                            volts(hi)
                        ),
                    );
                }
                continue;
            }
            if !reported.insert((k, "band")) {
                continue;
            }
            let window =
                format!("VIL {}, VIH {}", volts(worst(vil, false)), volts(worst(vih, true)));
            let meant_high = inp.leak.is_some() && high(&quiet) >= 0.0;
            let meant_low = inp.leak.is_some() && low(&quiet) >= 0.0;
            if meant_high || meant_low {
                let leak = inp.leak.map(|v| c.vars[v][1]).unwrap_or(0.0);
                let (way, to, limit) = if meant_high {
                    ("down", lo, format!("below VIH {}", volts(worst(vih, true))))
                } else {
                    ("up", hi, format!("above VIL {}", volts(worst(vil, false))))
                };
                d.error(
                    &at,
                    format!(
                        "{label} pull is too weak{state}: {} of input leakage takes it {way} to {}, {limit}",
                        amps(leak),
                        volts(to)
                    ),
                );
            } else {
                d.error(
                    &at,
                    format!(
                        "{label} sits at {} to {}{state}, not a valid level ({window})",
                        volts(lo),
                        volts(hi)
                    ),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_level_is_volts_a_share_of_supply_or_a_sum() {
        assert_eq!(Threshold::parse("2.0V").unwrap(), Threshold { volts: 2.0, of_supply: 0.0 });
        assert_eq!(Threshold::parse("75%").unwrap(), Threshold { volts: 0.0, of_supply: 0.75 });
        let t = Threshold::parse("100% + 0.3V").unwrap();
        assert!((t.at(3.3) - 3.6).abs() < 1e-9);
        assert!((Threshold::parse("-300mV").unwrap().volts + 0.3).abs() < 1e-9);
        assert!(Threshold::parse("0.7").is_err());
    }

    #[test]
    fn minimise_finds_the_corner_of_a_divider() {
        let ranges = [[4.75, 5.25], [99e3, 101e3], [99e3, 101e3]];
        let nominal = [5.0, 100e3, 100e3];
        let f = |x: &[f64]| x[0] * x[2] / (x[1] + x[2]);
        let (lo, _) = minimise(&ranges, &nominal, &f);
        assert!((lo - 4.75 * 99e3 / 200e3).abs() < 1e-9);
    }
}
