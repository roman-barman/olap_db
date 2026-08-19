use crate::DataType;
use crate::aggregate::make_aggregate;
use crate::column::Column;
use crate::query::{CmpOp, SimpleQuery};
use crate::storage_error::StorageError;
use crate::table::Table;
use crate::value::Value;

pub fn execute(table: &Table, q: &SimpleQuery) -> Result<Option<Value>, StorageError> {
    let agg_column_dt = table
        .schema()
        .iter()
        .find(|(c, _)| c == q.aggregate.0)
        .map(|(_, dt)| dt)
        .ok_or_else(|| StorageError::ColumnNotFound(q.aggregate.0.to_string()))?;
    let mut agg = make_aggregate(q.aggregate.1, *agg_column_dt);
    let agg_col = q.aggregate.0;

    let mut columns: Vec<&str> = vec![agg_col];
    if let Some((filter_col, _, _)) = &q.filter {
        if *filter_col != agg_col {
            columns.push(filter_col);
        }
    }

    for block_result in table.scan(&columns)? {
        let block = block_result?;
        match &q.filter {
            Some((col, op, val)) => {
                let mask = eval_predicate(
                    block
                        .column(col)
                        .expect("invariant broken: block missing schema column"),
                    *op,
                    val,
                );
                let filtered = block
                    .column(q.aggregate.0)
                    .expect("invariant broken: block missing schema column")
                    .filter(&mask);
                agg.update(&filtered);
            }
            None => agg.update(
                block
                    .column(q.aggregate.0)
                    .expect("invariant broken: block missing schema column"),
            ),
        }
    }

    Ok(agg.result())
}

fn eval_predicate(col: &Column, op: CmpOp, value: &Value) -> Vec<bool> {
    assert_eq!(
        col.data_type(),
        value.data_type(),
        "Data types of column '{:?}' and value '{:?}' do not match",
        col.data_type(),
        value.data_type()
    );

    if value.data_type() == DataType::String && op != CmpOp::Eq {
        panic!("Unsupported comparison {op:?} for string values");
    }

    match (col, value) {
        (Column::Int64(v), Value::Int64(x)) => match op {
            CmpOp::Gt => cmp_loop(v, |a| a > x),
            CmpOp::Lt => cmp_loop(v, |a| a < x),
            CmpOp::Eq => cmp_loop(v, |a| a == x),
        },
        (Column::Float64(v), Value::Float64(x)) => match op {
            CmpOp::Gt => cmp_loop(v, |a| a > x),
            CmpOp::Lt => cmp_loop(v, |a| a < x),
            CmpOp::Eq => cmp_loop(v, |a| a == x),
        },
        (Column::String(v), Value::String(x)) => match op {
            CmpOp::Eq => {
                let x_bytes = x.as_bytes();
                (0..v.len()).map(|i| v.bytes_at(i) == x_bytes).collect()
            }
            _ => unreachable!("Unsupported comparison {op:?} for string values"),
        },
        _ => unreachable!(
            "Type mismatch survived assert: {:?} vs {:?}",
            value.data_type(),
            col.data_type()
        ),
    }
}

