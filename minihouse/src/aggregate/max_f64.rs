use crate::aggregate::Aggregate;
use crate::column::Column;
use crate::value::Value;

#[derive(Debug, Clone, Default)]
pub(super) struct MaxF64 {
    max: Option<f64>,
}

impl Aggregate for MaxF64 {
    fn update(&mut self, col: &Column) {
        match col {
            Column::Float64(v) => {
                let block_max = v.iter().copied().max_by(|a, b| a.total_cmp(b));
                self.max = match (self.max, block_max) {
                    (None, m) => m,
                    (m, None) => m,
                    (Some(a), Some(b)) => Some(if a.total_cmp(&b).is_ge() { a } else { b }),
                };
            }
            other => panic!(
                "MaxF64: expected Float64 column, got {:?}",
                other.data_type()
            ),
        }
    }

    fn result(&self) -> Option<Value> {
        self.max.map(Value::Float64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DataType;

    #[test]
    fn max_f64_default_result_is_none() {
        let max = MaxF64::default();
        assert_eq!(max.result(), None);
    }

    #[test]
    fn max_f64_update_single_column_computes_max() {
        let mut max = MaxF64::default();
        max.update(&Column::Float64(vec![3.0, 1.5, 2.0]));
        assert_eq!(max.result(), Some(Value::Float64(3.0)));
    }

    #[test]
    fn max_f64_update_accumulates_across_multiple_calls() {
        let mut max = MaxF64::default();
        max.update(&Column::Float64(vec![5.0, 3.0]));
        max.update(&Column::Float64(vec![4.0, 9.5, 1.0]));
        assert_eq!(max.result(), Some(Value::Float64(9.5)));
    }

    #[test]
    fn max_f64_update_with_negative_numbers() {
        let mut max = MaxF64::default();
        max.update(&Column::Float64(vec![-10.0, -3.5, -2.0]));
        assert_eq!(max.result(), Some(Value::Float64(-2.0)));
    }

    #[test]
    fn max_f64_update_with_empty_column_is_noop() {
        let mut max = MaxF64::default();
        max.update(&Column::Float64(vec![5.0, 2.0]));
        max.update(&Column::new(DataType::Float64));
        assert_eq!(max.result(), Some(Value::Float64(5.0)));
    }

    #[test]
    fn max_f64_update_empty_column_before_data_is_none_then_updates() {
        let mut max = MaxF64::default();
        max.update(&Column::new(DataType::Float64));
        assert_eq!(max.result(), None);
        max.update(&Column::Float64(vec![7.0, 2.0, 9.0]));
        assert_eq!(max.result(), Some(Value::Float64(9.0)));
    }

    #[test]
    #[should_panic(expected = "MaxF64: expected Float64 column")]
    fn max_f64_update_panics_on_wrong_column_type() {
        let mut max = MaxF64::default();
        max.update(&Column::Int64(vec![1]));
    }

    #[test]
    fn max_f64_result_is_idempotent_without_update() {
        let mut max = MaxF64::default();
        max.update(&Column::Float64(vec![3.0, 1.5, 2.0]));
        assert_eq!(max.result(), Some(Value::Float64(3.0)));
        assert_eq!(max.result(), Some(Value::Float64(3.0)));
    }

    #[test]
    fn max_f64_works_through_aggregate_trait_object() {
        let mut agg: Box<dyn Aggregate> = Box::new(MaxF64::default());
        agg.update(&Column::Float64(vec![5.0, 3.0]));
        agg.update(&Column::Float64(vec![9.5]));
        assert_eq!(agg.result(), Some(Value::Float64(9.5)));
    }

    #[test]
    fn max_f64_update_with_nan_total_cmp_ranks_it_highest() {
        // Unlike MinF64, `total_cmp` ranks (positive, quiet) NaN as greater
        // than +inf and every finite value, so a NaN in the input becomes
        // the max — the opposite of IEEE `<`/`>` comparisons, where every
        // NaN comparison is false.
        let mut max = MaxF64::default();
        max.update(&Column::Float64(vec![f64::NAN, 5.0, 1.0]));
        match max.result() {
            Some(Value::Float64(v)) => assert!(v.is_nan()),
            other => panic!("expected Some(Value::Float64(_)), got {:?}", other),
        }
    }

    #[test]
    fn max_f64_update_with_infinities() {
        let mut max = MaxF64::default();
        max.update(&Column::Float64(vec![
            f64::INFINITY,
            f64::NEG_INFINITY,
            0.0,
        ]));
        assert_eq!(max.result(), Some(Value::Float64(f64::INFINITY)));
    }

    #[test]
    fn max_f64_update_distinguishes_negative_zero() {
        let mut max = MaxF64::default();
        max.update(&Column::Float64(vec![0.0, -0.0]));
        match max.result() {
            Some(Value::Float64(v)) => assert!(v.is_sign_positive()),
            other => panic!("expected Some(Value::Float64(_)), got {:?}", other),
        }
    }
}
