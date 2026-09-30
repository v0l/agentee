use agentee_core::logic::{
    Check, Circuit, Edge, GateOp, Input, Level, LogicResult, LogicSpec, Prim, Stimulus, Trace,
    Wave, fmt_time,
};
use agentee_core::sim::Reading;
use std::cmp::Reverse;
use std::collections::BinaryHeap;

pub const DELTA_LIMIT: usize = 1000;
pub const EVENT_LIMIT: u64 = 50_000_000;
const MESSAGE_LIMIT: usize = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Transition {
    Clean,
    Maybe,
    None,
}

fn rising(prev: Level, now: Level) -> Transition {
    match (prev, now) {
        (Level::L, Level::H) => Transition::Clean,
        (Level::L, Level::X | Level::Z) | (Level::X | Level::Z, Level::H) => Transition::Maybe,
        _ => Transition::None,
    }
}

fn data(l: Level) -> Level {
    if l == Level::Z { Level::X } else { l }
}

fn and(v: &[Level]) -> Level {
    if v.contains(&Level::L) {
        Level::L
    } else if v.iter().all(|x| *x == Level::H) {
        Level::H
    } else {
        Level::X
    }
}

fn gate(op: GateOp, v: &[Level]) -> Level {
    match op {
        GateOp::And => and(v),
        GateOp::Or => {
            if v.contains(&Level::H) {
                Level::H
            } else if v.iter().all(|x| *x == Level::L) {
                Level::L
            } else {
                Level::X
            }
        }
        GateOp::Xor => {
            let mut acc = false;
            for x in v {
                match x.bit() {
                    Some(b) => acc ^= b,
                    None => return Level::X,
                }
            }
            Level::of(acc)
        }
    }
}

fn clocked_update(edge: Transition, next: &[Level], state: &mut [Level]) {
    match edge {
        Transition::Clean => state.copy_from_slice(next),
        Transition::Maybe => {
            for (s, n) in state.iter_mut().zip(next) {
                if s != n {
                    *s = Level::X;
                }
            }
        }
        Transition::None => {}
    }
}

fn set_reset(s: Level, r: Level, state: &mut [Level]) -> bool {
    match (s, r) {
        (Level::H, Level::H) => {
            state[0] = Level::H;
            state[1] = Level::H;
        }
        (Level::H, Level::L) => {
            state[0] = Level::H;
            state[1] = Level::L;
        }
        (Level::L, Level::H) => {
            state[0] = Level::L;
            state[1] = Level::H;
        }
        (Level::L, Level::L) => return false,
        _ => {
            state[0] = Level::X;
            state[1] = Level::X;
        }
    }
    true
}

fn count_up(q: &[Level]) -> Vec<Level> {
    if q.iter().any(|x| x.bit().is_none()) {
        return vec![Level::X; q.len()];
    }
    let n = q.iter().enumerate().fold(0u32, |a, (i, x)| a | (x.bit().unwrap() as u32) << i);
    let n = (n + 1) & ((1 << q.len()) - 1);
    (0..q.len()).map(|i| Level::of(n >> i & 1 == 1)).collect()
}

pub fn initial_state(prim: &Prim) -> Vec<Level> {
    let n = match prim {
        Prim::Dff | Prim::Jk | Prim::Sr | Prim::Dlatch => 2,
        Prim::Counter { .. } => 4,
        Prim::Shift164 => 8,
        Prim::Shift595 => 16,
        _ => 0,
    };
    vec![Level::X; n]
}

