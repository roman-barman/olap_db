use std::hint::black_box;
use criterion::{criterion_group, criterion_main, Criterion};

const SIZE: usize = 100_000_000;

fn generate_data() -> Vec<i64> {
    (0..SIZE as i64).collect()
}

fn sum_vec(input: &[i64]) -> i64 {
    input.iter().sum()
}

fn bench_sum(c: &mut Criterion) {
    let data = generate_data();

    c.bench_function(
        "sum_vec",
        |b| b.iter(|| sum_vec(black_box(&data))));
}

criterion_group!(benches, bench_sum);
criterion_main!(benches);
