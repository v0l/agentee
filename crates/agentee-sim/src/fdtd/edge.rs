use std::collections::HashMap;
use std::f64::consts::PI;
use std::sync::{Mutex, OnceLock};

pub const BANDS: usize = 4;

#[derive(Clone, Debug)]
pub struct Weights {
    pub bands: [f64; BANDS],
    pub shift: f64,
}

fn axis(neg: f64, pos: f64, uniform: usize) -> Vec<f64> {
    let reach = 1e3 * neg.max(pos);
    let grow = |d: f64| {
        let mut out = vec![0.0];
        let mut step = d;
        while *out.last().unwrap() < reach {
            out.push(out.last().unwrap() + step);
            if out.len() > uniform {
                step *= 1.25;
            }
        }
        out
    };
    let mut line: Vec<f64> = grow(neg).into_iter().skip(1).map(|v| -v).rev().collect();
    line.extend(grow(pos));
    line
}

pub fn stopping_distance(t: f64) -> f64 {
    t * (-PI).exp() / (4.0 * PI)
}

pub fn thin_edge_charges(d: f64, dz_above: f64, dz_below: f64, count: usize) -> Vec<f64> {
    let uniform = (3 * count).max(16) + (3.0 * dz_above.max(dz_below) / d).ceil() as usize;
    let xs = axis(d, d, uniform);
    let zs = axis(dz_below, dz_above, uniform);
    let (nx, nz) = (xs.len(), zs.len());
    let i0 = xs.iter().position(|v| *v == 0.0).unwrap();
    let j0 = zs.iter().position(|v| *v == 0.0).unwrap();
    let at = |i: usize, j: usize| i * nz + j;
    let exact = |i: usize, j: usize| {
        let (x, z) = (xs[i], zs[j]);
        let r = (x * x + z * z).sqrt();
        ((r + x) / 2.0).max(0.0).sqrt()
    };
    let fixed =
        |i: usize, j: usize| i == 0 || j == 0 || i + 1 == nx || j + 1 == nz || (j == j0 && i <= i0);
    let value = |i: usize, j: usize| if j == j0 && i <= i0 { 0.0 } else { exact(i, j) };
    let mut phi = vec![0.0f64; nx * nz];
    for i in 0..nx {
        for j in 0..nz {
            phi[at(i, j)] = if fixed(i, j) { value(i, j) } else { 0.0 };
        }
    }
    let weights = |i: usize, j: usize| {
        let dual_x = 0.5 * (xs[i + 1] - xs[i - 1]);
        let dual_z = 0.5 * (zs[j + 1] - zs[j - 1]);
        [
            dual_z / (xs[i + 1] - xs[i]),
            dual_z / (xs[i] - xs[i - 1]),
            dual_x / (zs[j + 1] - zs[j]),
            dual_x / (zs[j] - zs[j - 1]),
        ]
    };
    let nbrs = |i: usize, j: usize| [(i + 1, j), (i - 1, j), (i, j + 1), (i, j - 1)];
    let mut unknown = vec![usize::MAX; nx * nz];
    let mut order = Vec::new();
    for i in 1..nx - 1 {
        for j in 1..nz - 1 {
            if !fixed(i, j) {
                unknown[at(i, j)] = order.len();
                order.push((i, j));
            }
        }
    }
    let mut triplets = Vec::with_capacity(3 * order.len());
    let mut rhs = vec![0.0f64; order.len()];
    for (row, (i, j)) in order.iter().copied().enumerate() {
        let w = weights(i, j);
        triplets.push(faer::sparse::Triplet::new(row, row, w.iter().sum::<f64>()));
        for (k, (a, b)) in nbrs(i, j).into_iter().enumerate() {
            if fixed(a, b) {
                rhs[row] += w[k] * value(a, b);
            } else if unknown[at(a, b)] < row {
                triplets.push(faer::sparse::Triplet::new(row, unknown[at(a, b)], -w[k]));
            }
        }
    }
    faer::set_global_parallelism(faer::Par::Seq);
    let solved = {
        use faer::linalg::solvers::Solve;
        let m = order.len();
        let a = faer::sparse::SparseColMat::<usize, f64>::try_new_from_triplets(m, m, &triplets)
            .expect("slit matrix");
        let llt = a.sp_cholesky(faer::Side::Lower).expect("slit matrix is positive definite");
        llt.solve(faer::Mat::<f64>::from_fn(m, 1, |r, _| rhs[r]))
    };
    let mut x = vec![0.0f64; nx * nz];
    for (row, (i, j)) in order.iter().enumerate() {
        x[at(*i, *j)] = solved[(row, 0)];
    }
    for i in 0..nx {
        for j in 0..nz {
            phi[at(i, j)] = if fixed(i, j) { value(i, j) } else { x[at(i, j)] };
        }
    }
    (0..=count)
        .map(|k| {
            let i = i0 - k;
            let w = [
                0.5 * (zs[j0 + 1] - zs[j0 - 1]) / (xs[i + 1] - xs[i]),
                0.5 * (zs[j0 + 1] - zs[j0 - 1]) / (xs[i] - xs[i - 1]),
                d / (zs[j0 + 1] - zs[j0]),
                d / (zs[j0] - zs[j0 - 1]),
            ];
            let side = |a: usize| if a > i0 { phi[at(a, j0)] } else { 0.0 };
            w[0] * side(i + 1)
                + w[1] * side(i - 1)
                + w[2] * phi[at(i, j0 + 1)]
                + w[3] * phi[at(i, j0 - 1)]
        })
        .collect()
}