pub fn eval(prim: &Prim, now: &[Level], prev: &[Level], state: &mut [Level]) -> Vec<Level> {
    match prim {
        Prim::Const(l) => vec![*l],
        Prim::Gate { op, invert } => {
            let v = gate(*op, now);
            vec![if *invert { v.flip() } else { v }]
        }
        Prim::Tri => vec![match now[1] {
            Level::H => data(now[0]),
            Level::L => Level::Z,
            _ => Level::X,
        }],
        Prim::Dff => {
            if !set_reset(now[2], now[3], state) {
                let next = [data(now[0]), data(now[0]).flip()];
                clocked_update(rising(prev[1], now[1]), &next, state);
            }
            state.to_vec()
        }
        Prim::Jk => {
            if !set_reset(now[3], now[4], state) {
                let q = state[0];
                let n = match (now[0].bit(), now[1].bit()) {
                    (Some(false), Some(false)) => q,
                    (Some(false), Some(true)) => Level::L,
                    (Some(true), Some(false)) => Level::H,
                    (Some(true), Some(true)) => q.flip(),
                    _ => Level::X,
                };
                clocked_update(rising(prev[2], now[2]), &[n, n.flip()], state);
            }
            state.to_vec()
        }
        Prim::Sr => {
            match (now[0], now[1]) {
                (Level::H, Level::L) => state.copy_from_slice(&[Level::H, Level::L]),
                (Level::L, Level::H) => state.copy_from_slice(&[Level::L, Level::H]),
                (Level::L, Level::L) => {}
                _ => state.copy_from_slice(&[Level::X, Level::X]),
            }
            state.to_vec()
        }
        Prim::Dlatch => {
            let d = data(now[0]);
            match (now[2], now[1]) {
                (Level::H, _) => state.copy_from_slice(&[Level::L, Level::H]),
                (Level::L, Level::H) => state.copy_from_slice(&[d, d.flip()]),
                (Level::L, Level::L) => {}
                (Level::L, _) => clocked_update(Transition::Maybe, &[d, d.flip()], state),
                _ => state.copy_from_slice(&[Level::X, Level::X]),
            }
            state.to_vec()
        }
        Prim::Mux2 => {
            let (a, b) = (data(now[0]), data(now[1]));
            vec![match (now[3], now[2]) {
                (Level::L, _) => Level::L,
                (Level::H, Level::L) => a,
                (Level::H, Level::H) => b,
                (Level::H, _) if a == b => a,
                _ => Level::X,
            }]
        }
        Prim::Dec138 => {
            let en = and(&now[3..6]);
            let addr: Option<usize> = now[0..3]
                .iter()
                .enumerate()
                .try_fold(0usize, |acc, (i, x)| x.bit().map(|b| acc | (b as usize) << i));
            (0..8)
                .map(|k| match (en, addr) {
                    (Level::L, _) => Level::L,
                    (Level::H, Some(a)) => Level::of(a == k),
                    _ => Level::X,
                })
                .collect()
        }
        Prim::Counter { sync_reset } => {
            let r = now[0];
            if !sync_reset && r != Level::L {
                let v = if r == Level::H { Level::L } else { Level::X };
                state.iter_mut().for_each(|s| *s = v);
            } else {
                let load = now[7];
                let count = and(&[now[6], now[8]]);
                let next: Vec<Level> = if *sync_reset && r != Level::L {
                    vec![if r == Level::H { Level::L } else { Level::X }; 4]
                } else if load == Level::H {
                    now[2..6].iter().map(|x| data(*x)).collect()
                } else if load != Level::L {
                    vec![Level::X; 4]
                } else {
                    match count {
                        Level::H => count_up(state),
                        Level::L => state.to_vec(),
                        _ => vec![Level::X; 4],
                    }
                };
                clocked_update(rising(prev[1], now[1]), &next, state);
            }
            let mut out = state.to_vec();
            out.push(and(&[now[8], state[0], state[1], state[2], state[3]]));
            out
        }
        Prim::Shift164 => {
            match now[3] {
                Level::H => state.iter_mut().for_each(|s| *s = Level::L),
                Level::L => {
                    let mut next = vec![and(&[now[0], now[1]])];
                    next.extend_from_slice(&state[..7]);
                    clocked_update(rising(prev[2], now[2]), &next, state);
                }
                _ => state.iter_mut().for_each(|s| *s = Level::X),
            }
            state.to_vec()
        }
        Prim::Shift595 => {
            let shifted: Vec<Level> = state[..8].to_vec();
            clocked_update(rising(prev[2], now[2]), &shifted, &mut state[8..]);
            match now[3] {
                Level::H => state[..8].iter_mut().for_each(|s| *s = Level::L),
                Level::L => {
                    let mut next = vec![data(now[0])];
                    next.extend_from_slice(&shifted[..7]);
                    clocked_update(rising(prev[1], now[1]), &next, &mut state[..8]);
                }
                _ => state[..8].iter_mut().for_each(|s| *s = Level::X),
            }
            let mut out: Vec<Level> = state[8..]
                .iter()
                .map(|o| match now[4] {
                    Level::H => *o,
                    Level::L => Level::Z,
                    _ => Level::X,
                })
                .collect();
            out.push(state[7]);
            out
        }
        Prim::Truth(rows) => {
            let hit = rows
                .iter()
                .find(|(pat, _)| pat.iter().zip(now).all(|(p, x)| p.is_none() || *p == x.bit()));
            match hit {
                Some((_, out)) => out.clone(),
                None => vec![Level::X; rows.first().map(|r| r.1.len()).unwrap_or(0)],
            }
        }
    }
}

fn invert_out(l: Level) -> Level {
    if l == Level::Z { l } else { l.flip() }
}

fn encode(l: Level) -> u8 {
    match l {
        Level::L => 0,
        Level::H => 1,
        Level::X => 2,
        Level::Z => 3,
    }
}

fn decode(b: u8) -> Level {
    [Level::L, Level::H, Level::X, Level::Z][b as usize & 3]
}

#[derive(Clone, Debug, Default)]
pub struct Run {
    pub changes: Vec<Vec<(u64, Level)>>,
    pub end: u64,
    pub events: u64,
    pub problems: Vec<String>,
    pub contentions: usize,
    pub violations: usize,
    pub oscillation: bool,
}

struct Engine<'a> {
    c: &'a Circuit,
    slot_val: Vec<Level>,
    slot_weak: Vec<bool>,
    slot_owner: Vec<String>,
    slot_net: Vec<usize>,
    pending: Vec<Level>,
    net_slots: Vec<Vec<usize>>,
    net_val: Vec<Level>,
    fanout: Vec<Vec<usize>>,
    out_slot: Vec<Vec<Option<usize>>>,
    prev: Vec<Vec<Level>>,
    state: Vec<Vec<Level>>,
    changed_at: Vec<Vec<Option<u64>>>,
    last_edge: Vec<Option<u64>>,
    queue: BinaryHeap<Reverse<(u64, u64, usize, u8)>>,
    seq: u64,
    run: Run,
    contention_seen: Vec<usize>,
    stimulus_slots: Vec<(usize, Wave)>,
}

