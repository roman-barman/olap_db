use crate::DataType;
use crate::aggregate::make_aggregate;
use crate::column::Column;
use crate::query::{CmpOp, SimpleQuery};
use crate::table::Table;
use crate::value::Value;

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
            CmpOp::Eq => cmp_loop(v, |a| a == x),
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
    use crate::DataType;
    use crate::aggregate::AggKind;
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
        let col = Column::String(vec!["a".into(), "b".into(), "a".into()]);
        let mask = eval_predicate(&col, CmpOp::Eq, &Value::String("a".into()));
        assert_eq!(mask, vec![true, false, true]);
    }

    #[test]
    #[should_panic(expected = "Unsupported comparison")]
    fn eval_predicate_string_gt_panics() {
        let col = Column::String(vec!["a".into()]);
        eval_predicate(&col, CmpOp::Gt, &Value::String("a".into()));
    }

    #[test]
    #[should_panic(expected = "Unsupported comparison")]
    fn eval_predicate_string_lt_panics() {
        let col = Column::String(vec!["a".into()]);
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
        let col = Column::String(vec!["a".into()]);
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