struct Canonical {
    charges: Vec<f64>,
    shift: f64,
}

fn canonical(above: f64, below: f64) -> Canonical {
    let count = ((8.0 * above.max(below)).ceil() as usize).max(8);
    let charges = thin_edge_charges(1.0, above, below, count);
    let total: f64 = charges.iter().sum();
    let shift = (total / 2.0).powi(2) - (count as f64 + 0.5);
    Canonical { charges, shift }
}

const STEP: f64 = 0.2;

fn corner(key: [i64; 2]) -> std::sync::Arc<Canonical> {
    static CACHE: OnceLock<Mutex<HashMap<[i64; 2], std::sync::Arc<Canonical>>>> = OnceLock::new();
    let key = [key[0].min(key[1]), key[0].max(key[1])];
    if let Some(c) = CACHE.get_or_init(Default::default).lock().unwrap().get(&key) {
        return c.clone();
    }
    let ratio = |k: i64| (k as f64 * STEP).exp();
    let c = std::sync::Arc::new(canonical(ratio(key[0]), ratio(key[1])));
    CACHE.get_or_init(Default::default).lock().unwrap().insert(key, c.clone());
    c
}

fn interpolated(above: f64, below: f64) -> Canonical {
    let pos = |r: f64| r.clamp(1.0 / 64.0, 64.0).ln() / STEP;
    let (a, b) = (pos(above), pos(below));
    let (a0, b0) = (a.floor() as i64, b.floor() as i64);
    let (fa, fb) = (a - a0 as f64, b - b0 as f64);
    let mut charges = vec![0.0; BANDS];
    let mut shift = 0.0;
    for (da, wa) in [(0, 1.0 - fa), (1, fa)] {
        for (db, wb) in [(0, 1.0 - fb), (1, fb)] {
            let w = wa * wb;
            if w == 0.0 {
                continue;
            }
            let c = corner([a0 + da, b0 + db]);
            shift += w * c.shift;
            for (k, q) in charges.iter_mut().enumerate() {
                *q += w * c.charges[k];
            }
        }
    }
    Canonical { charges, shift }
}

pub fn shift(d: f64, dz_above: f64, dz_below: f64) -> f64 {
    interpolated(dz_above / d, dz_below / d).shift * d
}

pub fn weights(d: f64, dz_above: f64, dz_below: f64, t: f64) -> Weights {
    let c = interpolated(dz_above / d, dz_below / d);
    let shift = c.shift * d;
    let mut bands = [1.0; BANDS];
    for (k, band) in bands.iter_mut().enumerate() {
        let q = c.charges[k] * d.sqrt();
        let (lo, hi) = if k == 0 {
            (stopping_distance(t), shift + 0.5 * d)
        } else {
            (shift + (k as f64 - 0.5) * d, shift + (k as f64 + 0.5) * d)
        };
        let truth = 0.5 * (hi / lo).ln();
        *band = if k == 0 { truth * (0.5 * d) / (q * q) } else { truth * d / (0.5 * q * q) };
    }
    Weights { bands, shift }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_square_mesh_moves_a_slit_edge_three_eighths_of_a_cell() {
        let q = thin_edge_charges(1.0, 1.0, 1.0, 8);
        let total: f64 = q.iter().sum();
        let shift = (total / 2.0).powi(2) - 8.5;
        assert!((shift - 0.37).abs() < 0.02, "{shift}");
        let lopsided = interpolated(0.55, 2.3).shift;
        let direct = canonical(0.55, 2.3).shift;
        assert!((lopsided - direct).abs() < 0.01, "{lopsided} {direct}");
        let w = weights(0.05, 0.05, 0.05, 0.035);
        assert!(w.bands.iter().all(|b| *b > 0.3 && *b < 3.0), "{w:?}");
    }
}