impl<'a> Engine<'a> {
    fn new(c: &'a Circuit, stimuli: &[Stimulus]) -> Engine<'a> {
        let n = c.nets.len();
        let mut e = Engine {
            c,
            slot_val: Vec::new(),
            slot_weak: Vec::new(),
            slot_owner: Vec::new(),
            slot_net: Vec::new(),
            pending: Vec::new(),
            net_slots: vec![Vec::new(); n],
            net_val: vec![Level::Z; n],
            fanout: vec![Vec::new(); n],
            out_slot: Vec::new(),
            prev: Vec::new(),
            state: Vec::new(),
            changed_at: Vec::new(),
            last_edge: vec![None; c.cells.len()],
            queue: BinaryHeap::new(),
            seq: 0,
            run: Run { changes: vec![Vec::new(); n], ..Default::default() },
            contention_seen: vec![0; n],
            stimulus_slots: Vec::new(),
        };
        for (ci, cell) in c.cells.iter().enumerate() {
            let mut slots = Vec::new();
            for o in &cell.outputs {
                slots.push(o.net.map(|net| e.add_slot(net, cell.weak, &cell.part)));
            }
            e.out_slot.push(slots);
            for i in &cell.inputs {
                if let Input::Net { net, .. } = i
                    && !e.fanout[*net].contains(&ci)
                {
                    e.fanout[*net].push(ci);
                }
            }
            e.state.push(initial_state(&cell.prim));
            e.changed_at.push(vec![None; cell.inputs.len()]);
        }
        for ci in 0..c.cells.len() {
            let now = e.inputs(ci);
            e.prev.push(now);
        }
        for s in stimuli {
            let slot = e.add_slot(s.net, false, &format!("stimulus {}", s.name));
            e.slot_owner[slot] = format!("the stimulus on {}", s.name);
            e.stimulus_slots.push((slot, s.wave.clone()));
        }
        e
    }
}

impl Engine<'_> {
    fn add_slot(&mut self, net: usize, weak: bool, owner: &str) -> usize {
        self.slot_val.push(Level::Z);
        self.pending.push(Level::Z);
        self.slot_weak.push(weak);
        self.slot_owner.push(owner.to_string());
        self.slot_net.push(net);
        self.net_slots[net].push(self.slot_val.len() - 1);
        self.slot_val.len() - 1
    }

    fn inputs(&self, ci: usize) -> Vec<Level> {
        self.c.cells[ci]
            .inputs
            .iter()
            .map(|i| match i {
                Input::Net { net, invert } => {
                    let v = self.net_val[*net];
                    if *invert { v.flip() } else { v }
                }
                Input::Fixed(l) => *l,
            })
            .collect()
    }

    fn schedule(&mut self, slot: usize, t: u64, v: Level) {
        if self.pending[slot] == v {
            return;
        }
        self.pending[slot] = v;
        self.seq += 1;
        self.queue.push(Reverse((t, self.seq, slot, encode(v))));
    }

    fn resolve(&mut self, net: usize, t: u64) -> Level {
        let mut strong = [false; 3];
        let mut weak = [false; 3];
        for &s in &self.net_slots[net] {
            let v = self.slot_val[s];
            if v == Level::Z {
                continue;
            }
            let k = encode(v) as usize;
            if self.slot_weak[s] { weak[k] = true } else { strong[k] = true }
        }
        let pick = |v: [bool; 3]| match v {
            [false, false, false] => None,
            [true, false, false] => Some(Level::L),
            [false, true, false] => Some(Level::H),
            _ => Some(Level::X),
        };
        if strong[0] && strong[1] {
            self.contention(net, t);
        }
        pick(strong).or(pick(weak)).unwrap_or(Level::Z)
    }

    fn contention(&mut self, net: usize, t: u64) {
        self.contention_seen[net] += 1;
        self.run.contentions += 1;
        if self.contention_seen[net] > 1 || self.run.problems.len() >= MESSAGE_LIMIT {
            return;
        }
        let drivers: Vec<String> = self.net_slots[net]
            .iter()
            .filter(|s| !self.slot_weak[**s] && self.slot_val[**s].bit().is_some())
            .map(|s| format!("{} drives {}", self.slot_owner[*s], self.slot_val[*s].char()))
            .collect();
        self.run.problems.push(format!(
            "contention on {} at {}: {}",
            self.c.nets[net],
            fmt_time(t),
            drivers.join(", ")
        ));
    }

    fn violation(&mut self, msg: String) {
        self.run.violations += 1;
        if self.run.problems.len() < MESSAGE_LIMIT {
            self.run.problems.push(msg);
        }
    }

    fn timing(&mut self, ci: usize, now: &[Level], prev: &[Level], t: u64) {
        let cell = &self.c.cells[ci];
        let Some(ck) = cell.prim.clocked() else { return };
        let (setup, hold) = (cell.setup, cell.hold);
        let quiet = ck.resets.iter().all(|i| now[*i] == Level::L);
        let mut found = Vec::new();
        for &i in ck.data {
            if now[i] != prev[i] {
                if quiet
                    && let Some(e) = self.last_edge[ci]
                    && t > e
                    && t - e < hold
                {
                    found.push(format!(
                        "hold: {} {} changed {} after the {} edge at {} (needs {})",
                        cell.part,
                        cell.input_names[i],
                        fmt_time(t - e),
                        cell.input_names[ck.clock],
                        fmt_time(e),
                        fmt_time(hold)
                    ));
                }
                self.changed_at[ci][i] = Some(t);
            }
        }
        if rising(prev[ck.clock], now[ck.clock]) == Transition::Clean {
            if quiet {
                for &i in ck.data {
                    if let Some(c) = self.changed_at[ci][i]
                        && t - c < setup
                    {
                        found.push(format!(
                            "setup: {} {} changed {} before the {} edge at {} (needs {})",
                            cell.part,
                            cell.input_names[i],
                            fmt_time(t - c),
                            cell.input_names[ck.clock],
                            fmt_time(t),
                            fmt_time(setup)
                        ));
                    }
                }
            }
            self.last_edge[ci] = Some(t);
        }
        for m in found {
            self.violation(m);
        }
    }

