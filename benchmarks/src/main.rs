#![warn(clippy::all)]

use crate::data_generator::generate;
use crate::rowstore::{RowTable, count_where_ts_gt, sum_dur_where_ts_gt};
use minihouse::aggregate::AggKind;
use minihouse::query::{execute, CmpOp, SimpleQuery};
use minihouse::table::Table;
use minihouse::value::Value;
use std::time::{Duration, Instant};

mod data_generator;
mod rowstore;

fn main() {
    let (col_table, row_table) = generate(10_000_000, 8192);

    sum_bench(&col_table, &row_table);
    without_filter_bench(&col_table, &row_table);
    count_bench(&col_table, &row_table);
}

fn count_bench(col_table: &Table, row_table: &RowTable) {
    for threshold in [990_000, 500_000, 10_000] {
        let colv = execute(&col_table, &count_query(threshold));
        let rowv = count_where_ts_gt(&row_table, threshold);
        assert_eq!(
            colv,
            Some(Value::Int64(rowv)),
            "engines disagree at {threshold}"
        );
        println!("cross-check passed for thresholds {threshold}")
    }
    println!("cross-check passed");

    for (label, threshold) in [("~1%", 990_000), ("~50%", 500_000), ("~99%", 10_000)] {
        let c = bench("columnar", 7, || {
            execute(&col_table, &count_query(threshold))
        });
        let r = bench("row", 7, || count_where_ts_gt(&row_table, threshold));
        let ratio = r.as_secs_f64() / c.as_secs_f64();
        println!("{label}: columnar {c:?}, row {r:?}, ratio {ratio:.1}x");
    }
}

fn without_filter_bench(col_table: &Table, row_table: &RowTable) {
    println!("without filter bench");

    let colv = execute(
        &col_table,
        &SimpleQuery {
            filter: None,
            aggregate: ("dur", AggKind::Sum),
        },
    );
    let rowv = row_table.rows.iter().map(|r| r.dur).sum::<i64>();
    assert_eq!(colv, Some(Value::Int64(rowv)), "engines disagree");
    println!("cross-check passed");

    let c = bench("columnar no-filter", 7, || {
        execute(
            &col_table,
            &SimpleQuery {
                filter: None,
                aggregate: ("dur", AggKind::Sum),
            },
        )
    });
    let r = bench("row no-filter", 7, || {
        row_table.rows.iter().map(|r| r.dur).sum::<i64>()
    });
    let ratio = r.as_secs_f64() / c.as_secs_f64();
    println!("without filter: columnar {c:?}, row {r:?}, ratio {ratio:.1}x");
}

fn sum_bench(col_table: &Table, row_table: &RowTable) {
    println!("sum bench");

    for threshold in [990_000, 500_000, 10_000] {
        let colv = execute(col_table, &sum_dur_query(threshold));
        let rowv = sum_dur_where_ts_gt(row_table, threshold);
        assert_eq!(
            colv,
            Some(Value::Int64(rowv)),
            "engines disagree at {threshold}"
        );
        println!("cross-check passed for thresholds {threshold}")
    }
    println!("cross-check passed");

    for (label, threshold) in [("~1%", 990_000), ("~50%", 500_000), ("~99%", 10_000)] {
        let c = bench("columnar", 7, || {
            execute(col_table, &sum_dur_query(threshold))
        });
        let r = bench("row", 7, || sum_dur_where_ts_gt(row_table, threshold));
        let ratio = r.as_secs_f64() / c.as_secs_f64();
        println!("{label}: columnar {c:?}, row {r:?}, ratio {ratio:.1}x");
    }
}

fn sum_dur_query(x: i64) -> SimpleQuery<'static> {
    SimpleQuery {
        filter: Some(("ts", CmpOp::Gt, Value::Int64(x))),
        aggregate: ("dur", AggKind::Sum),
    }
}

fn count_query(x: i64) -> SimpleQuery<'static> {
    SimpleQuery {
        filter: Some(("ts", CmpOp::Gt, Value::Int64(x))),
        aggregate: ("id", AggKind::Count),
    }
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
