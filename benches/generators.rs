use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use bulletproofs::{BulletproofGens, PedersenGens};

fn pc_gens(c: &mut Criterion) {
    c.bench_function("PedersenGens::new", |b| b.iter(|| PedersenGens::default()));
}

fn bp_gens(c: &mut Criterion) {
    let mut group = c.benchmark_group("BulletproofGens::new");

    for size in (0..10).map(|i| 2 << i) {
        group.bench_with_input(
            BenchmarkId::from_parameter(size),
            &size,
            |b, &s| {
                b.iter(|| BulletproofGens::new(s, 1))
            },
        );
    }

    group.finish();
}

criterion_group! {
    bp,
    bp_gens,
    pc_gens,
}

criterion_main!(bp);
