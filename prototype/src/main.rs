use std::time::Instant;

fn main() {
    let data = generate_data();

    let start = Instant::now();
    let result: i64 = data.iter().sum();
    let duration = start.elapsed();

    println!("Result: {}", result);
    println!("Time: {:?}", duration);
}

const SIZE: usize = 100_000_001;
fn generate_data() -> Vec<i64> {
    (0..SIZE as i64).collect()
}

pub enum Column {
    Int64(Vec<i64>),
    Float64(Vec<f64>),
    String(Vec<String>),
}