    fn step(&mut self, ci: usize, t: u64) {
        let now = self.inputs(ci);
        let prev = std::mem::take(&mut self.prev[ci]);
        self.timing(ci, &now, &prev, t);
        let cell = &self.c.cells[ci];
        let outs = eval(&cell.prim, &now, &prev, &mut self.state[ci]);
        self.prev[ci] = now;
        for (k, o) in cell.outputs.iter().enumerate() {
            if let Some(slot) = self.out_slot[ci][k] {
                let v = outs.get(k).copied().unwrap_or(Level::X);
                let v = if o.invert { invert_out(v) } else { v };
                self.schedule(slot, t + o.delay, v);
            }
        }
    }

    fn record(&mut self, net: usize, t: u64, v: Level) {
        let ch = &mut self.run.changes[net];
        if let Some(last) = ch.last_mut()
            && last.0 == t
        {
            last.1 = v;
            let before = if ch.len() >= 2 { ch[ch.len() - 2].1 } else { Level::Z };
            if before == v {
                ch.pop();
            }
            return;
        }
        ch.push((t, v));
    }
}

pub fn simulate(c: &Circuit, stimuli: &[Stimulus], duration: u64) -> Run {
    let mut e = Engine::new(c, stimuli);
    for (slot, wave) in std::mem::take(&mut e.stimulus_slots) {
        match wave {
            Wave::Steps(steps) => {
                for (t, v) in steps.into_iter().filter(|s| s.0 <= duration) {
                    e.schedule(slot, t, v);
                }
            }
            Wave::Clock { period, high, phase } => {
                if phase > 0 {
                    e.schedule(slot, 0, Level::L);
                }
                let mut t = phase;
                while t <= duration && e.seq < EVENT_LIMIT {
                    e.schedule(slot, t, Level::H);
                    e.schedule(slot, t + high, Level::L);
                    t += period;
                }
            }
        }
    }
    for ci in 0..c.cells.len() {
        e.step(ci, 0);
    }
    let mut end = duration;
    'outer: while let Some(&Reverse((t, ..))) = e.queue.peek() {
        if t > duration {
            break;
        }
        let mut deltas = 0;
        loop {
            let mut dirty = Vec::new();
            while let Some(&Reverse((tt, _, slot, v))) = e.queue.peek() {
                if tt != t {
                    break;
                }
                e.queue.pop();
                e.run.events += 1;
                e.slot_val[slot] = decode(v);
                dirty.push(e.slot_net[slot]);
            }
            dirty.sort_unstable();
            dirty.dedup();
            let mut cells = Vec::new();
            let mut changed = Vec::new();
            for net in dirty {
                let v = e.resolve(net, t);
                if v != e.net_val[net] {
                    e.net_val[net] = v;
                    e.record(net, t, v);
                    cells.extend_from_slice(&e.fanout[net]);
                    changed.push(net);
                }
            }
            cells.sort_unstable();
            cells.dedup();
            for ci in cells {
                e.step(ci, t);
            }
            if e.queue.peek().map(|x| x.0.0) != Some(t) {
                break;
            }
            deltas += 1;
            if deltas > DELTA_LIMIT {
                let names: Vec<&str> =
                    changed.iter().take(6).map(|n| c.nets[*n].as_str()).collect();
                e.run.problems.push(format!(
                    "zero-delay oscillation at {} through {}; the run stopped there",
                    fmt_time(t),
                    names.join(", ")
                ));
                e.run.oscillation = true;
                end = t;
                break 'outer;
            }
        }
        if e.run.events > EVENT_LIMIT {
            e.run.problems.push(format!(
                "more than {EVENT_LIMIT} events by {}; the run stopped there",
                fmt_time(t)
            ));
            end = t;
            break;
        }
    }
    let mut run = e.run;
    run.end = end;
    run
}

fn value(ch: &[(u64, Level)], t: u64, before: bool) -> Level {
    let k = ch.partition_point(|c| if before { c.0 < t } else { c.0 <= t });
    if k == 0 { Level::Z } else { ch[k - 1].1 }
}

fn edges(ch: &[(u64, Level)], edge: Edge, from: u64) -> Vec<u64> {
    let mut prev = Level::Z;
    let mut out = Vec::new();
    for (t, v) in ch {
        let hit = match edge {
            Edge::Rising => prev == Level::L && *v == Level::H,
            Edge::Falling => prev == Level::H && *v == Level::L,
        };
        if hit && *t >= from {
            out.push(*t);
        }
        prev = *v;
    }
    out
}

fn show(p: &[Option<Level>]) -> String {
    p.iter().map(|x| x.map(Level::char).unwrap_or('-')).collect()
}

fn matches(want: &[Option<Level>], got: &[Level]) -> bool {
    want.iter().zip(got).all(|(w, g)| w.is_none() || *w == Some(*g))
}

