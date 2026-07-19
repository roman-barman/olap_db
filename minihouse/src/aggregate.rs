use crate::column::Column;
use crate::value::Value;

pub trait Aggregate {
    fn update(&mut self, col: &Column);
    fn result(&self) -> Option<Value>;
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Count {
    count: usize,
}

impl Aggregate for Count {
    fn update(&mut self, col: &Column) {
        self.count += col.len()
    }

    fn result(&self) -> Option<Value> {
        Some(Value::Int64(
            i64::try_from(self.count).expect("count overflows i64"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::DataType;

    #[test]
    fn count_default_result_is_zero() {
        let count = Count::default();
        assert_eq!(count.result(), Some(Value::Int64(0)));
    }

    #[test]
    fn count_update_int64_column_adds_len() {
        let mut count = Count::default();
        count.update(&Column::Int64(vec![1, 2, 3]));
        assert_eq!(count.result(), Some(Value::Int64(3)));
    }

    #[test]
    fn count_update_float64_column_adds_len() {
        let mut count = Count::default();
        count.update(&Column::Float64(vec![1.0, 2.0]));
        assert_eq!(count.result(), Some(Value::Int64(2)));
    }

    #[test]
    fn count_update_string_column_adds_len() {
        let mut count = Count::default();
        count.update(&Column::String(vec![
            "a".into(),
            "b".into(),
            "c".into(),
            "d".into(),
        ]));
        assert_eq!(count.result(), Some(Value::Int64(4)));
    }

    #[test]
    fn count_update_accumulates_across_multiple_calls() {
        let mut count = Count::default();
        count.update(&Column::Int64(vec![1, 2, 3]));
        count.update(&Column::Int64(vec![4, 5]));
        assert_eq!(count.result(), Some(Value::Int64(5)));
    }

    #[test]
    fn count_update_with_empty_column_is_noop() {
        let mut count = Count::default();
        count.update(&Column::Int64(vec![1, 2]));
        count.update(&Column::new(DataType::Int64));
        assert_eq!(count.result(), Some(Value::Int64(2)));
    }

    #[test]
    fn count_result_is_idempotent_without_update() {
        let mut count = Count::default();
        count.update(&Column::Int64(vec![1, 2, 3]));
        assert_eq!(count.result(), Some(Value::Int64(3)));
        assert_eq!(count.result(), Some(Value::Int64(3)));
    }

    #[test]
    #[should_panic(expected = "count overflows i64")]
    fn count_result_panics_on_usize_overflow() {
        let count = Count { count: usize::MAX };
        count.result();
    }

    #[test]
    fn count_works_through_aggregate_trait_object() {
        let mut agg: Box<dyn Aggregate> = Box::new(Count::default());
        agg.update(&Column::Int64(vec![1, 2, 3]));
        agg.update(&Column::Float64(vec![1.0]));
        assert_eq!(agg.result(), Some(Value::Int64(4)));
    }
}
