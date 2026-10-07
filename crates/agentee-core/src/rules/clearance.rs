use super::{Layer, Spacing};
use crate::board::Board;
use crate::layout::LayoutNet;

#[derive(Clone, Default)]
pub struct ClassClearance {
    pub of: Vec<f64>,
    pub unconnected: f64,
    widest: f64,
}

impl ClassClearance {
    pub fn new(board: &Board, nets: &[LayoutNet]) -> ClassClearance {
        let of: Vec<f64> = nets.iter().map(|n| n.clearance).collect();
        let unconnected = board
            .netclass("Default")
            .map(|c| c.clearance.to_mm())
            .unwrap_or(board.rules.min_clearance.to_mm());
        let widest = of.iter().copied().fold(unconnected, f64::max);
        ClassClearance { of, unconnected, widest }
    }

    pub fn of(&self, net: Option<usize>) -> f64 {
        match net {
            Some(n) => self.of.get(n).copied().unwrap_or(self.unconnected),
            None => self.unconnected,
        }
    }
}

impl Spacing for ClassClearance {
    fn rule(&self) -> &'static str {
        "clearance"
    }

    fn gap(&self, a: Option<usize>, b: Option<usize>, _: Layer) -> f64 {
        if a.is_some() && a == b {
            return 0.0;
        }
        self.of(a).max(self.of(b))
    }

    fn reach(&self, net: Option<usize>) -> f64 {
        self.of(net).max(self.widest)
    }
}
