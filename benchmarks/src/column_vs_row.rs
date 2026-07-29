use crate::bench;
use crate::column_vs_row::generator::generate;
use crate::column_vs_row::row_table::{
    RowTable, count_where_ts_gt, count_where_url_eq, sum_dur, sum_dur_where_ts_gt,
};
use minihouse::aggregate::AggKind;
use minihouse::query::{CmpOp, SimpleQuery};
use minihouse::{Table, Value};

mod generator;
mod row_table;

pub(super) fn execute() {
    let (col_table, row_table) = generate(10_000_000, 8192);
    sum_bench(&col_table, &row_table);
    without_filter_bench(&col_table, &row_table);
    count_bench(&col_table, &row_table);
    count_string_bench(&col_table, &row_table)
}

fn sum_bench(col_table: &Table, row_table: &RowTable) {
    println!("sum bench");
    println!();

    for threshold in [990_000, 500_000, 10_000] {
        println!("run cross-check threshold: {}", threshold);
        crosscheck(
            col_table,
            row_table,
            &sum_dur_where_ts_gt_query(threshold),
            |table| sum_dur_where_ts_gt(table, threshold),
        );
        println!();
    }

    println!("run benchmark");
    println!();

    for (label, threshold) in [("~1%", 990_000), ("~50%", 500_000), ("~99%", 10_000)] {
        bench_pair(
            label,
            col_table,
            row_table,
            &sum_dur_where_ts_gt_query(threshold),
            |table| sum_dur_where_ts_gt(table, threshold),
        );
        println!();
    }

    println!("--------------------");
    println!();
}

fn without_filter_bench(col_table: &Table, row_table: &RowTable) {
    println!("without filter bench");
    println!();

    println!("run cross-check");
    crosscheck(col_table, row_table, &sum_dur_query(), |table| {
        sum_dur(table)
    });
    println!();

    bench_pair(
        "without filter",
        col_table,
        row_table,
        &sum_dur_query(),
        |table| sum_dur(table),
    );
    println!();
    println!("--------------------");
    println!();
}

fn count_bench(col_table: &Table, row_table: &RowTable) {
    println!("count bench");
    println!();

    for threshold in [990_000, 500_000, 10_000] {
        println!("run cross-check threshold: {}", threshold);
        crosscheck(
            col_table,
            row_table,
            &count_id_where_ts_gt_query(threshold),
            |table| count_where_ts_gt(table, threshold),
        );
        println!();
    }

    println!("run benchmark");
    println!();

    for (label, threshold) in [("~1%", 990_000), ("~50%", 500_000), ("~99%", 10_000)] {
        bench_pair(
            label,
            col_table,
            row_table,
            &count_id_where_ts_gt_query(threshold),
            |table| count_where_ts_gt(table, threshold),
        );
        println!();
    }

    println!("--------------------");
    println!();
}

fn count_string_bench(col_table: &Table, row_table: &RowTable) {
    println!("count string bench");
    println!();

    let value = "/page/42";

    println!("run cross-check");
    crosscheck(
        col_table,
        row_table,
        &count_where_url_eq_query(value),
        |table| count_where_url_eq(table, value),
    );
    println!();

    println!("run benchmark");
    println!();

    bench_pair(
        "count string",
        col_table,
        row_table,
        &count_where_url_eq_query(value),
        |table| count_where_url_eq(table, value),
    );
    println!();

    println!("--------------------");
    println!();
}

fn bench_pair<F>(
    label: &str,
    col_table: &Table,
    row_table: &RowTable,
    col_query: &SimpleQuery,
    row_query: F,
) where
    F: Fn(&RowTable) -> i64,
{
    let column_result = bench("columnar", 7, || {
        minihouse::query::execute(col_table, col_query)
    });
    let row_result = bench("row", 7, || row_query(row_table));
    let ratio = row_result.as_secs_f64() / column_result.as_secs_f64();
    println!("{label}: columnar {column_result:?}, row {row_result:?}, ratio {ratio:.1}x");
}

fn sum_dur_where_ts_gt_query(x: i64) -> SimpleQuery<'static> {
    SimpleQuery {
        filter: Some(("ts", CmpOp::Gt, Value::Int64(x))),
        aggregate: ("dur", AggKind::Sum),
    }
}

fn count_id_where_ts_gt_query(x: i64) -> SimpleQuery<'static> {
    SimpleQuery {
        filter: Some(("ts", CmpOp::Gt, Value::Int64(x))),
        aggregate: ("id", AggKind::Count),
    }
}

fn count_where_url_eq_query(url: &str) -> SimpleQuery<'static> {
    SimpleQuery {
        filter: Some(("url", CmpOp::Eq, Value::String(url.to_string()))),
        aggregate: ("id", AggKind::Count),
    }
}

fn sum_dur_query() -> SimpleQuery<'static> {
    SimpleQuery {
        filter: None,
        aggregate: ("dur", AggKind::Sum),
    }
}

fn crosscheck<F>(col_table: &Table, row_table: &RowTable, col_query: &SimpleQuery, row_query: F)
where
    F: Fn(&RowTable) -> i64,
{
    let col_result = minihouse::query::execute(col_table, col_query);
    let row_value = row_query(row_table);
    assert_eq!(
        col_result,
        Some(Value::Int64(row_value)),
        "engines disagree at"
    );
    println!("cross-check passed")
}
