use crate::aggregate_factory::{AggKind, make_aggregate};
use crate::query::{CmpOp, eval_predicate};
use crate::table::Table;
use crate::value::Value;

#[derive(Debug, Clone)]
pub struct SimpleQuery<'a> {
    pub filter: Option<(&'a str, CmpOp, Value)>,
    pub aggregate: (&'a str, AggKind),
}

pub fn execute(table: &Table, q: &SimpleQuery) -> Option<Value> {
    let schema = table.schema();

    if let Some((col, _, _)) = &q.filter {
        schema
            .iter()
            .find(|(c, _)| c == col)
            .expect("table does not contain filter column");
    }

    let agg_column_dt = schema
        .iter()
        .find(|(c, _)| c == q.aggregate.0)
        .map(|(_, dt)| dt)
        .expect("table does not contain aggregate column");
    let mut agg = make_aggregate(q.aggregate.1, *agg_column_dt);

    for block in table.scan() {
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

    agg.result()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DataType;
    use crate::block::Block;
    use crate::column::Column;

    fn sample_schema() -> Vec<(String, DataType)> {
        vec![
            ("id".to_string(), DataType::Int64),
            ("name".to_string(), DataType::String),
            ("score".to_string(), DataType::Float64),
        ]
    }

    fn sample_block(ids: Vec<i64>, names: Vec<&str>, scores: Vec<f64>) -> Block {
        let n = ids.len();
        Block::new(
            vec![
                ("id".to_string(), Column::Int64(ids)),
                (
                    "name".to_string(),
                    Column::String(names.into_iter().map(String::from).collect()),
                ),
                ("score".to_string(), Column::Float64(scores)),
            ],
            n,
        )
    }

    #[test]
    fn execute_count_no_filter_single_block() {
        let mut table = Table::new(sample_schema());
        table.insert(sample_block(
            vec![1, 2, 3],
            vec!["a", "b", "c"],
            vec![1.0, 2.0, 3.0],
        ));

        let q = SimpleQuery {
            filter: None,
            aggregate: ("id", AggKind::Count),
        };
        assert_eq!(execute(&table, &q), Some(Value::Int64(3)));
    }

    #[test]
    fn execute_count_no_filter_multiple_blocks() {
        let mut table = Table::new(sample_schema());
        table.insert(sample_block(vec![1, 2], vec!["a", "b"], vec![1.0, 2.0]));
        table.insert(sample_block(
            vec![3, 4, 5],
            vec!["c", "d", "e"],
            vec![3.0, 4.0, 5.0],
        ));

        let q = SimpleQuery {
            filter: None,
            aggregate: ("id", AggKind::Count),
        };
        assert_eq!(execute(&table, &q), Some(Value::Int64(5)));
    }

    #[test]
    fn execute_sum_int64_no_filter() {
        let mut table = Table::new(sample_schema());
        table.insert(sample_block(vec![1, 2], vec!["a", "b"], vec![1.0, 2.0]));
        table.insert(sample_block(vec![3], vec!["c"], vec![3.0]));

        let q = SimpleQuery {
            filter: None,
            aggregate: ("id", AggKind::Sum),
        };
        assert_eq!(execute(&table, &q), Some(Value::Int64(6)));
    }

    #[test]
    fn execute_sum_float64_no_filter() {
        let mut table = Table::new(sample_schema());
        table.insert(sample_block(vec![1, 2], vec!["a", "b"], vec![1.5, 2.5]));
        table.insert(sample_block(vec![3], vec!["c"], vec![3.0]));

        let q = SimpleQuery {
            filter: None,
            aggregate: ("score", AggKind::Sum),
        };
        assert_eq!(execute(&table, &q), Some(Value::Float64(7.0)));
    }

    #[test]
    fn execute_min_int64_no_filter() {
        let mut table = Table::new(sample_schema());
        table.insert(sample_block(vec![5, 3], vec!["a", "b"], vec![1.0, 2.0]));
        table.insert(sample_block(vec![1, 9], vec!["c", "d"], vec![3.0, 4.0]));

        let q = SimpleQuery {
            filter: None,
            aggregate: ("id", AggKind::Min),
        };
        assert_eq!(execute(&table, &q), Some(Value::Int64(1)));
    }

    #[test]
    fn execute_max_float64_no_filter() {
        let mut table = Table::new(sample_schema());
        table.insert(sample_block(vec![1, 2], vec!["a", "b"], vec![1.5, 9.5]));
        table.insert(sample_block(vec![3], vec!["c"], vec![3.0]));

        let q = SimpleQuery {
            filter: None,
            aggregate: ("score", AggKind::Max),
        };
        assert_eq!(execute(&table, &q), Some(Value::Float64(9.5)));
    }

    #[test]
    fn execute_filter_eq_returns_aggregate_over_matching_rows() {
        let mut table = Table::new(sample_schema());
        table.insert(sample_block(
            vec![1, 2, 3],
            vec!["a", "b", "b"],
            vec![1.0, 2.0, 4.0],
        ));

        let q = SimpleQuery {
            filter: Some(("name", CmpOp::Eq, Value::String("b".to_string()))),
            aggregate: ("score", AggKind::Sum),
        };
        assert_eq!(execute(&table, &q), Some(Value::Float64(6.0)));
    }

    #[test]
    fn execute_filter_gt_int64() {
        let mut table = Table::new(sample_schema());
        table.insert(sample_block(
            vec![1, 2, 3],
            vec!["a", "b", "c"],
            vec![1.0, 2.0, 3.0],
        ));

        let q = SimpleQuery {
            filter: Some(("id", CmpOp::Gt, Value::Int64(1))),
            aggregate: ("id", AggKind::Count),
        };
        assert_eq!(execute(&table, &q), Some(Value::Int64(2)));
    }

    #[test]
    fn execute_filter_lt_float64() {
        let mut table = Table::new(sample_schema());
        table.insert(sample_block(
            vec![1, 2, 3],
            vec!["a", "b", "c"],
            vec![1.0, 2.0, 3.0],
        ));

        let q = SimpleQuery {
            filter: Some(("score", CmpOp::Lt, Value::Float64(3.0))),
            aggregate: ("id", AggKind::Sum),
        };
        assert_eq!(execute(&table, &q), Some(Value::Int64(3)));
    }

    #[test]
    fn execute_filter_applied_independently_per_block() {
        let mut table = Table::new(sample_schema());
        // Block 1: id > 1 matches only row 2 (id=2).
        table.insert(sample_block(vec![1, 2], vec!["a", "b"], vec![1.0, 2.0]));
        // Block 2: id > 1 matches rows 1 and 2 (id=5, id=9), not row 3 (id=0).
        table.insert(sample_block(
            vec![5, 9, 0],
            vec!["c", "d", "e"],
            vec![3.0, 4.0, 5.0],
        ));

        let q = SimpleQuery {
            filter: Some(("id", CmpOp::Gt, Value::Int64(1))),
            aggregate: ("id", AggKind::Count),
        };
        // Matches: id=2 (block 1) + id=5, id=9 (block 2) = 3 rows.
        assert_eq!(execute(&table, &q), Some(Value::Int64(3)));
    }

    #[test]
    fn execute_filter_matches_no_rows_returns_aggregate_default() {
        let mut table = Table::new(sample_schema());
        table.insert(sample_block(
            vec![1, 2, 3],
            vec!["a", "b", "c"],
            vec![1.0, 2.0, 3.0],
        ));

        let filter = || Some(("id", CmpOp::Gt, Value::Int64(100)));

        let count_q = SimpleQuery {
            filter: filter(),
            aggregate: ("id", AggKind::Count),
        };
        assert_eq!(execute(&table, &count_q), Some(Value::Int64(0)));

        let sum_q = SimpleQuery {
            filter: filter(),
            aggregate: ("id", AggKind::Sum),
        };
        assert_eq!(execute(&table, &sum_q), Some(Value::Int64(0)));

        let min_q = SimpleQuery {
            filter: filter(),
            aggregate: ("id", AggKind::Min),
        };
        assert_eq!(execute(&table, &min_q), None);
    }

    #[test]
    fn execute_filter_matches_all_rows_equivalent_to_no_filter() {
        let mut table = Table::new(sample_schema());
        table.insert(sample_block(
            vec![1, 2, 3],
            vec!["a", "b", "c"],
            vec![1.0, 2.0, 3.0],
        ));

        let no_filter_q = SimpleQuery {
            filter: None,
            aggregate: ("id", AggKind::Sum),
        };
        let all_match_q = SimpleQuery {
            filter: Some(("id", CmpOp::Gt, Value::Int64(0))),
            aggregate: ("id", AggKind::Sum),
        };
        assert_eq!(execute(&table, &no_filter_q), execute(&table, &all_match_q));
    }

    #[test]
    fn execute_filter_and_aggregate_same_column() {
        let mut table = Table::new(sample_schema());
        table.insert(sample_block(
            vec![1, 2, 3],
            vec!["a", "b", "c"],
            vec![1.0, 2.0, 3.0],
        ));

        let q = SimpleQuery {
            filter: Some(("id", CmpOp::Gt, Value::Int64(1))),
            aggregate: ("id", AggKind::Sum),
        };
        assert_eq!(execute(&table, &q), Some(Value::Int64(5)));
    }

    #[test]
    fn execute_on_empty_table_no_filter_returns_aggregate_default() {
        let table = Table::new(sample_schema());

        let count_q = SimpleQuery {
            filter: None,
            aggregate: ("id", AggKind::Count),
        };
        assert_eq!(execute(&table, &count_q), Some(Value::Int64(0)));

        let min_q = SimpleQuery {
            filter: None,
            aggregate: ("id", AggKind::Min),
        };
        assert_eq!(execute(&table, &min_q), None);

        let max_q = SimpleQuery {
            filter: None,
            aggregate: ("score", AggKind::Max),
        };
        assert_eq!(execute(&table, &max_q), None);
    }

    #[test]
    #[should_panic(expected = "table does not contain filter column")]
    fn execute_filter_column_not_in_schema_panics() {
        let table = Table::new(sample_schema());
        let q = SimpleQuery {
            filter: Some(("missing", CmpOp::Eq, Value::Int64(1))),
            aggregate: ("id", AggKind::Count),
        };
        execute(&table, &q);
    }

    #[test]
    #[should_panic(expected = "table does not contain aggregate column")]
    fn execute_aggregate_column_not_in_schema_panics() {
        let table = Table::new(sample_schema());
        let q = SimpleQuery {
            filter: None,
            aggregate: ("missing", AggKind::Count),
        };
        execute(&table, &q);
    }

    #[test]
    #[should_panic(expected = "sum over string column")]
    fn execute_incompatible_aggregate_kind_panics_even_on_empty_table() {
        let table = Table::new(sample_schema());
        let q = SimpleQuery {
            filter: None,
            aggregate: ("name", AggKind::Sum),
        };
        execute(&table, &q);
    }

    #[test]
    fn execute_invalid_filter_op_on_string_column_no_panic_when_table_empty() {
        let table = Table::new(sample_schema());
        let q = SimpleQuery {
            filter: Some(("name", CmpOp::Gt, Value::String("m".to_string()))),
            aggregate: ("id", AggKind::Count),
        };
        assert_eq!(execute(&table, &q), Some(Value::Int64(0)));
    }

    #[test]
    #[should_panic(expected = "Unsupported comparison")]
    fn execute_invalid_filter_op_on_string_column_panics_when_table_has_rows() {
        let mut table = Table::new(sample_schema());
        table.insert(sample_block(vec![1], vec!["a"], vec![1.0]));

        let q = SimpleQuery {
            filter: Some(("name", CmpOp::Gt, Value::String("m".to_string()))),
            aggregate: ("id", AggKind::Count),
        };
        execute(&table, &q);
    }

    #[test]
    #[should_panic(expected = "do not match")]
    fn execute_filter_value_type_mismatch_panics_only_with_rows() {
        let mut table = Table::new(sample_schema());
        table.insert(sample_block(vec![1], vec!["a"], vec![1.0]));

        let q = SimpleQuery {
            filter: Some(("score", CmpOp::Eq, Value::Int64(1))),
            aggregate: ("id", AggKind::Count),
        };
        execute(&table, &q);
    }
}
