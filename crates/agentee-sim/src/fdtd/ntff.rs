pub use sims_field::fdtd::{C, Far, Patches, patches};

pub fn fcc_class_b_dbuv(f: f64) -> f64 {
    match f {
        f if f < 88e6 => 40.0,
        f if f < 216e6 => 43.5,
        f if f < 960e6 => 46.0,
        _ => 54.0,
    }
}
