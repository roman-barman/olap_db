# minihouse

A small disk-backed columnar OLAP engine, built to explore why column stores are fast.

## What's here

This is a Cargo workspace with two crates:

- **`minihouse`** — the engine itself. A library with a small dependency footprint (`thiserror`
  for error types, `lz4_flex` for optional block compression).
- **`benchmarks`** — a binary that generates synthetic data, cross-checks `minihouse`'s query
  results against a naive row-oriented store for correctness, and benchmarks the two to
  quantify the columnar-vs-row performance difference.

## Core concepts

- [`Table`](minihouse/src/table.rs) — a directory-backed table. `Table::create` writes a fresh
  table (a `Schema` plus a `Codec`) to a new directory; `Table::open` re-opens one created
  earlier. `insert` validates that each inserted block's column names, order, and types match the
  schema exactly, silently drops zero-row blocks, and writes the rest as a new on-disk part.
- [`Schema`](minihouse/src/core/schema.rs) — an ordered, validated list of `(name, DataType)`
  pairs (rejects empty schemas, empty names, and duplicate names). Column order is also the
  on-disk column layout.
- [`Block`](minihouse/src/core/block.rs) — a chunk of columnar data: a set of named `Column`s, all
  with the same length.
- [`Column`](minihouse/src/core/column.rs) — a typed vector of values (`Int64`, `Float64`, or
  `String`), built incrementally with `push_i64`/`push_f64`/`push_str`.
- [`DataType`](minihouse/src/core/data_type.rs) / [`Value`](minihouse/src/core/value.rs) —
  `DataType` names the three supported column types; `Value` is a single typed scalar used for
  filter literals and aggregate results.
- [`Codec`](minihouse/src/storage/codec.rs) — `Codec::None` or `Codec::Lz4`, chosen once per table
  at `Table::create` time and persisted alongside the schema.

On disk, a table is a directory holding a `schema.txt` (format version, codec, one `column=`
line per column) plus one `part_NNNN/` directory per `insert` call.

## Querying

A [`SimpleQuery`](minihouse/src/query.rs) is a single-column filter (optional) plus a
single-column aggregate:

- Filters compare a column against a literal `Value` using `CmpOp::Gt`, `Lt`, or `Eq`. `Gt`/`Lt`
  are only supported on numeric columns; `Eq` works on all types, including strings.
- Aggregates are `AggKind::Count`, `Sum`, `Min`, or `Max`. `Sum`, `Min`, and `Max` are not defined
  over string columns and will panic if requested.

[`query::execute`](minihouse/src/query/execute.rs) scans the table's stored parts, evaluates the
filter into a per-block boolean mask (if any), filters the aggregate column by that mask, and
folds the results through the chosen aggregate. It returns `Result<Option<Value>, StorageError>`,
propagating any I/O or on-disk corruption error instead of panicking.

```rust
use minihouse::aggregate::AggKind;
use minihouse::query::{execute, CmpOp, SimpleQuery};
use minihouse::{Block, Codec, Column, DataType, Schema, Table, Value};
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let schema = Schema::new(vec![
        ("id".to_string(), DataType::Int64),
        ("score".to_string(), DataType::Float64),
    ])?;

    let mut table = Table::create(PathBuf::from("/tmp/minihouse_example"), schema, Codec::Lz4)?;

    table.insert(&[Block::new(
        vec![
            ("id".to_string(), Column::Int64(vec![1, 2, 3])),
            ("score".to_string(), Column::Float64(vec![10.0, 20.0, 30.0])),
        ],
        3,
    )])?;

    let query = SimpleQuery {
        filter: Some(("id", CmpOp::Gt, Value::Int64(1))),
        aggregate: ("score", AggKind::Sum),
    };

    assert_eq!(execute(&table, &query)?, Some(Value::Float64(50.0)));
    Ok(())
}
```

## Public API

The supported public surface is `Table` (`create`/`open`/`insert`), `Schema`, `Block::new`,
`Column`, `Value`, `Codec`, and `query::{execute, SimpleQuery, CmpOp}` /
`aggregate::AggKind`. Internal helpers such as `Table::scan`, `Block`/`Column` filtering, and
type-introspection methods are private implementation details, not part of the crate's API.

## Benchmarks

`cargo run -p benchmarks --release` generates a 10-million-row synthetic dataset into both a
`minihouse::Table` and a hand-rolled row-oriented store, cross-checks that `sum`/`count` queries
agree between the two, then benchmarks median wall-clock time (7 runs) at three filter
selectivities (~1%, ~50%, ~99%), printing a columnar-vs-row speed ratio for each. This exists to
empirically demonstrate the performance advantage of columnar scans over row-oriented ones.

See [`benchmarks/src/bench_iter_2.rs`](benchmarks/src/bench_iter_2.rs) for the benchmark logic,
and [Iteration 1](benchmarks/results/ITERATION%201.md) / [Iteration 2](benchmarks/results/ITERATION%202.md)
for recorded runs.

## Development

```sh
# Run all unit tests across the workspace
cargo test

# Run the columnar-vs-row benchmarks
cargo run -p benchmarks --release
```

`minihouse` builds with `#![warn(clippy::all)]` and `#![deny(unreachable_pub)]`, so anything
reachable from outside the crate must be intentionally `pub`.

## License

MIT — see [LICENSE](LICENSE).
