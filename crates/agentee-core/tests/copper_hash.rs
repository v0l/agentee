use agentee_core::Project;
use agentee_core::sim::copper_hash;
use std::path::Path;

#[test]
fn copper_hash_ignores_sub_micron_fill_jitter_but_sees_track_moves() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/lna");
    let p = Project::load(&root).unwrap();
    let board = &p.boards[0].item;
    let mut layout = p.layouts[0].item.clone();
    assert!(!layout.zones.is_empty() && !layout.tracks.is_empty());
    for z in &mut layout.zones {
        for r in &mut z.rings {
            for c in r.iter_mut() {
                *c = [(c[0] * 1e3).round() / 1e3, (c[1] * 1e3).round() / 1e3];
            }
        }
    }
    let base = copper_hash(&layout, board, None);
    let mut jitter = layout.clone();
    let mut sign = 1.0;
    for z in &mut jitter.zones {
        for r in &mut z.rings {
            for c in r.iter_mut() {
                *c = [c[0] + sign * 1e-4, c[1] - sign * 1e-4];
                sign = -sign;
            }
        }
    }
    assert_eq!(copper_hash(&jitter, board, None), base);
    let mut moved = layout.clone();
    moved.tracks[0].points.iter_mut().for_each(|q| q[0] += 5e-3);
    assert_ne!(copper_hash(&moved, board, None), base);
    let mut ring = layout.clone();
    let z = ring.zones.iter_mut().find(|z| !z.rings.is_empty()).unwrap();
    z.rings[0][0][0] += 5e-3;
    assert_ne!(copper_hash(&ring, board, None), base);
}