pub fn check(spec: &LogicSpec, run: &Run) -> (usize, Vec<String>) {
    let mut passed = 0;
    let mut failures = Vec::new();
    for x in &spec.expects {
        let sample = |t: u64, before: bool| -> Vec<Level> {
            x.nets.iter().map(|n| value(&run.changes[*n], t, before)).collect()
        };
        let failure = match &x.check {
            Check::At { time, value: want } => {
                if *time > run.end {
                    Some(format!(
                        "{}: {} was not reached, the run stopped at {}",
                        x.label,
                        fmt_time(*time),
                        fmt_time(run.end)
                    ))
                } else {
                    let got = sample(*time, false);
                    (!matches(want, &got)).then(|| {
                        format!(
                            "{}: expected {} at {}, got {}",
                            x.label,
                            show(want),
                            fmt_time(*time),
                            got.iter().map(|l| l.char()).collect::<String>()
                        )
                    })
                }
            }
            Check::Sequence { clock, clock_name, edge, from, values } => {
                let at = edges(&run.changes[*clock], *edge, *from);
                let dir = if *edge == Edge::Rising { "rising" } else { "falling" };
                let mut fail = None;
                for (k, want) in values.iter().enumerate() {
                    let Some(t) = at.get(k) else {
                        fail = Some(format!(
                            "{}: {} has {} {dir} edge(s) from {}, the sequence needs {}",
                            x.label,
                            clock_name,
                            at.len(),
                            fmt_time(*from),
                            values.len()
                        ));
                        break;
                    };
                    let got = sample(*t, true);
                    if !matches(want, &got) {
                        fail = Some(format!(
                            "{}: {dir} edge {} of {} at {}: expected {}, got {}",
                            x.label,
                            k + 1,
                            clock_name,
                            fmt_time(*t),
                            show(want),
                            got.iter().map(|l| l.char()).collect::<String>()
                        ));
                        break;
                    }
                }
                fail
            }
        };
        match failure {
            Some(f) => failures.push(f),
            None => passed += 1,
        }
    }
    (passed, failures)
}

fn vcd_id(mut k: usize) -> String {
    let mut s = String::new();
    loop {
        s.push((b'!' + (k % 94) as u8) as char);
        k /= 94;
        if k == 0 {
            return s;
        }
        k -= 1;
    }
}

pub fn vcd(name: &str, traces: &[Trace]) -> String {
    let clean =
        |s: &str| s.chars().map(|c| if c.is_whitespace() { '_' } else { c }).collect::<String>();
    let mut out = String::new();
    out.push_str("$version agentee logic $end\n$timescale 1ps $end\n");
    out.push_str(&format!("$scope module {} $end\n", clean(name)));
    for (k, t) in traces.iter().enumerate() {
        out.push_str(&format!("$var wire 1 {} {} $end\n", vcd_id(k), clean(&t.name)));
    }
    out.push_str("$upscope $end\n$enddefinitions $end\n");
    let mut events: Vec<(u64, usize, char)> = Vec::new();
    for (k, t) in traces.iter().enumerate() {
        for (time, c) in t.times.iter().zip(t.values.chars()) {
            events.push((*time, k, c));
        }
    }
    events.sort_by_key(|e| (e.0, e.1));
    out.push_str("#0\n$dumpvars\n");
    for (k, t) in traces.iter().enumerate() {
        let first = t.times.first().filter(|x| **x == 0).map(|_| t.values.chars().next().unwrap());
        out.push_str(&format!("{}{}\n", first.unwrap_or('z'), vcd_id(k)));
    }
    out.push_str("$end\n");
    let mut last = 0;
    for (time, k, c) in events.into_iter().filter(|e| e.0 > 0) {
        if time != last {
            out.push_str(&format!("#{time}\n"));
            last = time;
        }
        out.push_str(&format!("{c}{}\n", vcd_id(k)));
    }
    out
}

