use std::time::{Duration, Instant};

mod column_vs_row;

fn main() {
    column_vs_row::execute();
}

fn bench<F: Fn() -> R, R>(name: &str, runs: usize, f: F) -> Duration {
    let mut times: Vec<Duration> = (0..runs + 1)
        .map(|_| {
            let t = Instant::now();
            std::hint::black_box(f());
            t.elapsed()
        })
        .skip(1)
        .collect();
    times.sort();
    let median = times[times.len() / 2];
    println!("{name}: median {median:?} over {runs} runs");
    median
}