fn cmp_loop<T, F>(v: &[T], f: F) -> Vec<bool>
where
    F: Fn(&T) -> bool,
{
    v.iter().map(f).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aggregate::AggKind;
    use crate::block::Block;
    use crate::codec::Codec;
    use crate::string_column::StringColumn;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    // ---- fixtures ------------------------------------------------------

    fn string_column(values: &[&str]) -> StringColumn {
        let mut col = StringColumn::new();
        for v in values {
            col.push(v);
        }
        col
    }

    fn sample_schema() -> Vec<(String, DataType)> {
        vec![
            ("id".to_string(), DataType::Int64),
            ("name".to_string(), DataType::String),
            ("score".to_string(), DataType::Float64),
        ]
    }

    fn sample_block(ids: &[i64], names: &[&str], scores: &[f64]) -> Block {
        assert_eq!(ids.len(), names.len());
        assert_eq!(ids.len(), scores.len());
        Block::new(
            vec![
                ("id".to_string(), Column::Int64(ids.to_vec())),
                ("name".to_string(), Column::String(string_column(names))),
                ("score".to_string(), Column::Float64(scores.to_vec())),
            ],
            ids.len(),
        )
    }

    /// A table holding one part per block, so the scan walks several
    /// `PartReader`s. The `TempDir` must outlive the `Table` — bind it, don't
    /// discard it.
    fn table_of_parts(blocks: Vec<Block>) -> (TempDir, Table) {
        let root = TempDir::new().unwrap();
        let mut table =
            Table::create(root.path().join("tbl"), sample_schema(), Codec::Lz4).unwrap();
        for block in blocks {
            table.insert(&[block]).unwrap();
        }
        (root, table)
    }

    /// One `insert` call, so the blocks become separate chunks inside a single
    /// part — the other half of the scan: many blocks, one `PartReader`.
    fn table_of_one_part(blocks: Vec<Block>) -> (TempDir, Table) {
        let root = TempDir::new().unwrap();
        let mut table =
            Table::create(root.path().join("tbl"), sample_schema(), Codec::Lz4).unwrap();
        table.insert(&blocks).unwrap();
        (root, table)
    }

    fn empty_table() -> (TempDir, Table) {
        table_of_parts(vec![])
    }

    /// The on-disk directory of a part in a table built by the fixtures above,
    /// for the tests that damage individual column files.
    fn part_dir(root: &TempDir, id: usize) -> PathBuf {
        root.path().join("tbl").join(format!("part_{id:04}"))
    }

    // ---- assertions ----------------------------------------------------

    /// Most tests care about the aggregate, not the `Result` wrapper.
    fn run(table: &Table, q: &SimpleQuery) -> Option<Value> {
        execute(table, q).unwrap()
    }

    /// `StorageError` has no `PartialEq`, so failures are matched on the variant
    /// plus the rendered message — the convention `table.rs` and `part_reader.rs`
    /// already use.
    fn run_error(table: &Table, q: &SimpleQuery) -> StorageError {
        match execute(table, q) {
            Ok(v) => panic!("expected {q:?} to fail, got {v:?}"),
            Err(e) => e,
        }
    }

    // ---- aggregates without a filter -----------------------------------

    #[test]
    fn execute_count_no_filter_single_block() {
        let (_root, table) = table_of_one_part(vec![sample_block(
            &[1, 2, 3],
            &["a", "b", "c"],
            &[1.0, 2.0, 3.0],
        )]);

        let q = SimpleQuery {
            filter: None,
            aggregate: ("id", AggKind::Count),
        };
        assert_eq!(run(&table, &q), Some(Value::Int64(3)));
    }

    #[test]
    fn execute_count_no_filter_multiple_blocks() {
        let (_root, table) = table_of_parts(vec![
            sample_block(&[1, 2], &["a", "b"], &[1.0, 2.0]),
            sample_block(&[3, 4, 5], &["c", "d", "e"], &[3.0, 4.0, 5.0]),
        ]);

        let q = SimpleQuery {
            filter: None,
            aggregate: ("id", AggKind::Count),
        };
        assert_eq!(run(&table, &q), Some(Value::Int64(5)));
    }

    #[test]
    fn execute_sum_int64_no_filter() {
        let (_root, table) = table_of_parts(vec![
            sample_block(&[1, 2], &["a", "b"], &[1.0, 2.0]),
            sample_block(&[3], &["c"], &[3.0]),
        ]);

        let q = SimpleQuery {
            filter: None,
            aggregate: ("id", AggKind::Sum),
        };
        assert_eq!(run(&table, &q), Some(Value::Int64(6)));
    }

    #[test]
    fn execute_sum_float64_no_filter() {
        let (_root, table) = table_of_parts(vec![
            sample_block(&[1, 2], &["a", "b"], &[1.5, 2.5]),
            sample_block(&[3], &["c"], &[3.0]),
        ]);

        let q = SimpleQuery {
            filter: None,
            aggregate: ("score", AggKind::Sum),
        };
        assert_eq!(run(&table, &q), Some(Value::Float64(7.0)));
    }

    #[test]
    fn execute_min_int64_no_filter() {
        let (_root, table) = table_of_parts(vec![
            sample_block(&[5, 3], &["a", "b"], &[1.0, 2.0]),
            sample_block(&[1, 9], &["c", "d"], &[3.0, 4.0]),
        ]);

        let q = SimpleQuery {
            filter: None,
            aggregate: ("id", AggKind::Min),
        };
        assert_eq!(run(&table, &q), Some(Value::Int64(1)));
    }

    #[test]
    fn execute_max_float64_no_filter() {
        let (_root, table) = table_of_parts(vec![
            sample_block(&[1, 2], &["a", "b"], &[1.5, 9.5]),
            sample_block(&[3], &["c"], &[3.0]),
        ]);

        let q = SimpleQuery {
            filter: None,
            aggregate: ("score", AggKind::Max),
        };
        assert_eq!(run(&table, &q), Some(Value::Float64(9.5)));
    }

    /// The scan walks parts in id order and the aggregate accumulates across
    /// them — one `PartReader` per part, one running aggregate.
    #[test]
    fn execute_aggregates_across_parts() {
        let (_root, table) = table_of_parts(vec![
            sample_block(&[1, 2], &["a", "b"], &[1.0, 2.0]),
            sample_block(&[3], &["c"], &[3.0]),
            sample_block(&[4, 5], &["d", "e"], &[4.0, 5.0]),
        ]);

        let sum = SimpleQuery {
            filter: None,
            aggregate: ("id", AggKind::Sum),
        };
        let min = SimpleQuery {
            filter: None,
            aggregate: ("id", AggKind::Min),
        };
        let max = SimpleQuery {
            filter: None,
            aggregate: ("score", AggKind::Max),
        };

        assert_eq!(run(&table, &sum), Some(Value::Int64(15)));
        assert_eq!(run(&table, &min), Some(Value::Int64(1)));
        assert_eq!(run(&table, &max), Some(Value::Float64(5.0)));
    }

    /// The same totals with every block in one part, so the chunks are read by a
    /// single `PartReader` instead of one per block.
    #[test]
    fn execute_aggregates_blocks_within_one_part() {
        let (_root, table) = table_of_one_part(vec![
            sample_block(&[1, 2], &["a", "b"], &[1.0, 2.0]),
            sample_block(&[3], &["c"], &[3.0]),
            sample_block(&[4, 5], &["d", "e"], &[4.0, 5.0]),
        ]);

        let sum = SimpleQuery {
            filter: None,
            aggregate: ("id", AggKind::Sum),
        };
        let min = SimpleQuery {
            filter: None,
            aggregate: ("id", AggKind::Min),
        };

        assert_eq!(run(&table, &sum), Some(Value::Int64(15)));
        assert_eq!(run(&table, &min), Some(Value::Int64(1)));
    }

    // ---- filters -------------------------------------------------------

    #[test]
    fn execute_filter_eq_returns_aggregate_over_matching_rows() {
        let (_root, table) = table_of_one_part(vec![sample_block(
            &[1, 2, 3],
            &["a", "b", "b"],
            &[1.0, 2.0, 4.0],
        )]);

        let q = SimpleQuery {
            filter: Some(("name", CmpOp::Eq, Value::String("b".to_string()))),
            aggregate: ("score", AggKind::Sum),
        };
        assert_eq!(run(&table, &q), Some(Value::Float64(6.0)));
    }

    #[test]
    fn execute_filter_gt_int64() {
        let (_root, table) = table_of_one_part(vec![sample_block(
            &[1, 2, 3],
            &["a", "b", "c"],
            &[1.0, 2.0, 3.0],
        )]);

        let q = SimpleQuery {
            filter: Some(("id", CmpOp::Gt, Value::Int64(1))),
            aggregate: ("id", AggKind::Count),
        };
        assert_eq!(run(&table, &q), Some(Value::Int64(2)));
    }

    #[test]
    fn execute_filter_lt_float64() {
        let (_root, table) = table_of_one_part(vec![sample_block(
            &[1, 2, 3],
            &["a", "b", "c"],
            &[1.0, 2.0, 3.0],
        )]);

        let q = SimpleQuery {
            filter: Some(("score", CmpOp::Lt, Value::Float64(3.0))),
            aggregate: ("id", AggKind::Sum),
        };
        assert_eq!(run(&table, &q), Some(Value::Int64(3)));
    }

    /// A projection is built as `[aggregate, filter]`, but `PartReader` hands the
    /// columns back in *schema* order — so `execute` must look them up by name.
    /// Aggregating `score` (schema index 2) while filtering on `id` (index 0)
    /// inverts the order the projection was built in.
    #[test]
    fn a_filter_column_ordered_before_the_aggregate_column_works() {
        let (_root, table) = table_of_one_part(vec![sample_block(
            &[1, 2, 3],
            &["a", "b", "c"],
            &[1.5, 2.5, 4.0],
        )]);

        let q = SimpleQuery {
            filter: Some(("id", CmpOp::Gt, Value::Int64(1))),
            aggregate: ("score", AggKind::Sum),
        };
        assert_eq!(run(&table, &q), Some(Value::Float64(6.5)));
    }

    #[test]
    fn execute_filter_applied_independently_per_block() {
        let (_root, table) = table_of_parts(vec![
            // Block 1: id > 1 matches only row 2 (id=2).
            sample_block(&[1, 2], &["a", "b"], &[1.0, 2.0]),
            // Block 2: id > 1 matches rows 1 and 2 (id=5, id=9), not row 3 (id=0).
            sample_block(&[5, 9, 0], &["c", "d", "e"], &[3.0, 4.0, 5.0]),
        ]);

        let q = SimpleQuery {
            filter: Some(("id", CmpOp::Gt, Value::Int64(1))),
            aggregate: ("id", AggKind::Count),
        };
        // Matches: id=2 (block 1) + id=5, id=9 (block 2) = 3 rows.
        assert_eq!(run(&table, &q), Some(Value::Int64(3)));
    }

    #[test]
    fn execute_filter_matches_no_rows_returns_aggregate_default() {
        let (_root, table) = table_of_one_part(vec![sample_block(
            &[1, 2, 3],
            &["a", "b", "c"],
            &[1.0, 2.0, 3.0],
        )]);

        let filter = || Some(("id", CmpOp::Gt, Value::Int64(100)));

        let count_q = SimpleQuery {
            filter: filter(),
            aggregate: ("id", AggKind::Count),
        };
        assert_eq!(run(&table, &count_q), Some(Value::Int64(0)));

        let sum_q = SimpleQuery {
            filter: filter(),
            aggregate: ("id", AggKind::Sum),
        };
        assert_eq!(run(&table, &sum_q), Some(Value::Int64(0)));

        let min_q = SimpleQuery {
            filter: filter(),
            aggregate: ("id", AggKind::Min),
        };
        assert_eq!(run(&table, &min_q), None);
    }

    #[test]
    fn execute_filter_matches_all_rows_equivalent_to_no_filter() {
        let (_root, table) = table_of_one_part(vec![sample_block(
            &[1, 2, 3],
            &["a", "b", "c"],
            &[1.0, 2.0, 3.0],
        )]);

        let no_filter_q = SimpleQuery {
            filter: None,
            aggregate: ("id", AggKind::Sum),
        };
        let all_match_q = SimpleQuery {
            filter: Some(("id", CmpOp::Gt, Value::Int64(0))),
            aggregate: ("id", AggKind::Sum),
        };
        assert_eq!(run(&table, &no_filter_q), run(&table, &all_match_q));
    }

    /// `execute` drops the filter column from the projection when it is also the
    /// aggregate column. That guard is load-bearing: `PartReader::open` *asserts*
    /// on a duplicated projection entry, so without it this query would panic
    /// rather than merely read the column twice.
    #[test]
    fn execute_filter_and_aggregate_same_column() {
        let (_root, table) = table_of_one_part(vec![sample_block(
            &[1, 2, 3],
            &["a", "b", "c"],
            &[1.0, 2.0, 3.0],
        )]);

        let q = SimpleQuery {
            filter: Some(("id", CmpOp::Gt, Value::Int64(1))),
            aggregate: ("id", AggKind::Sum),
        };
        assert_eq!(run(&table, &q), Some(Value::Int64(5)));
    }

    #[test]
    fn execute_on_empty_table_no_filter_returns_aggregate_default() {
        let (_root, table) = empty_table();

        let count_q = SimpleQuery {
            filter: None,
            aggregate: ("id", AggKind::Count),
        };
        assert_eq!(run(&table, &count_q), Some(Value::Int64(0)));

        let min_q = SimpleQuery {
            filter: None,
            aggregate: ("id", AggKind::Min),
        };
        assert_eq!(run(&table, &min_q), None);

        let max_q = SimpleQuery {
            filter: None,
            aggregate: ("score", AggKind::Max),
        };
        assert_eq!(run(&table, &max_q), None);
    }

    // ---- projection ----------------------------------------------------

    /// The point of projecting: `execute` opens only the column files it needs,
    /// so the rest of the part can be missing from disk and the query still
    /// answers. Nothing else in this suite would notice a regression to reading
    /// every column.
    #[test]
    fn execute_reads_only_the_projected_columns() {
        let (root, table) = table_of_one_part(vec![sample_block(
            &[1, 2, 3],
            &["a", "b", "c"],
            &[1.0, 2.0, 3.0],
        )]);
        let part = part_dir(&root, 0);
        fs::remove_file(part.join("score.bin")).unwrap();
        fs::remove_file(part.join("name.data.bin")).unwrap();
        fs::remove_file(part.join("name.offsets.bin")).unwrap();

        let q = SimpleQuery {
            filter: None,
            aggregate: ("id", AggKind::Count),
        };
        assert_eq!(run(&table, &q), Some(Value::Int64(3)));
    }

    /// The filter column is projected alongside the aggregate column, so which
    /// files a query needs depends on the filter as well.
    #[test]
    fn execute_projects_the_filter_column_too() {
        let (root, table) = table_of_one_part(vec![sample_block(
            &[1, 2, 3],
            &["a", "b", "b"],
            &[1.0, 2.0, 3.0],
        )]);
        fs::remove_file(part_dir(&root, 0).join("score.bin")).unwrap();

        let on_name = SimpleQuery {
            filter: Some(("name", CmpOp::Eq, Value::String("b".to_string()))),
            aggregate: ("id", AggKind::Sum),
        };
        assert_eq!(run(&table, &on_name), Some(Value::Int64(5)));

        let on_score = SimpleQuery {
            filter: Some(("score", CmpOp::Gt, Value::Float64(1.0))),
            aggregate: ("id", AggKind::Sum),
        };
        assert_eq!(
            run_error(&table, &on_score).to_string(),
            "corrupt: column file 'score.bin' missing"
        );
    }

    // ---- error paths ---------------------------------------------------

    #[test]
    fn execute_propagates_a_corrupt_part() {
        let (root, table) = table_of_one_part(vec![sample_block(&[1], &["a"], &[1.0])]);
        fs::remove_file(part_dir(&root, 0).join("id.bin")).unwrap();

        let q = SimpleQuery {
            filter: None,
            aggregate: ("id", AggKind::Count),
        };
        let e = run_error(&table, &q);

        assert!(matches!(e, StorageError::Corrupt(_)), "{e:?}");
        assert_eq!(e.to_string(), "corrupt: column file 'id.bin' missing");
    }

    /// The aggregate column is looked up in `table.schema()` before any scan, so
    /// a missing one is reported whether or not the table holds rows — unlike
    /// the filter column below.
    #[test]
    fn a_missing_aggregate_column_is_an_error() {
        let (_empty_root, empty) = empty_table();
        let (_root, populated) = table_of_one_part(vec![sample_block(&[1], &["a"], &[1.0])]);

        let q = SimpleQuery {
            filter: None,
            aggregate: ("missing", AggKind::Count),
        };

        for table in [&empty, &populated] {
            let e = run_error(table, &q);
            assert!(
                matches!(e, StorageError::ColumnNotFound(ref n) if n == "missing"),
                "{e:?}"
            );
            assert_eq!(e.to_string(), "column 'missing' not found");
        }
    }

    /// There is no schema check for the filter column — it rides along in the
    /// projection, so `PartReader::open` is what rejects it.
    #[test]
    fn a_missing_filter_column_is_an_error_once_the_table_has_rows() {
        let (_root, table) = table_of_one_part(vec![sample_block(&[1], &["a"], &[1.0])]);

        let q = SimpleQuery {
            filter: Some(("missing", CmpOp::Eq, Value::Int64(1))),
            aggregate: ("id", AggKind::Count),
        };
        let e = run_error(&table, &q);

        assert!(
            matches!(e, StorageError::ColumnNotFound(ref n) if n == "missing"),
            "{e:?}"
        );
        assert_eq!(e.to_string(), "column 'missing' not found");
    }

    /// The flip side: with no parts to open, nothing ever resolves the
    /// projection, so a filter naming a column the table does not have returns a
    /// result instead of an error. A gap left by dropping the eager schema
    /// check, not intended design.
    #[test]
    fn a_missing_filter_column_goes_unreported_on_an_empty_table() {
        let (_root, table) = empty_table();

        let q = SimpleQuery {
            filter: Some(("missing", CmpOp::Eq, Value::Int64(1))),
            aggregate: ("id", AggKind::Count),
        };
        assert_eq!(run(&table, &q), Some(Value::Int64(0)));
    }

    /// `make_aggregate` runs before the scan, so an aggregate the column type
    /// cannot support fails even with nothing to read.
    #[test]
    #[should_panic(expected = "sum over string column")]
    fn execute_incompatible_aggregate_kind_panics_even_on_empty_table() {
        let (_root, table) = empty_table();

        let q = SimpleQuery {
            filter: None,
            aggregate: ("name", AggKind::Sum),
        };
        let _ = execute(&table, &q);
    }

    /// Predicate validation happens per block, so an unsupported comparison is
    /// invisible until there is a row to apply it to.
    #[test]
    fn execute_invalid_filter_op_on_string_column_no_panic_when_table_empty() {
        let (_root, table) = empty_table();

        let q = SimpleQuery {
            filter: Some(("name", CmpOp::Gt, Value::String("m".to_string()))),
            aggregate: ("id", AggKind::Count),
        };
        assert_eq!(run(&table, &q), Some(Value::Int64(0)));
    }

    #[test]
    #[should_panic(expected = "Unsupported comparison")]
    fn execute_invalid_filter_op_on_string_column_panics_when_table_has_rows() {
        let (_root, table) = table_of_one_part(vec![sample_block(&[1], &["a"], &[1.0])]);

        let q = SimpleQuery {
            filter: Some(("name", CmpOp::Gt, Value::String("m".to_string()))),
            aggregate: ("id", AggKind::Count),
        };
        let _ = execute(&table, &q);
    }

    #[test]
    #[should_panic(expected = "do not match")]
    fn execute_filter_value_type_mismatch_panics_only_with_rows() {
        let (_root, table) = table_of_one_part(vec![sample_block(&[1], &["a"], &[1.0])]);

        let q = SimpleQuery {
            filter: Some(("score", CmpOp::Eq, Value::Int64(1))),
            aggregate: ("id", AggKind::Count),
        };
        let _ = execute(&table, &q);
    }

    // ---- eval_predicate ------------------------------------------------

    #[test]
    fn eval_predicate_int64_eq_returns_matching_mask() {
        let col = Column::Int64(vec![1, 2, 3, 2]);
        let mask = eval_predicate(&col, CmpOp::Eq, &Value::Int64(2));
        assert_eq!(mask, vec![false, true, false, true]);
    }

    #[test]
    fn eval_predicate_int64_gt_returns_matching_mask() {
        let col = Column::Int64(vec![1, 2, 3, 2]);
        let mask = eval_predicate(&col, CmpOp::Gt, &Value::Int64(2));
        assert_eq!(mask, vec![false, false, true, false]);
    }

    #[test]
    fn eval_predicate_int64_lt_returns_matching_mask() {
        let col = Column::Int64(vec![1, 2, 3, 2]);
        let mask = eval_predicate(&col, CmpOp::Lt, &Value::Int64(2));
        assert_eq!(mask, vec![true, false, false, false]);
    }

    #[test]
    fn eval_predicate_float64_eq_returns_matching_mask() {
        let col = Column::Float64(vec![1.0, 2.5, 3.0, 2.5]);
        let mask = eval_predicate(&col, CmpOp::Eq, &Value::Float64(2.5));
        assert_eq!(mask, vec![false, true, false, true]);
    }

    #[test]
    fn eval_predicate_float64_gt_returns_matching_mask() {
        let col = Column::Float64(vec![1.0, 2.5, 3.0, 2.5]);
        let mask = eval_predicate(&col, CmpOp::Gt, &Value::Float64(2.5));
        assert_eq!(mask, vec![false, false, true, false]);
    }

    #[test]
    fn eval_predicate_float64_lt_returns_matching_mask() {
        let col = Column::Float64(vec![1.0, 2.5, 3.0, 2.5]);
        let mask = eval_predicate(&col, CmpOp::Lt, &Value::Float64(2.5));
        assert_eq!(mask, vec![true, false, false, false]);
    }

    #[test]
    fn eval_predicate_string_eq_returns_matching_mask() {
        let col = Column::String(string_column(&["a", "b", "a"]));
        let mask = eval_predicate(&col, CmpOp::Eq, &Value::String("a".into()));
        assert_eq!(mask, vec![true, false, true]);
    }

    #[test]
    #[should_panic(expected = "Unsupported comparison")]
    fn eval_predicate_string_gt_panics() {
        let col = Column::String(string_column(&["a"]));
        eval_predicate(&col, CmpOp::Gt, &Value::String("a".into()));
    }

    #[test]
    #[should_panic(expected = "Unsupported comparison")]
    fn eval_predicate_string_lt_panics() {
        let col = Column::String(string_column(&["a"]));
        eval_predicate(&col, CmpOp::Lt, &Value::String("a".into()));
    }

    #[test]
    #[should_panic(expected = "do not match")]
    fn eval_predicate_int64_column_float64_value_panics() {
        let col = Column::Int64(vec![1, 2, 3]);
        eval_predicate(&col, CmpOp::Eq, &Value::Float64(1.0));
    }

    #[test]
    #[should_panic(expected = "do not match")]
    fn eval_predicate_int64_column_string_value_panics() {
        let col = Column::Int64(vec![1, 2, 3]);
        eval_predicate(&col, CmpOp::Eq, &Value::String("1".into()));
    }

    #[test]
    #[should_panic(expected = "do not match")]
    fn eval_predicate_string_column_int64_value_panics() {
        let col = Column::String(string_column(&["a"]));
        eval_predicate(&col, CmpOp::Eq, &Value::Int64(1));
    }

    #[test]
    fn eval_predicate_on_empty_int64_column_returns_empty_mask() {
        let col = Column::new(DataType::Int64);
        let mask = eval_predicate(&col, CmpOp::Eq, &Value::Int64(1));
        assert_eq!(mask, Vec::<bool>::new());
    }

    #[test]
    fn eval_predicate_on_empty_float64_column_returns_empty_mask() {
        let col = Column::new(DataType::Float64);
        let mask = eval_predicate(&col, CmpOp::Eq, &Value::Float64(1.0));
        assert_eq!(mask, Vec::<bool>::new());
    }

    #[test]
    fn eval_predicate_on_empty_string_column_returns_empty_mask() {
        let col = Column::new(DataType::String);
        let mask = eval_predicate(&col, CmpOp::Eq, &Value::String("a".into()));
        assert_eq!(mask, Vec::<bool>::new());
    }
}
