use crate::DataType;

#[derive(Debug, Clone, PartialEq)]
pub enum Column {
    Int64(Vec<i64>),
    Float64(Vec<f64>),
    String(Vec<String>),
}

impl Column {
    pub fn new(dt: DataType) -> Self {
        Self::with_capacity(dt, 0)
    }

    pub fn with_capacity(dt: DataType, cap: usize) -> Self {
        match dt {
            DataType::Int64 => Column::Int64(Vec::with_capacity(cap)),
            DataType::Float64 => Column::Float64(Vec::with_capacity(cap)),
            DataType::String => Column::String(Vec::with_capacity(cap)),
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Column::Int64(v) => v.len(),
            Column::Float64(v) => v.len(),
            Column::String(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub(crate) fn data_type(&self) -> DataType {
        match self {
            Column::Int64(_) => DataType::Int64,
            Column::Float64(_) => DataType::Float64,
            Column::String(_) => DataType::String,
        }
    }

    pub fn push_i64(&mut self, value: i64) {
        match self {
            Column::Int64(v) => v.push(value),
            _ => panic!("push_i64 on {:?} column", self.data_type()),
        }
    }
    pub fn push_f64(&mut self, value: f64) {
        match self {
            Column::Float64(v) => v.push(value),
            _ => panic!("push_f64 on {:?} column", self.data_type()),
        }
    }
    pub fn push_str(&mut self, value: &str) {
        match self {
            Column::String(v) => v.push(value.to_string()),
            _ => panic!("push_str on {:?} column", self.data_type()),
        }
    }

    pub(crate) fn filter(&self, mask: &[bool]) -> Column {
        assert_eq!(
            mask.len(),
            self.len(),
            "filter: mask length != column length"
        );
        let cap = mask.iter().filter(|&&m| m).count();

        if cap == 0 {
            return Column::new(self.data_type());
        }
        if cap == mask.len() {
            return self.clone();
        }

        match self {
            Column::Int64(v) => Column::Int64(filter_slice(v, mask, cap)),
            Column::Float64(v) => Column::Float64(filter_slice(v, mask, cap)),
            Column::String(v) => Column::String(filter_slice(v, mask, cap)),
        }
    }
}

fn filter_slice<T: Clone>(v: &[T], mask: &[bool], cap: usize) -> Vec<T> {
    let mut out = Vec::with_capacity(cap);
    out.extend(
        v.iter()
            .zip(mask)
            .filter_map(|(x, &m)| m.then(|| x.clone())),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_and_with_capacity() {
        for dt in [DataType::Int64, DataType::Float64, DataType::String] {
            let c = Column::new(dt.clone());
            assert_eq!(c.len(), 0);
            assert!(c.is_empty());
            assert_eq!(c.data_type(), dt.clone());

            let c = Column::with_capacity(dt.clone(), 8);
            assert_eq!(c.len(), 0);
            assert!(c.is_empty());
            assert_eq!(c.data_type(), dt.clone());

            let c = Column::with_capacity(dt.clone(), 0);
            assert_eq!(c.len(), 0);
            assert!(c.is_empty());
        }
    }

    #[test]
    fn data_type_per_variant() {
        assert_eq!(Column::Int64(vec![]).data_type(), DataType::Int64);
        assert_eq!(Column::Float64(vec![]).data_type(), DataType::Float64);
        assert_eq!(Column::String(vec![]).data_type(), DataType::String);

        let mut c = Column::new(DataType::Int64);
        c.push_i64(1);
        assert_eq!(c.data_type(), DataType::Int64);
    }

    #[test]
    fn len_and_is_empty() {
        let mut c = Column::new(DataType::Int64);
        assert_eq!(c.len(), 0);
        assert!(c.is_empty());

        c.push_i64(1);
        c.push_i64(2);
        assert_eq!(c.len(), 2);
        assert!(!c.is_empty());
    }

    #[test]
    fn push_i64_appends_in_order() {
        let mut c = Column::new(DataType::Int64);
        c.push_i64(1);
        c.push_i64(2);
        c.push_i64(3);
        assert_eq!(c, Column::Int64(vec![1, 2, 3]));
    }

    #[test]
    fn push_f64_appends_in_order() {
        let mut c = Column::new(DataType::Float64);
        c.push_f64(1.5);
        c.push_f64(2.5);
        assert_eq!(c, Column::Float64(vec![1.5, 2.5]));
    }

    #[test]
    fn push_str_appends_owned_strings_in_order() {
        let mut c = Column::new(DataType::String);
        let borrowed = String::from("hello");
        c.push_str(&borrowed);
        c.push_str("");
        c.push_str("world");
        assert_eq!(
            c,
            Column::String(vec![
                "hello".to_string(),
                "".to_string(),
                "world".to_string()
            ])
        );
    }

    #[test]
    #[should_panic(expected = "push_i64 on Float64 column")]
    fn push_i64_on_float64_panics() {
        Column::new(DataType::Float64).push_i64(1);
    }

    #[test]
    #[should_panic(expected = "push_i64 on String column")]
    fn push_i64_on_string_panics() {
        Column::new(DataType::String).push_i64(1);
    }

    #[test]
    #[should_panic(expected = "push_f64 on Int64 column")]
    fn push_f64_on_int64_panics() {
        Column::new(DataType::Int64).push_f64(1.0);
    }

    #[test]
    #[should_panic(expected = "push_f64 on String column")]
    fn push_f64_on_string_panics() {
        Column::new(DataType::String).push_f64(1.0);
    }

    #[test]
    #[should_panic(expected = "push_str on Int64 column")]
    fn push_str_on_int64_panics() {
        Column::new(DataType::Int64).push_str("x");
    }

    #[test]
    #[should_panic(expected = "push_str on Float64 column")]
    fn push_str_on_float64_panics() {
        Column::new(DataType::Float64).push_str("x");
    }

    #[test]
    fn filter_keeps_masked_in_elements_in_order_int64() {
        let c = Column::Int64(vec![1, 2, 3, 4]);
        let mask = [true, false, true, false];
        assert_eq!(c.filter(&mask), Column::Int64(vec![1, 3]));
    }

    #[test]
    fn filter_keeps_masked_in_elements_in_order_float64() {
        let c = Column::Float64(vec![1.0, 2.0, 3.0, 4.0]);
        let mask = [false, true, false, true];
        assert_eq!(c.filter(&mask), Column::Float64(vec![2.0, 4.0]));
    }

    #[test]
    fn filter_keeps_masked_in_elements_in_order_string() {
        let c = Column::String(vec!["a".into(), "b".into(), "c".into()]);
        let mask = [true, false, true];
        assert_eq!(
            c.filter(&mask),
            Column::String(vec!["a".into(), "c".into()])
        );
    }

    #[test]
    fn filter_all_true_returns_equal_column_and_leaves_original_untouched() {
        let c = Column::Int64(vec![1, 2, 3]);
        let mask = [true, true, true];
        let filtered = c.filter(&mask);
        assert_eq!(filtered, c);
    }

    #[test]
    fn filter_all_false_returns_empty_column_of_same_type() {
        let c = Column::Int64(vec![1, 2, 3]);
        let mask = [false, false, false];
        let filtered = c.filter(&mask);
        assert!(filtered.is_empty());
        assert_eq!(filtered.data_type(), DataType::Int64);
    }

    #[test]
    fn filter_empty_column_with_empty_mask() {
        let c = Column::new(DataType::String);
        let filtered = c.filter(&[]);
        assert!(filtered.is_empty());
        assert_eq!(filtered.data_type(), DataType::String);
    }

    #[test]
    fn filter_single_element_true_and_false() {
        let c = Column::Int64(vec![42]);
        assert_eq!(c.filter(&[true]), Column::Int64(vec![42]));
        assert_eq!(c.filter(&[false]), Column::Int64(vec![]));
    }

    #[test]
    #[should_panic(expected = "filter: mask length != column length")]
    fn filter_mask_shorter_than_column_panics() {
        Column::Int64(vec![1, 2, 3]).filter(&[true, false]);
    }

    #[test]
    #[should_panic(expected = "filter: mask length != column length")]
    fn filter_mask_longer_than_column_panics() {
        Column::Int64(vec![1, 2]).filter(&[true, false, true]);
    }

    #[test]
    fn filter_does_not_mutate_original() {
        let c = Column::Int64(vec![1, 2, 3, 4]);
        let _ = c.filter(&[true, false, true, false]);
        assert_eq!(c, Column::Int64(vec![1, 2, 3, 4]));
    }

    #[test]
    fn equality_same_variant_same_contents() {
        assert_eq!(Column::Int64(vec![1, 2]), Column::Int64(vec![1, 2]));
    }

    #[test]
    fn equality_same_variant_different_contents() {
        assert_ne!(Column::Int64(vec![1, 2]), Column::Int64(vec![1, 3]));
    }

    #[test]
    fn equality_different_variants_never_equal() {
        assert_ne!(Column::Int64(vec![1]), Column::Float64(vec![1.0]));
    }

    #[test]
    fn clone_produces_independent_equal_copy() {
        let original = Column::Int64(vec![1, 2, 3]);
        let mut cloned = original.clone();
        assert_eq!(original, cloned);

        cloned.push_i64(4);
        assert_ne!(original, cloned);
        assert_eq!(original, Column::Int64(vec![1, 2, 3]));
    }
}
