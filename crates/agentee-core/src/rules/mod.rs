mod clearance;
mod context;
mod edge;
mod holes;
mod isolation;

pub use clearance::{ClassClearance, NetClearance};
pub use context::{Context, Placed, Planned};
pub use edge::CopperToEdge;
pub use holes::{HoleToCopper, HoleToHole, Which};
pub use isolation::Isolation;

use crate::board::Board;
use crate::layout::LayoutNet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layer {
    pub index: usize,
    pub outer: bool,
}

impl Layer {
    pub fn new(index: usize, count: usize) -> Layer {
        Layer { index, outer: index == 0 || index + 1 == count }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Violation {
    pub rule: &'static str,
    pub group: String,
    pub subject: String,
    pub other: String,
    pub gap: f64,
    pub need: f64,
    pub at: crate::geom::P,
}

pub trait Rule: Sync {
    fn id(&self) -> &'static str;
    fn eval<C: Context>(&self, cx: &C, out: &mut Vec<Violation>);
}

pub fn check<C: Context>(cx: &C, out: &mut Vec<Violation>) {
    NetClearance.eval(cx, out);
    CopperToEdge.eval(cx, out);
    HoleToCopper(Which::Plated).eval(cx, out);
    HoleToCopper(Which::Inner).eval(cx, out);
    HoleToCopper(Which::Npth).eval(cx, out);
    HoleToHole.eval(cx, out);
}

pub fn legal<C: Context>(cx: &C) -> Result<(), Vec<Violation>> {
    let mut out = Vec::new();
    check(cx, &mut out);
    if out.is_empty() { Ok(()) } else { Err(out) }
}

pub trait Spacing: Send + Sync {
    fn rule(&self) -> &'static str;
    fn gap(&self, a: Option<usize>, b: Option<usize>, layer: Layer) -> f64;
    fn reach(&self, net: Option<usize>) -> f64;
}

#[derive(Clone, Default)]
pub struct Spacings {
    pub class: ClassClearance,
    pub isolation: Isolation,
    pub layers: usize,
}

impl Spacings {
    pub fn new(board: &Board, nets: &[LayoutNet], layers: usize) -> Spacings {
        Spacings {
            class: ClassClearance::new(board, nets),
            isolation: Isolation::new(board, nets, layers),
            layers,
        }
    }

    pub fn all(&self) -> [&dyn Spacing; 2] {
        [&self.class, &self.isolation]
    }

    pub fn layer(&self, index: usize) -> Layer {
        Layer::new(index, self.layers)
    }

    pub fn gap(&self, a: Option<usize>, b: Option<usize>, layer: usize) -> f64 {
        let l = self.layer(layer);
        self.all().iter().map(|r| r.gap(a, b, l)).fold(0.0, f64::max)
    }

    pub fn widest(&self, a: Option<usize>, b: Option<usize>) -> f64 {
        (0..self.layers).map(|l| self.gap(a, b, l)).fold(0.0, f64::max)
    }

    pub fn reach(&self, net: Option<usize>) -> f64 {
        self.all().iter().map(|r| r.reach(net)).fold(0.0, f64::max)
    }
}
