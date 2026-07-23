use crate::aggregate::Aggregate;
use crate::column::Column;
use crate::value::Value;

#[derive(Debug, Clone, Default)]
pub(super) struct MinI64 {
    min: Option<i64>,
}

impl Aggregate for MinI64 {
    fn update(&mut self, col: &Column) {
        match col {
            Column::Int64(v) => {
                let block_min = v.iter().copied().min();
                self.min = match (self.min, block_min) {
                    (None, m) => m,
                    (m, None) => m,
                    (Some(a), Some(b)) => Some(a.min(b)),
                };
            }
            other => panic!("MinI64: expected Int64 column, got {:?}", other.data_type()),
        }
    }

    fn result(&self) -> Option<Value> {
        self.min.map(Value::Int64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DataType;

    #[test]
    fn min_i64_default_result_is_none() {
        let min = MinI64::default();
        assert_eq!(min.result(), None);
    }

    #[test]
    fn min_i64_update_single_column_computes_min() {
        let mut min = MinI64::default();
        min.update(&Column::Int64(vec![3, 1, 2]));
        assert_eq!(min.result(), Some(Value::Int64(1)));
    }

    #[test]
    fn min_i64_update_accumulates_across_multiple_calls() {
        let mut min = MinI64::default();
        min.update(&Column::Int64(vec![5, 3]));
        min.update(&Column::Int64(vec![4, 1, 9]));
        assert_eq!(min.result(), Some(Value::Int64(1)));
    }

    #[test]
    fn min_i64_update_with_negative_numbers() {
        let mut min = MinI64::default();
        min.update(&Column::Int64(vec![10, -3, -2]));
        assert_eq!(min.result(), Some(Value::Int64(-3)));
    }

    #[test]
    fn min_i64_update_with_empty_column_is_noop() {
        let mut min = MinI64::default();
        min.update(&Column::Int64(vec![5, 2]));
        min.update(&Column::new(DataType::Int64));
        assert_eq!(min.result(), Some(Value::Int64(2)));
    }

    #[test]
    fn min_i64_update_empty_column_before_data_is_none_then_updates() {
        let mut min = MinI64::default();
        min.update(&Column::new(DataType::Int64));
        assert_eq!(min.result(), None);
        min.update(&Column::Int64(vec![7, 2, 9]));
        assert_eq!(min.result(), Some(Value::Int64(2)));
    }

    #[test]
    #[should_panic(expected = "MinI64: expected Int64 column")]
    fn min_i64_update_panics_on_wrong_column_type() {
        let mut min = MinI64::default();
        min.update(&Column::Float64(vec![1.0]));
    }

    #[test]
    fn min_i64_result_is_idempotent_without_update() {
        let mut min = MinI64::default();
        min.update(&Column::Int64(vec![3, 1, 2]));
        assert_eq!(min.result(), Some(Value::Int64(1)));
        assert_eq!(min.result(), Some(Value::Int64(1)));
    }

    #[test]
    fn min_i64_works_through_aggregate_trait_object() {
        let mut agg: Box<dyn Aggregate> = Box::new(MinI64::default());
        agg.update(&Column::Int64(vec![5, 3]));
        agg.update(&Column::Int64(vec![1]));
        assert_eq!(agg.result(), Some(Value::Int64(1)));
    }
}
