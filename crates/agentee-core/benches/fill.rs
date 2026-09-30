use agentee_core::Project;
use agentee_core::layout::{
    FillCase, ZonesCase, capture_fill_cases, take_fill_cases, take_zones_cases,
};
use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};
use std::cell::OnceCell;
use std::hint::black_box;
use std::path::PathBuf;
use std::time::Duration;

const EXAMPLES: &[&str] = &["lna", "hackrf-pro"];

fn example(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples").join(name)
}

fn cases(name: &str) -> (Vec<FillCase>, Vec<ZonesCase>) {
    capture_fill_cases();
    Project::load(&example(name)).expect("example loads");
    (take_fill_cases(), take_zones_cases())
}

fn load(c: &mut Criterion) {
    let mut g = c.benchmark_group("load");
    g.sample_size(10).measurement_time(Duration::from_secs(5));
    for name in EXAMPLES {
        let dir = example(name);
        g.bench_function(*name, |b| b.iter(|| Project::load(black_box(&dir)).unwrap()));
    }
    g.finish();
}

type Shapes = Vec<Vec<Vec<[f64; 2]>>>;

fn fill(c: &mut Criterion) {
    for name in EXAMPLES {
        let (cases, zones) = cases(name);
        let mut g = c.benchmark_group(format!("fill/{name}"));
        g.sample_size(10).measurement_time(Duration::from_secs(5));
        for z in &zones {
            g.bench_function("check_zones", |b| b.iter(|| z.check()));
        }
        for case in &cases {
            let filled = OnceCell::new();
            let raster = || filled.get_or_init(|| case.fill());
            let clipped = OnceCell::new();
            let clip = || clipped.get_or_init(|| case.clip());
            let cut: OnceCell<Shapes> = OnceCell::new();
            let overlaid = || cut.get_or_init(|| case.overlay(&clip().0, &clip().1));
            let label = case.name.replace('/', ".");
            let id = |f: &str| BenchmarkId::new(f, &label);
            g.bench_function(id("fill_zone"), |b| b.iter(|| case.fill()));
            g.bench_function(id("vector_fill"), |b| b.iter(|| case.vector(black_box(raster()))));
            g.bench_function(id("clip"), |b| b.iter(|| case.clip()));
            g.bench_function(id("overlay"), |b| {
                b.iter(|| case.overlay(black_box(&clip().0), black_box(&clip().1)))
            });
            g.bench_function(id("min_width"), |b| {
                b.iter_batched(|| overlaid().clone(), |s| case.open(s), BatchSize::LargeInput)
            });
            g.bench_function(id("probe"), |b| {
                b.iter_batched(
                    || overlaid().clone(),
                    |s| case.probe(s, raster()),
                    BatchSize::LargeInput,
                )
            });
            g.bench_function(id("keep_connected"), |b| {
                b.iter(|| case.keep_connected(black_box(&raster().rings)))
            });
            g.bench_function(id("rasterize"), |b| {
                b.iter_batched_ref(
                    || raster().clone(),
                    |f| case.rasterize(f),
                    BatchSize::LargeInput,
                )
            });
        }
        g.finish();
    }
}

criterion_group!(benches, load, fill);
criterion_main!(benches);
