use crate::aggregate::Aggregate;
use crate::column::Column;
use crate::value::Value;

#[derive(Debug, Clone, Default)]
pub(super) struct SumF64 {
    sum: f64,
}

impl Aggregate for SumF64 {
    fn update(&mut self, col: &Column) {
        match col {
            Column::Float64(v) => {
                self.sum += v.iter().sum::<f64>();
            }
            other => panic!(
                "SumF64: expected Float64 column, got {:?}",
                other.data_type()
            ),
        }
    }

    fn result(&self) -> Option<Value> {
        Some(Value::Float64(self.sum))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DataType;

    #[test]
    fn sum_f64_default_result_is_zero() {
        let sum = SumF64::default();
        assert_eq!(sum.result(), Some(Value::Float64(0.0)));
    }

    #[test]
    fn sum_f64_update_float64_column_computes_sum() {
        let mut sum = SumF64::default();
        sum.update(&Column::Float64(vec![1.5, 2.5, 3.0]));
        assert_eq!(sum.result(), Some(Value::Float64(7.0)));
    }

    #[test]
    fn sum_f64_update_accumulates_across_multiple_calls() {
        let mut sum = SumF64::default();
        sum.update(&Column::Float64(vec![1.5, 2.5]));
        sum.update(&Column::Float64(vec![1.0]));
        assert_eq!(sum.result(), Some(Value::Float64(5.0)));
    }

    #[test]
    fn sum_f64_update_with_negative_numbers() {
        let mut sum = SumF64::default();
        sum.update(&Column::Float64(vec![10.0, -3.5, -2.5]));
        assert_eq!(sum.result(), Some(Value::Float64(4.0)));
    }

    #[test]
    fn sum_f64_update_with_empty_column_is_noop() {
        let mut sum = SumF64::default();
        sum.update(&Column::Float64(vec![1.5, 2.5]));
        sum.update(&Column::new(DataType::Float64));
        assert_eq!(sum.result(), Some(Value::Float64(4.0)));
    }

    #[test]
    #[should_panic(expected = "SumF64: expected Float64 column")]
    fn sum_f64_update_panics_on_wrong_column_type() {
        let mut sum = SumF64::default();
        sum.update(&Column::Int64(vec![1]));
    }

    #[test]
    fn sum_f64_update_does_not_panic_on_overflow_saturates_to_infinity() {
        let mut sum = SumF64::default();
        sum.update(&Column::Float64(vec![f64::MAX, f64::MAX]));
        match sum.result() {
            Some(Value::Float64(v)) => assert!(v.is_infinite()),
            other => panic!("expected Some(Value::Float64(_)), got {:?}", other),
        }
    }

    #[test]
    fn sum_f64_result_is_idempotent_without_update() {
        let mut sum = SumF64::default();
        sum.update(&Column::Float64(vec![1.5, 2.5]));
        assert_eq!(sum.result(), Some(Value::Float64(4.0)));
        assert_eq!(sum.result(), Some(Value::Float64(4.0)));
    }

    #[test]
    fn sum_f64_works_through_aggregate_trait_object() {
        let mut agg: Box<dyn Aggregate> = Box::new(SumF64::default());
        agg.update(&Column::Float64(vec![1.5, 2.5]));
        agg.update(&Column::Float64(vec![1.0]));
        assert_eq!(agg.result(), Some(Value::Float64(5.0)));
    }
}
