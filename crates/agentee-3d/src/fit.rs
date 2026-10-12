use agentee_core::footprint::Footprint;
use agentee_core::geom::{self, P};

#[derive(Clone, Debug, PartialEq)]
pub struct Fit {
    pub turn: f64,
    pub shift: P,
    pub rms: f64,
    pub pads: usize,
    pub model_offset: [f64; 3],
    pub model_rotate: [f64; 3],
    pub model_scale: [f64; 3],
}

fn centres(fp: &Footprint) -> Vec<(String, P)> {
    let mut out: Vec<(String, Vec<P>)> = Vec::new();
    for p in fp.pads.iter().filter(|p| !p.number.is_empty()) {
        match out.iter_mut().find(|(n, _)| *n == p.number) {
            Some((_, v)) => v.push(p.at.to_mm()),
            None => out.push((p.number.clone(), vec![p.at.to_mm()])),
        }
    }
    out.into_iter()
        .map(|(n, v)| {
            let k = v.len() as f64;
            (n, [v.iter().map(|q| q[0]).sum::<f64>() / k, v.iter().map(|q| q[1]).sum::<f64>() / k])
        })
        .collect()
}

pub fn fit(vendor: &Footprint, ours: &Footprint) -> Result<Fit, String> {
    let (v, u) = (centres(vendor), centres(ours));
    let pairs: Vec<(P, P)> = v
        .iter()
        .filter_map(|(n, a)| u.iter().find(|(m, _)| m == n).map(|(_, b)| (*a, *b)))
        .collect();
    if pairs.len() < 2 || pairs.len() * 2 < u.len() {
        return Err(format!(
            "the vendor footprint shares {} pad numbers with ours ({} pads), too few to place its model",
            pairs.len(),
            u.len()
        ));
    }
    let n = pairs.len() as f64;
    let mean = |f: fn(&(P, P)) -> P| {
        let s = pairs.iter().map(f).fold([0.0, 0.0], |a, q| [a[0] + q[0], a[1] + q[1]]);
        [s[0] / n, s[1] / n]
    };
    let (cv, cu) = (mean(|p| p.0), mean(|p| p.1));
    let best = [0.0, 90.0, 180.0, 270.0]
        .into_iter()
        .map(|turn| {
            let g = geom::rotate(cv, turn);
            let shift = [cu[0] - g[0], cu[1] - g[1]];
            let err: f64 = pairs
                .iter()
                .map(|(a, b)| {
                    let q = geom::rotate(*a, turn);
                    (q[0] + shift[0] - b[0]).powi(2) + (q[1] + shift[1] - b[1]).powi(2)
                })
                .sum();
            (turn, shift, (err / n).sqrt())
        })
        .min_by(|a, b| a.2.total_cmp(&b.2))
        .unwrap_or((0.0, [0.0, 0.0], f64::MAX));
    let (turn, shift, rms) = best;
    if rms > 0.1 {
        return Err(format!(
            "the vendor footprint's pads sit {rms:.2} mm from ours at best, so its model would not line up"
        ));
    }
    let ex = geom::rotate([1.0, 0.0], turn);
    let ey = geom::rotate([0.0, 1.0], turn);
    let flipped = [[ex[0], -ey[0], 0.0], [-ex[1], ey[1], 0.0], [0.0, 0.0, 1.0]];
    let alpha = flipped[1][0].atan2(flipped[0][0]).to_degrees();
    let o = vendor.model_offset;
    let moved = [0, 1, 2].map(|i| (0..3).map(|k| flipped[i][k] * o[k]).sum::<f64>());
    let r = vendor.model_rotate;
    let z = r[2] - alpha;
    let z = (z + 180.0).rem_euclid(360.0) - 180.0;
    let round = |v: f64| (v * 1e4).round() / 1e4;
    Ok(Fit {
        turn,
        shift,
        rms,
        pads: pairs.len(),
        model_offset: [round(moved[0] + shift[0]), round(moved[1] - shift[1]), round(moved[2])],
        model_rotate: [r[0], r[1], round(z)],
        model_scale: vendor.model_scale,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fp(pads: &[(&str, [f64; 2])], model: &str) -> Footprint {
        let mut text = format!("name = \"X\"\nmodel = \"x.step\"\n{model}");
        for (n, at) in pads {
            text += &format!(
                "\n[[pads]]\nnumber = \"{n}\"\nkind = \"smd\"\nshape = \"rect\"\nat = [{}, {}]\nsize = [0.5, 0.5]\n",
                at[0], at[1]
            );
        }
        let f: agentee_core::footprint::FootprintFile = toml::from_str(&text).unwrap();
        f.resolve(&mut agentee_core::diag::Diags::new("X"))
    }

    fn placed(fp: &Footprint, m: [f64; 3]) -> P {
        let r = crate::export::rotation(fp.model_rotate);
        let q = [0, 1, 2].map(|i| (0..3).map(|k| r[i][k] * m[k]).sum::<f64>() + fp.model_offset[i]);
        [q[0], -q[1]]
    }

    #[test]
    fn a_vendor_model_lands_where_its_pads_say() {
        let vendor = fp(
            &[("1", [-1.0, 0.5]), ("2", [1.0, 0.5]), ("3", [0.0, -2.0])],
            "model_offset = [\"0.2mm\", \"-0.3mm\", \"0mm\"]\nmodel_rotate = [0.0, 0.0, 30.0]\n",
        );
        for turn in [0.0, 90.0, 180.0, 270.0] {
            let at = |p: P| {
                let q = geom::rotate(p, turn);
                [q[0] + 3.0, q[1] - 1.5]
            };
            let ours =
                fp(&[("1", at([-1.0, 0.5])), ("2", at([1.0, 0.5])), ("3", at([0.0, -2.0]))], "");
            let f = fit(&vendor, &ours).unwrap();
            assert!(f.rms < 1e-9 && f.turn == turn, "{f:?}");
            let mut mine = ours.clone();
            mine.model_offset = f.model_offset;
            mine.model_rotate = f.model_rotate;
            for m in [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 2.0, 1.0]] {
                let want = at(placed(&vendor, m));
                let got = placed(&mine, m);
                assert!(geom::dist(want, got) < 1e-3, "turn {turn}: {want:?} {got:?}");
            }
        }
    }

    #[test]
    fn mismatched_pads_are_refused() {
        let vendor = fp(&[("1", [-1.0, 0.0]), ("2", [1.0, 0.0])], "");
        let ours = fp(&[("1", [-1.5, 0.0]), ("2", [1.5, 0.0])], "");
        assert!(fit(&vendor, &ours).unwrap_err().contains("0.50 mm"));
        let other = fp(&[("A", [-1.0, 0.0]), ("B", [1.0, 0.0])], "");
        assert!(fit(&vendor, &other).is_err());
    }
}
