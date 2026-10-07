mod clearance;
mod isolation;

pub use clearance::ClassClearance;
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
