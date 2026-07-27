# minihouse

A small in-memory columnar OLAP engine, built to explore why column stores are fast.

## What's here

This is a Cargo workspace with two crates:

- **`minihouse`** — the engine itself. A library with no external dependencies.
- **`benchmarks`** — a binary that generates synthetic data, cross-checks `minihouse`'s query
  results against a naive row-oriented store for correctness, and benchmarks the two to
  quantify the columnar-vs-row performance difference.

## Core concepts

- [`Table`](minihouse/src/table.rs) — owns a schema (`Vec<(String, DataType)>`) and a list of
  `Block`s. `insert` validates that an inserted block's column names, order, and types match the
  schema exactly, and silently drops zero-row blocks. `scan` yields blocks in insertion order.
- [`Block`](minihouse/src/block.rs) — a chunk of columnar data: a set of named `Column`s, all with
  the same length (`num_rows`). Supports filtering by a boolean row mask.
- [`Column`](minihouse/src/column.rs) — a typed vector of values (`Int64`, `Float64`, or
  `String`). Supports filtering by a boolean mask, returning a new column with only the selected
  rows.
- [`DataType`](minihouse/src/lib.rs) / [`Value`](minihouse/src/value.rs) — `DataType` names the
  three supported column types; `Value` is a single typed scalar used for filter literals and
  aggregate results.

## Querying

A [`SimpleQuery`](minihouse/src/query.rs) is a single-column filter (optional) plus a
single-column aggregate:

- Filters compare a column against a literal `Value` using `CmpOp::Gt`, `Lt`, or `Eq`. `Gt`/`Lt`
  are only supported on numeric columns; `Eq` works on all types, including strings.
- Aggregates are `AggKind::Count`, `Sum`, `Min`, or `Max`. `Sum`, `Min`, and `Max` are not defined
  over string columns and will panic if requested.

[`query::execute`](minihouse/src/query/execute.rs) scans every block in the table, evaluates the
filter into a per-block boolean mask (if any), filters the aggregate column by that mask, and
folds the results through the chosen aggregate.

```rust
use minihouse::aggregate::AggKind;
use minihouse::query::{execute, CmpOp, SimpleQuery};
use minihouse::{Block, Column, DataType, Table, Value};

let mut table = Table::new(vec![
    ("id".to_string(), DataType::Int64),
    ("score".to_string(), DataType::Float64),
]);

table.insert(Block::new(
    vec![
        ("id".to_string(), Column::Int64(vec![1, 2, 3])),
        ("score".to_string(), Column::Float64(vec![10.0, 20.0, 30.0])),
    ],
    3,
));

let query = SimpleQuery {
    filter: Some(("id", CmpOp::Gt, Value::Int64(1))),
    aggregate: ("score", AggKind::Sum),
};

assert_eq!(execute(&table, &query), Some(Value::Float64(50.0)));
```

## Benchmarks

`cargo run -p benchmarks --release` generates a 10-million-row synthetic dataset into both a
`minihouse::Table` and a hand-rolled row-oriented store, cross-checks that `sum`/`count` queries
agree between the two, then benchmarks median wall-clock time (7 runs) at three filter
selectivities (~1%, ~50%, ~99%), printing a columnar-vs-row speed ratio for each. This exists to
empirically demonstrate the performance advantage of columnar scans over row-oriented ones.

See [`benchmarks/src/column_vs_row.rs`](benchmarks/src/column_vs_row.rs) for the benchmark logic.

## Development

```sh
# Run all unit tests across the workspace
cargo test

# Run the columnar-vs-row benchmarks
cargo run -p benchmarks --release
```

`minihouse` builds with `#![warn(clippy::all)]`.

## License

MIT — see [LICENSE](LICENSE).
