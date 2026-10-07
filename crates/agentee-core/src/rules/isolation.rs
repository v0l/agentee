use super::{Layer, Spacing};
use crate::board::{Barrier, Board};
use crate::layout::LayoutNet;

#[derive(Clone, Default)]
pub struct Isolation {
    pub domain: Vec<Option<usize>>,
    pub gap: Vec<Vec<[f64; 2]>>,
    barrier: Vec<Vec<Option<usize>>>,
    pub layers: usize,
}

impl Isolation {
    pub fn new(board: &Board, nets: &[LayoutNet], layers: usize) -> Isolation {
        let n = board.domains.len();
        let mut gap = vec![vec![[0.0; 2]; n]; n];
        let mut barrier = vec![vec![None; n]; n];
        for (k, b) in board.barriers.iter().enumerate() {
            let inner = b.clearance.map_or(0.0, |c| c.to_mm());
            let outer = inner.max(b.creepage.map_or(0.0, |c| c.to_mm()));
            let [x, y] = b.between;
            gap[x][y] = [inner, outer];
            gap[y][x] = [inner, outer];
            barrier[x][y] = Some(k);
            barrier[y][x] = Some(k);
        }
        let barred: Vec<bool> = (0..n).map(|d| gap[d].iter().any(|g| g[1] > 0.0)).collect();
        let domain = nets
            .iter()
            .map(|net| {
                board.domain_of(&net.name, &net.class).first().copied().filter(|&d| barred[d])
            })
            .collect();
        Isolation { domain, gap, barrier, layers }
    }

    pub fn active(&self) -> bool {
        self.domain.iter().any(Option::is_some)
    }

    pub fn of(&self, net: usize) -> Option<usize> {
        self.domain.get(net).copied().flatten()
    }

    pub fn apart(&self, a: Option<usize>, b: Option<usize>, l: usize) -> f64 {
        match (a, b) {
            (Some(a), Some(b)) if a != b => {
                let outer = Layer::new(l, self.layers).outer;
                self.gap[a][b][usize::from(outer)]
            }
            _ => 0.0,
        }
    }

    pub fn widest(&self, a: Option<usize>) -> f64 {
        a.map_or(0.0, |a| self.gap[a].iter().map(|g| g[0].max(g[1])).fold(0.0, f64::max))
    }

    pub fn barrier<'b>(&self, board: &'b Board, a: usize, b: usize) -> Option<&'b Barrier> {
        let (x, y) = (self.of(a)?, self.of(b)?);
        self.barrier[x][y].map(|k| &board.barriers[k])
    }
}

impl Spacing for Isolation {
    fn rule(&self) -> &'static str {
        "isolation-clearance"
    }

    fn gap(&self, a: Option<usize>, b: Option<usize>, layer: Layer) -> f64 {
        let domain = |n: Option<usize>| n.and_then(|n| self.of(n));
        self.apart(domain(a), domain(b), layer.index)
    }

    fn reach(&self, net: Option<usize>) -> f64 {
        self.widest(net.and_then(|n| self.of(n)))
    }
}
