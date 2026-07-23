use crate::aggregate::Aggregate;
use crate::column::Column;
use crate::value::Value;

#[derive(Debug, Clone, Default)]
pub(super) struct MaxI64 {
    max: Option<i64>,
}

impl Aggregate for MaxI64 {
    fn update(&mut self, col: &Column) {
        match col {
            Column::Int64(v) => {
                let block_max = v.iter().copied().max();
                self.max = match (self.max, block_max) {
                    (None, m) => m,
                    (m, None) => m,
                    (Some(a), Some(b)) => Some(a.max(b)),
                };
            }
            other => panic!("MaxI64: expected Int64 column, got {:?}", other.data_type()),
        }
    }

    fn result(&self) -> Option<Value> {
        self.max.map(Value::Int64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DataType;

    #[test]
    fn max_i64_default_result_is_none() {
        let max = MaxI64::default();
        assert_eq!(max.result(), None);
    }

    #[test]
    fn max_i64_update_single_column_computes_max() {
        let mut max = MaxI64::default();
        max.update(&Column::Int64(vec![3, 1, 2]));
        assert_eq!(max.result(), Some(Value::Int64(3)));
    }

    #[test]
    fn max_i64_update_accumulates_across_multiple_calls() {
        let mut max = MaxI64::default();
        max.update(&Column::Int64(vec![5, 3]));
        max.update(&Column::Int64(vec![4, 9, 1]));
        assert_eq!(max.result(), Some(Value::Int64(9)));
    }

    #[test]
    fn max_i64_update_with_negative_numbers() {
        let mut max = MaxI64::default();
        max.update(&Column::Int64(vec![-10, -3, -2]));
        assert_eq!(max.result(), Some(Value::Int64(-2)));
    }

    #[test]
    fn max_i64_update_with_empty_column_is_noop() {
        let mut max = MaxI64::default();
        max.update(&Column::Int64(vec![5, 2]));
        max.update(&Column::new(DataType::Int64));
        assert_eq!(max.result(), Some(Value::Int64(5)));
    }

    #[test]
    fn max_i64_update_empty_column_before_data_is_none_then_updates() {
        let mut max = MaxI64::default();
        max.update(&Column::new(DataType::Int64));
        assert_eq!(max.result(), None);
        max.update(&Column::Int64(vec![7, 2, 9]));
        assert_eq!(max.result(), Some(Value::Int64(9)));
    }

    #[test]
    #[should_panic(expected = "MaxI64: expected Int64 column")]
    fn max_i64_update_panics_on_wrong_column_type() {
        let mut max = MaxI64::default();
        max.update(&Column::Float64(vec![1.0]));
    }

    #[test]
    fn max_i64_result_is_idempotent_without_update() {
        let mut max = MaxI64::default();
        max.update(&Column::Int64(vec![3, 1, 2]));
        assert_eq!(max.result(), Some(Value::Int64(3)));
        assert_eq!(max.result(), Some(Value::Int64(3)));
    }

    #[test]
    fn max_i64_works_through_aggregate_trait_object() {
        let mut agg: Box<dyn Aggregate> = Box::new(MaxI64::default());
        agg.update(&Column::Int64(vec![5, 3]));
        agg.update(&Column::Int64(vec![9]));
        assert_eq!(agg.result(), Some(Value::Int64(9)));
    }
}
