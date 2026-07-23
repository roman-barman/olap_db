use crate::aggregate::Aggregate;
use crate::column::Column;
use crate::value::Value;

#[derive(Debug, Clone, Default)]
pub(super) struct MinF64 {
    min: Option<f64>,
}

impl Aggregate for MinF64 {
    fn update(&mut self, col: &Column) {
        match col {
            Column::Float64(v) => {
                let block_min = v.iter().copied().min_by(|a, b| a.total_cmp(b));
                self.min = match (self.min, block_min) {
                    (None, m) => m,
                    (m, None) => m,
                    (Some(a), Some(b)) => Some(if a.total_cmp(&b).is_le() { a } else { b }),
                };
            }
            other => panic!(
                "MinF64: expected Float64 column, got {:?}",
                other.data_type()
            ),
        }
    }

    fn result(&self) -> Option<Value> {
        self.min.map(Value::Float64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DataType;

    #[test]
    fn min_f64_default_result_is_none() {
        let min = MinF64::default();
        assert_eq!(min.result(), None);
    }

    #[test]
    fn min_f64_update_single_column_computes_min() {
        let mut min = MinF64::default();
        min.update(&Column::Float64(vec![3.0, 1.5, 2.0]));
        assert_eq!(min.result(), Some(Value::Float64(1.5)));
    }

    #[test]
    fn min_f64_update_accumulates_across_multiple_calls() {
        let mut min = MinF64::default();
        min.update(&Column::Float64(vec![5.0, 3.0]));
        min.update(&Column::Float64(vec![4.0, 1.5, 9.0]));
        assert_eq!(min.result(), Some(Value::Float64(1.5)));
    }

    #[test]
    fn min_f64_update_with_negative_numbers() {
        let mut min = MinF64::default();
        min.update(&Column::Float64(vec![10.0, -3.5, -2.0]));
        assert_eq!(min.result(), Some(Value::Float64(-3.5)));
    }

    #[test]
    fn min_f64_update_with_empty_column_is_noop() {
        let mut min = MinF64::default();
        min.update(&Column::Float64(vec![5.0, 2.0]));
        min.update(&Column::new(DataType::Float64));
        assert_eq!(min.result(), Some(Value::Float64(2.0)));
    }

    #[test]
    fn min_f64_update_empty_column_before_data_is_none_then_updates() {
        let mut min = MinF64::default();
        min.update(&Column::new(DataType::Float64));
        assert_eq!(min.result(), None);
        min.update(&Column::Float64(vec![7.0, 2.0, 9.0]));
        assert_eq!(min.result(), Some(Value::Float64(2.0)));
    }

    #[test]
    #[should_panic(expected = "MinF64: expected Float64 column")]
    fn min_f64_update_panics_on_wrong_column_type() {
        let mut min = MinF64::default();
        min.update(&Column::Int64(vec![1]));
    }

    #[test]
    fn min_f64_result_is_idempotent_without_update() {
        let mut min = MinF64::default();
        min.update(&Column::Float64(vec![3.0, 1.5, 2.0]));
        assert_eq!(min.result(), Some(Value::Float64(1.5)));
        assert_eq!(min.result(), Some(Value::Float64(1.5)));
    }

    #[test]
    fn min_f64_works_through_aggregate_trait_object() {
        let mut agg: Box<dyn Aggregate> = Box::new(MinF64::default());
        agg.update(&Column::Float64(vec![5.0, 3.0]));
        agg.update(&Column::Float64(vec![1.5]));
        assert_eq!(agg.result(), Some(Value::Float64(1.5)));
    }

    #[test]
    fn min_f64_update_with_nan_is_ignored_by_total_cmp() {
        // `total_cmp` orders NaN as greater than all other values (positive NaN
        // sorts above +inf), so a NaN in the input never becomes the min as
        // long as at least one non-NaN value is present.
        let mut min = MinF64::default();
        min.update(&Column::Float64(vec![f64::NAN, 5.0, 1.0]));
        assert_eq!(min.result(), Some(Value::Float64(1.0)));
    }

    #[test]
    fn min_f64_update_with_infinities() {
        let mut min = MinF64::default();
        min.update(&Column::Float64(vec![
            f64::INFINITY,
            f64::NEG_INFINITY,
            0.0,
        ]));
        assert_eq!(min.result(), Some(Value::Float64(f64::NEG_INFINITY)));
    }

    #[test]
    fn min_f64_update_distinguishes_negative_zero() {
        // total_cmp orders -0.0 < 0.0, unlike `==`, so assert on sign bit
        // rather than PartialEq (under which -0.0 == 0.0).
        let mut min = MinF64::default();
        min.update(&Column::Float64(vec![0.0, -0.0]));
        match min.result() {
            Some(Value::Float64(v)) => assert!(v.is_sign_negative()),
            other => panic!("expected Some(Value::Float64(_)), got {:?}", other),
        }
    }
}
