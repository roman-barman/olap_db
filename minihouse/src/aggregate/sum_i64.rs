use crate::aggregate::Aggregate;
use crate::column::Column;
use crate::value::Value;

#[derive(Debug, Clone, Default)]
pub(super) struct SumI64 {
    sum: i64,
}

impl Aggregate for SumI64 {
    fn update(&mut self, col: &Column) {
        match col {
            Column::Int64(v) => {
                self.sum = v
                    .iter()
                    .try_fold(self.sum, |a, &x| a.checked_add(x))
                    .expect("sum overflows i64");
            }
            other => panic!("SumI64: expected Int64 column, got {:?}", other.data_type()),
        }
    }

    fn result(&self) -> Option<Value> {
        Some(Value::Int64(self.sum))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DataType;

    #[test]
    fn sum_i64_default_result_is_zero() {
        let sum = SumI64::default();
        assert_eq!(sum.result(), Some(Value::Int64(0)));
    }

    #[test]
    fn sum_i64_update_int64_column_computes_sum() {
        let mut sum = SumI64::default();
        sum.update(&Column::Int64(vec![1, 2, 3]));
        assert_eq!(sum.result(), Some(Value::Int64(6)));
    }

    #[test]
    fn sum_i64_update_accumulates_across_multiple_calls() {
        let mut sum = SumI64::default();
        sum.update(&Column::Int64(vec![1, 2, 3]));
        sum.update(&Column::Int64(vec![4, 5]));
        assert_eq!(sum.result(), Some(Value::Int64(15)));
    }

    #[test]
    fn sum_i64_update_with_negative_numbers() {
        let mut sum = SumI64::default();
        sum.update(&Column::Int64(vec![10, -3, -2]));
        assert_eq!(sum.result(), Some(Value::Int64(5)));
    }

    #[test]
    fn sum_i64_update_with_empty_column_is_noop() {
        let mut sum = SumI64::default();
        sum.update(&Column::Int64(vec![1, 2]));
        sum.update(&Column::new(DataType::Int64));
        assert_eq!(sum.result(), Some(Value::Int64(3)));
    }

    #[test]
    #[should_panic(expected = "SumI64: expected Int64 column")]
    fn sum_i64_update_panics_on_wrong_column_type() {
        let mut sum = SumI64::default();
        sum.update(&Column::Float64(vec![1.0]));
    }

    #[test]
    #[should_panic(expected = "sum overflows i64")]
    fn sum_i64_update_panics_on_i64_overflow() {
        let mut sum = SumI64::default();
        sum.update(&Column::Int64(vec![i64::MAX, 1]));
    }

    #[test]
    fn sum_i64_result_is_idempotent_without_update() {
        let mut sum = SumI64::default();
        sum.update(&Column::Int64(vec![1, 2, 3]));
        assert_eq!(sum.result(), Some(Value::Int64(6)));
        assert_eq!(sum.result(), Some(Value::Int64(6)));
    }

    #[test]
    fn sum_i64_works_through_aggregate_trait_object() {
        let mut agg: Box<dyn Aggregate> = Box::new(SumI64::default());
        agg.update(&Column::Int64(vec![1, 2, 3]));
        agg.update(&Column::Int64(vec![4]));
        assert_eq!(agg.result(), Some(Value::Int64(10)));
    }
}