pub fn run(spec: &LogicSpec, name: &str, spec_hash: u64, vcd_name: &str) -> (LogicResult, String) {
    let started = std::time::Instant::now();
    let r = simulate(&spec.circuit, &spec.stimuli, spec.duration);
    let (passed, failed) = check(spec, &r);
    let traces: Vec<Trace> = spec
        .record
        .iter()
        .map(|(n, id)| Trace {
            name: n.clone(),
            times: r.changes[*id].iter().map(|c| c.0).collect(),
            values: r.changes[*id].iter().map(|c| c.1.char()).collect(),
        })
        .collect();
    let reading = |label: &str, value: f64, unit: &str, detail: String| Reading {
        label: label.into(),
        value,
        unit: unit.into(),
        detail,
    };
    let readings = vec![
        reading("assertions passed", passed as f64, "", format!("of {}", spec.expects.len())),
        reading("assertions failed", failed.len() as f64, "", String::new()),
        reading("contention", r.contentions as f64, "", "strong drivers disagreeing".into()),
        reading("timing violations", r.violations as f64, "", "setup and hold".into()),
        reading(
            "cells",
            spec.circuit.cells.len() as f64,
            "",
            format!("{} nets", spec.circuit.nets.len()),
        ),
        reading("events", r.events as f64, "", format!("to {}", fmt_time(r.end))),
    ];
    let mut failures = r.problems.clone();
    failures.extend(failed);
    let result = LogicResult {
        name: name.to_string(),
        kind: "logic".into(),
        spec_hash,
        duration_ps: spec.duration,
        end_ps: r.end,
        traces,
        passed,
        failures,
        readings,
        events: r.events,
        seconds: started.elapsed().as_secs_f64(),
        vcd: vcd_name.to_string(),
    };
    let text = vcd(name, &result.traces);
    (result, text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use Level::{H, L, X, Z};
    use agentee_core::logic::{Cell, Expect, Output};

    fn cell(prim: Prim, ins: &[usize], outs: &[usize], delay: u64) -> Cell {
        Cell {
            part: format!("U{}", outs.first().copied().unwrap_or(0)),
            input_names: (0..ins.len()).map(|i| format!("I{i}")).collect(),
            inputs: ins.iter().map(|n| Input::Net { net: *n, invert: false }).collect(),
            outputs: outs.iter().map(|n| Output { net: Some(*n), invert: false, delay }).collect(),
            prim,
            weak: false,
            setup: 0,
            hold: 0,
        }
    }

    fn circuit(nets: usize, cells: Vec<Cell>) -> Circuit {
        Circuit { nets: (0..nets).map(|i| format!("n{i}")).collect(), cells }
    }

    fn steps(net: usize, v: &[(u64, Level)]) -> Stimulus {
        Stimulus { net, name: format!("n{net}"), wave: Wave::Steps(v.to_vec()) }
    }

    fn at(r: &Run, net: usize, t: u64) -> Level {
        value(&r.changes[net], t, false)
    }

    fn g(op: GateOp, invert: bool, v: &[Level]) -> Level {
        eval(&Prim::Gate { op, invert }, v, v, &mut [])[0]
    }

    #[test]
    fn gates_follow_their_tables() {
        use GateOp::*;
        let all = [[L, L], [L, H], [H, L], [H, H]];
        let want = |f: fn(bool, bool) -> bool| all.map(|p| Level::of(f(p[0] == H, p[1] == H)));
        let got = |op, inv| all.map(|p| g(op, inv, &p));
        assert_eq!(got(And, false), want(|a, b| a && b));
        assert_eq!(got(And, true), want(|a, b| !(a && b)));
        assert_eq!(got(Or, false), want(|a, b| a || b));
        assert_eq!(got(Or, true), want(|a, b| !(a || b)));
        assert_eq!(got(Xor, false), want(|a, b| a ^ b));
        assert_eq!(got(Xor, true), want(|a, b| !(a ^ b)));
        assert_eq!(g(And, true, &[H]), L);
        assert_eq!(g(And, false, &[L]), L);
        assert_eq!(g(And, false, &[H, H, H, L]), L);
        assert_eq!(g(Or, false, &[L, L, L, H]), H);
        assert_eq!(g(Xor, false, &[H, H, H]), H);
    }

    #[test]
    fn unknowns_propagate_only_when_they_matter() {
        use GateOp::*;
        assert_eq!(g(And, false, &[L, X]), L);
        assert_eq!(g(And, false, &[H, X]), X);
        assert_eq!(g(And, false, &[H, Z]), X);
        assert_eq!(g(Or, false, &[H, X]), H);
        assert_eq!(g(Or, true, &[L, Z]), X);
        assert_eq!(g(Xor, false, &[L, X]), X);
        assert_eq!(eval(&Prim::Mux2, &[H, H, X, H], &[L; 4], &mut [])[0], H);
        assert_eq!(eval(&Prim::Mux2, &[L, H, X, H], &[L; 4], &mut [])[0], X);
        let c = circuit(2, vec![cell(Prim::Gate { op: And, invert: true }, &[0], &[1], 1000)]);
        let r = simulate(&c, &[], 10_000);
        assert_eq!(at(&r, 1, 5_000), X);
    }

    #[test]
    fn tri_state_buffer_releases_the_net() {
        assert_eq!(eval(&Prim::Tri, &[H, H], &[L; 2], &mut []), vec![H]);
        assert_eq!(eval(&Prim::Tri, &[L, H], &[L; 2], &mut []), vec![L]);
        assert_eq!(eval(&Prim::Tri, &[H, L], &[L; 2], &mut []), vec![Z]);
        assert_eq!(eval(&Prim::Tri, &[H, X], &[L; 2], &mut []), vec![X]);
    }

    #[test]
    fn d_flip_flop_clocks_sets_and_resets() {
        let mut s = initial_state(&Prim::Dff);
        let e = |now: [Level; 4], prev: [Level; 4], s: &mut Vec<Level>| {
            eval(&Prim::Dff, &now, &prev, s)
        };
        assert_eq!(e([H, L, L, L], [H, L, L, L], &mut s), vec![X, X]);
        assert_eq!(e([H, H, L, L], [H, L, L, L], &mut s), vec![H, L]);
        assert_eq!(e([L, H, L, L], [H, H, L, L], &mut s), vec![H, L]);
        assert_eq!(e([L, L, L, L], [L, H, L, L], &mut s), vec![H, L]);
        assert_eq!(e([L, H, L, L], [L, L, L, L], &mut s), vec![L, H]);
        assert_eq!(e([L, H, H, L], [L, H, L, L], &mut s), vec![H, L]);
        assert_eq!(e([L, H, L, H], [L, H, H, L], &mut s), vec![L, H]);
        assert_eq!(e([L, H, H, H], [L, H, L, H], &mut s), vec![H, H]);
        let mut s = vec![L, H];
        assert_eq!(e([H, X, L, L], [H, L, L, L], &mut s), vec![X, X]);
    }

    #[test]
    fn jk_flip_flop_toggles() {
        let mut s = vec![L, H];
        let clk =
            |j, k, s: &mut Vec<Level>| eval(&Prim::Jk, &[j, k, H, L, L], &[j, k, L, L, L], s)[0];
        assert_eq!(clk(H, H, &mut s), H);
        assert_eq!(clk(H, H, &mut s), L);
        assert_eq!(clk(H, L, &mut s), H);
        assert_eq!(clk(L, L, &mut s), H);
        assert_eq!(clk(L, H, &mut s), L);
        assert_eq!(clk(X, L, &mut s), X);
    }

    #[test]
    fn latches_hold_and_follow() {
        let mut s = initial_state(&Prim::Sr);
        assert_eq!(eval(&Prim::Sr, &[H, L], &[L, L], &mut s), vec![H, L]);
        assert_eq!(eval(&Prim::Sr, &[L, L], &[H, L], &mut s), vec![H, L]);
        assert_eq!(eval(&Prim::Sr, &[L, H], &[L, L], &mut s), vec![L, H]);
        assert_eq!(eval(&Prim::Sr, &[H, H], &[L, H], &mut s), vec![X, X]);
        let mut s = initial_state(&Prim::Dlatch);
        assert_eq!(eval(&Prim::Dlatch, &[H, H, L], &[L; 3], &mut s), vec![H, L]);
        assert_eq!(eval(&Prim::Dlatch, &[L, H, L], &[L; 3], &mut s), vec![L, H]);
        assert_eq!(eval(&Prim::Dlatch, &[H, L, L], &[L; 3], &mut s), vec![L, H]);
        assert_eq!(eval(&Prim::Dlatch, &[H, H, H], &[L; 3], &mut s), vec![L, H]);
    }

    #[test]
    fn mux_and_decoder_select() {
        assert_eq!(eval(&Prim::Mux2, &[L, H, L, H], &[L; 4], &mut []), vec![L]);
        assert_eq!(eval(&Prim::Mux2, &[L, H, H, H], &[L; 4], &mut []), vec![H]);
        assert_eq!(eval(&Prim::Mux2, &[L, H, H, L], &[L; 4], &mut []), vec![L]);
        let y = eval(&Prim::Dec138, &[H, L, H, H, H, H], &[L; 6], &mut []);
        assert_eq!(y, vec![L, L, L, L, L, H, L, L]);
        let y = eval(&Prim::Dec138, &[H, L, H, H, L, H], &[L; 6], &mut []);
        assert_eq!(y, vec![L; 8]);
        let y = eval(&Prim::Dec138, &[X, L, H, H, H, H], &[L; 6], &mut []);
        assert_eq!(y, vec![X; 8]);
    }

    #[test]
    fn counter_counts_loads_and_carries() {
        let p = Prim::Counter { sync_reset: false };
        let mut s = initial_state(&p);
        let base = [L, L, H, L, H, L, H, L, H];
        let mut now = base;
        now[0] = H;
        assert_eq!(eval(&p, &now, &now, &mut s), vec![L, L, L, L, L]);
        let tick = |s: &mut Vec<Level>, now: [Level; 9]| {
            let mut prev = now;
            prev[1] = L;
            let mut n = now;
            n[1] = H;
            eval(&p, &n, &prev, s)
        };
        for _ in 0..14 {
            tick(&mut s, base);
        }
        assert_eq!(tick(&mut s, base), vec![H, H, H, H, H]);
        assert_eq!(tick(&mut s, base), vec![L, L, L, L, L]);
        let mut load = base;
        load[7] = H;
        assert_eq!(tick(&mut s, load), vec![H, L, H, L, L]);
        let q = Prim::Counter { sync_reset: true };
        let mut s = vec![H, H, L, L];
        let mut r = base;
        r[0] = H;
        assert_eq!(eval(&q, &r, &r, &mut s)[..4], [H, H, L, L]);
        let mut prev = r;
        prev[1] = L;
        r[1] = H;
        assert_eq!(eval(&q, &r, &prev, &mut s)[..4], [L, L, L, L]);
    }

    #[test]
    fn shift_registers_shift_and_latch() {
        let mut s = initial_state(&Prim::Shift164);
        eval(&Prim::Shift164, &[L, H, L, H], &[L; 4], &mut s);
        let clk = |s: &mut Vec<Level>, a| eval(&Prim::Shift164, &[a, H, H, L], &[a, H, L, L], s);
        clk(&mut s, H);
        clk(&mut s, L);
        assert_eq!(clk(&mut s, H), vec![H, L, H, L, L, L, L, L]);
        let p = Prim::Shift595;
        let mut s = initial_state(&p);
        eval(&p, &[L, L, L, H, H], &[L; 5], &mut s);
        let shift = |s: &mut Vec<Level>, d| eval(&p, &[d, H, L, L, H], &[d, L, L, L, H], s);
        shift(&mut s, H);
        let out = shift(&mut s, H);
        assert_eq!(out[..8], [X; 8]);
        assert_eq!(out[8], L);
        let out = eval(&p, &[L, L, H, L, H], &[L, L, L, L, H], &mut s);
        assert_eq!(out[..3], [H, H, L]);
        let out = eval(&p, &[L, L, H, L, L], &[L, L, H, L, H], &mut s);
        assert_eq!(out[..8], [Z; 8]);
    }

    #[test]
    fn truth_table_matches_first_row() {
        let p = Prim::Truth(vec![
            (vec![Some(true), None], vec![H]),
            (vec![Some(false), Some(true)], vec![Z]),
        ]);
        assert_eq!(eval(&p, &[H, X], &[L; 2], &mut []), vec![H]);
        assert_eq!(eval(&p, &[L, H], &[L; 2], &mut []), vec![Z]);
        assert_eq!(eval(&p, &[L, L], &[L; 2], &mut []), vec![X]);
        assert_eq!(eval(&p, &[X, H], &[L; 2], &mut []), vec![X]);
    }

    #[test]
    fn nets_resolve_strong_over_weak_and_flag_contention() {
        let mut pull = cell(Prim::Const(H), &[], &[0], 0);
        pull.weak = true;
        let mut down = cell(Prim::Const(L), &[], &[1], 0);
        down.weak = true;
        let mut up = cell(Prim::Const(H), &[], &[1], 0);
        up.weak = true;
        let c = circuit(
            5,
            vec![
                pull,
                cell(Prim::Tri, &[3, 4], &[0], 0),
                down,
                up,
                cell(Prim::Const(L), &[], &[2], 0),
            ],
        );
        let stim = [steps(3, &[(0, L)]), steps(2, &[(5_000, H)]), steps(4, &[(0, L), (5_000, H)])];
        let r = simulate(&c, &stim, 10_000);
        assert_eq!(at(&r, 0, 1_000), H);
        assert_eq!(at(&r, 1, 1_000), X);
        assert_eq!(at(&r, 2, 1_000), L);
        assert_eq!(at(&r, 0, 6_000), L);
        assert_eq!(at(&r, 2, 6_000), X);
        assert_eq!(r.contentions, 1);
        assert!(r.problems[0].starts_with("contention on n2 at 5ns"), "{}", r.problems[0]);
    }

    #[test]
    fn zero_delay_loop_stops_the_run() {
        let nand = Prim::Gate { op: GateOp::And, invert: true };
        let c = circuit(2, vec![cell(nand.clone(), &[0, 1], &[0], 0)]);
        let r = simulate(&c, &[steps(1, &[(0, L), (10_000, H)])], 50_000);
        assert!(r.oscillation);
        assert_eq!(r.end, 10_000);
        assert!(r.problems[0].contains("zero-delay oscillation at 10ns"));
        let c = circuit(2, vec![cell(nand, &[0, 1], &[0], 1_000)]);
        let r = simulate(&c, &[steps(1, &[(0, L), (10_000, H)])], 50_000);
        assert!(!r.oscillation);
        assert!(r.changes[0].len() > 30);
    }

    #[test]
    fn setup_and_hold_are_checked_against_the_clock() {
        let mut ff = cell(Prim::Dff, &[0, 1], &[2, 3], 1_000);
        ff.inputs.push(Input::Fixed(L));
        ff.inputs.push(Input::Fixed(L));
        ff.input_names = vec!["D".into(), "CLK".into(), "S".into(), "R".into()];
        ff.setup = 5_000;
        ff.hold = 2_000;
        let c = circuit(4, vec![ff]);
        let clock = Stimulus {
            net: 1,
            name: "n1".into(),
            wave: Wave::Clock { period: 100_000, high: 50_000, phase: 100_000 },
        };
        let r = simulate(&c, &[clock.clone(), steps(0, &[(0, L), (50_000, H)])], 150_000);
        assert_eq!(r.violations, 0);
        assert_eq!(at(&r, 2, 120_000), H);
        let r = simulate(&c, &[clock.clone(), steps(0, &[(0, L), (197_000, H)])], 250_000);
        assert_eq!(r.violations, 1);
        assert!(
            r.problems[0].starts_with("setup: U2 D changed 3ns before the CLK edge at 200ns"),
            "{}",
            r.problems[0]
        );
        let r = simulate(&c, &[clock, steps(0, &[(0, L), (101_000, H)])], 150_000);
        assert_eq!(r.violations, 1);
        assert!(
            r.problems[0].starts_with("hold: U2 D changed 1ns after the CLK edge at 100ns"),
            "{}",
            r.problems[0]
        );
    }

    #[test]
    fn expectations_sample_levels_and_clocked_sequences() {
        let c = circuit(
            7,
            vec![{
                let mut k =
                    cell(Prim::Counter { sync_reset: false }, &[0, 1], &[2, 3, 4, 5, 6], 1_000);
                k.inputs.extend([Input::Fixed(L); 4]);
                k.inputs.extend([Input::Fixed(H), Input::Fixed(L), Input::Fixed(H)]);
                k
            }],
        );
        let spec = LogicSpec {
            duration: 1_000_000,
            record: vec![("n2".into(), 2), ("n3".into(), 3)],
            stimuli: vec![
                steps(0, &[(0, H), (150_000, L)]),
                Stimulus {
                    net: 1,
                    name: "n1".into(),
                    wave: Wave::Clock { period: 100_000, high: 50_000, phase: 100_000 },
                },
            ],
            expects: vec![
                Expect {
                    label: "a".into(),
                    nets: vec![3, 2],
                    names: vec![],
                    check: Check::At { time: 360_000, value: vec![Some(H), Some(L)] },
                },
                Expect {
                    label: "b".into(),
                    nets: vec![3, 2],
                    names: vec![],
                    check: Check::Sequence {
                        clock: 1,
                        clock_name: "n1".into(),
                        edge: Edge::Rising,
                        from: 200_000,
                        values: vec![
                            vec![Some(L), Some(L)],
                            vec![Some(L), Some(H)],
                            vec![None, Some(L)],
                        ],
                    },
                },
                Expect {
                    label: "c".into(),
                    nets: vec![2],
                    names: vec![],
                    check: Check::At { time: 360_000, value: vec![Some(H)] },
                },
            ],
            ..Default::default()
        };
        let spec = LogicSpec { circuit: c, ..spec };
        let (res, text) = run(&spec, "count", 7, "count.vcd");
        assert_eq!(res.passed, 2);
        assert_eq!(res.failures, vec!["c: expected 1 at 360ns, got 0".to_string()]);
        assert_eq!(res.traces[0].values.chars().next(), Some('0'));
        assert!(text.contains("$timescale 1ps $end"));
        assert!(text.contains("$var wire 1 ! n2 $end"));
        assert!(text.contains("#201000\n1!"));
    }

    #[test]
    fn logic_example_counts_and_decodes() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/logic");
        let p = agentee_core::Project::load(&root).unwrap();
        let s = p.sims.iter().find(|s| s.name == "counter").unwrap();
        let spec = s.item.logic.as_ref().unwrap();
        let (res, _) = run(spec, "counter", 0, "counter.vcd");
        assert_eq!(res.failures, Vec::<String>::new());
        assert_eq!(res.passed, spec.expects.len());
        let mut broken = spec.clone();
        broken.circuit.cells.retain(|c| c.part != "R1");
        let (res, _) = run(&broken, "counter", 0, "counter.vcd");
        assert!(res.passed < spec.expects.len());
    }
}
