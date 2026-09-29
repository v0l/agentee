fn main() {
    for f in std::env::args().skip(1) {
        let t = std::time::Instant::now();
        match agentee_3d::load(std::path::Path::new(&f)) {
            Ok(m) => {
                let (lo, hi) = m.bounds();
                println!(
                    "{f}: {} tris {:.3}s lo {lo:?} hi {hi:?}",
                    m.triangles(),
                    t.elapsed().as_secs_f64()
                );
                for p in &m.parts {
                    println!("   {:?} {}", p.colour, p.positions.len() / 3);
                }
            }
            Err(e) => println!("{f}: {e}"),
        }
    }
}
