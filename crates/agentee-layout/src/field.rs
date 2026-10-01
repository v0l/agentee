use agentee_core::geom::P;
use agentee_core::graphic::Bounds;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct CostField {
    pub tile: f64,
    pub origin: P,
    pub nx: usize,
    pub ny: usize,
    pub layers: Vec<String>,
    pub capacity: Vec<f32>,
    pub demand: Vec<f32>,
    pub history: Vec<f32>,
}

impl CostField {
    pub fn new(bounds: Bounds, tile: f64, layers: Vec<String>) -> CostField {
        let nx = ((bounds.max[0] - bounds.min[0]) / tile).ceil().max(1.0) as usize;
        let ny = ((bounds.max[1] - bounds.min[1]) / tile).ceil().max(1.0) as usize;
        let n = nx * ny * layers.len();
        CostField {
            tile,
            origin: bounds.min,
            nx,
            ny,
            layers,
            capacity: vec![0.0; n],
            demand: vec![0.0; n],
            history: vec![0.0; n],
        }
    }

    pub fn idx(&self, layer: usize, x: usize, y: usize) -> usize {
        (layer * self.ny + y) * self.nx + x
    }

    pub fn tile_of(&self, p: P) -> Option<(usize, usize)> {
        let x = ((p[0] - self.origin[0]) / self.tile).floor();
        let y = ((p[1] - self.origin[1]) / self.tile).floor();
        if x < 0.0 || y < 0.0 || x as usize >= self.nx || y as usize >= self.ny {
            return None;
        }
        Some((x as usize, y as usize))
    }

    pub fn centre(&self, x: usize, y: usize) -> P {
        [
            self.origin[0] + (x as f64 + 0.5) * self.tile,
            self.origin[1] + (y as f64 + 0.5) * self.tile,
        ]
    }

    pub fn cost(&self, i: usize) -> f64 {
        let cap = self.capacity[i].max(1e-3) as f64;
        let use_ = self.demand[i] as f64;
        self.tile * (1.0 + use_ / cap) + self.history[i] as f64
    }

    pub fn overflow(&self) -> f64 {
        self.demand.iter().zip(&self.capacity).map(|(d, c)| (d - c.max(1.0)).max(0.0) as f64).sum()
    }

    pub fn add_history(&mut self, gain: f32) {
        for i in 0..self.demand.len() {
            let cap = self.capacity[i].max(1.0);
            if self.demand[i] > cap {
                self.history[i] += gain * (self.demand[i] - cap);
            }
        }
    }

    pub fn clear_demand(&mut self) {
        self.demand.iter_mut().for_each(|d| *d = 0.0);
    }

    pub fn fill_capacity(&mut self, layer: usize, tracks_per_tile: f32) {
        for y in 0..self.ny {
            for x in 0..self.nx {
                let i = self.idx(layer, x, y);
                self.capacity[i] = tracks_per_tile;
            }
        }
    }

    pub fn block(&mut self, layer: usize, poly: &[P]) {
        let mut b = Bounds::EMPTY;
        poly.iter().for_each(|p| b.add(*p));
        let Some((x0, y0)) = self.tile_of([b.min[0], b.min[1]]) else { return };
        let (x1, y1) = self.tile_of([b.max[0], b.max[1]]).unwrap_or((self.nx - 1, self.ny - 1));
        for y in y0..=y1 {
            for x in x0..=x1 {
                if agentee_core::geom::point_in_polygon(self.centre(x, y), poly) {
                    let i = self.idx(layer, x, y);
                    self.capacity[i] = 0.0;
                }
            }
        }
    }

    pub fn rudy(&mut self, layer: usize, a: P, b: P, width: f64) {
        let (Some((ax, ay)), Some((bx, by))) = (self.tile_of(a), self.tile_of(b)) else { return };
        let (x0, x1) = (ax.min(bx), ax.max(bx));
        let (y0, y1) = (ay.min(by), ay.max(by));
        let w = (x1 - x0 + 1) as f64;
        let h = (y1 - y0 + 1) as f64;
        let per_tile = (width / self.tile) * (w + h) / (w * h);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let i = self.idx(layer, x, y);
                self.demand[i] += per_tile as f32;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rudy_spreads_a_net_over_its_box_and_cost_rises_with_demand() {
        let b = Bounds { min: [0.0, 0.0], max: [10.0, 10.0] };
        let mut f = CostField::new(b, 1.0, vec!["F.Cu".into()]);
        f.fill_capacity(0, 4.0);
        let i = f.idx(0, 5, 5);
        let c0 = f.cost(i);
        f.rudy(0, [1.0, 1.0], [9.0, 9.0], 1.0);
        assert!(f.demand[i] > 0.0);
        assert!(f.cost(i) > c0);
        assert_eq!(f.overflow(), 0.0);
        for _ in 0..40 {
            f.rudy(0, [5.0, 5.0], [5.5, 5.5], 1.0);
        }
        assert!(f.overflow() > 0.0);
        f.add_history(1.0);
        assert!(f.history[i] > 0.0);
    }
}
