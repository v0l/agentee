pub use sims_field::fdtd::{
    Axis, C0, Debye, EPS0, ETA0, Edge, Element, Grid, Lumped, MU0, Materials, Media, PortDef,
    Resistive, SheetEdge, Sheets, Sim, axis, gaussian, idx, psi_len, pulse_shape, time_step, unidx,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debye_poles_hold_the_loss_tangent_across_the_band() {
        let d = Debye::band(1e8, 4e10);
        let (inf, dd) = d.fit(3.5, 0.004, 2e9);
        for f in [1e8, 1e9, 1e10, 4e10] {
            let (a, b) = d.response(2.0 * std::f64::consts::PI * f);
            let (re, im) = (inf + dd * a, dd * b);
            let (want_re, want_tan) = crate::loss::djordjevic_sarkar(3.5, 0.004, 2e9, f);
            assert!((im / re / want_tan - 1.0).abs() < 0.03, "{f}: tan {} vs {want_tan}", im / re);
            assert!((re - want_re).abs() < 0.01, "{f}: er {re} vs {want_re}");
        }
    }
}
