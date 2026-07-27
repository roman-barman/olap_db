#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StringColumn {
    data: Vec<u8>,
    offsets: Vec<u32>,
}

impl StringColumn {
    pub(crate) fn new() -> Self {
        Self {
            data: Vec::new(),
            offsets: vec![0],
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.offsets.len() - 1
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub(crate) fn push(&mut self, s: &str) {
        self.data.extend_from_slice(s.as_bytes());
        let end = u32::try_from(self.data.len())
            .expect("string column data exceeds u32 limit (4 GiB per column)");
        self.offsets.push(end);
    }

    pub(crate) fn get(&self, i: usize) -> &str {
        let start = self.offsets[i] as usize;
        let end = self.offsets[i + 1] as usize;
        std::str::from_utf8(&self.data[start..end]).expect("invalid UTF-8 sequence")
    }

    pub(crate) fn filter(&self, mask: &[bool]) -> StringColumn {
        assert_eq!(
            mask.len(),
            self.len(),
            "Mask length does not match number of rows"
        );

        let num_rows = mask.iter().filter(|&&m| m).count();

        if num_rows == 0 {
            return StringColumn::new();
        }

        if num_rows == self.len() {
            return self.clone();
        }

        let mut data_size = 0;
        for (i, &m) in mask.iter().enumerate() {
            if m {
                let (start, end) = (self.offsets[i] as usize, self.offsets[i + 1] as usize);
                data_size += end - start;
            }
        }

        let mut result = StringColumn {
            data: Vec::with_capacity(data_size),
            offsets: Vec::with_capacity(num_rows + 1),
        };
        result.offsets.push(0);

        for (i, &m) in mask.iter().enumerate() {
            if m {
                let (start, end) = (self.offsets[i] as usize, self.offsets[i + 1] as usize);
                result.data.extend_from_slice(&self.data[start..end]);
                result.offsets.push(result.data.len() as u32);
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_is_empty() {
        let col = StringColumn::new();
        assert_eq!(col.len(), 0);
        assert!(col.is_empty());
    }

    #[test]
    fn push_increases_len() {
        let mut col = StringColumn::new();
        col.push("a");
        col.push("b");
        col.push("c");
        assert_eq!(col.len(), 3);
        assert!(!col.is_empty());
    }

    #[test]
    fn push_then_get_returns_values_in_order() {
        let mut col = StringColumn::new();
        col.push("hello");
        col.push("world");
        col.push("foo");
        assert_eq!(col.get(0), "hello");
        assert_eq!(col.get(1), "world");
        assert_eq!(col.get(2), "foo");
    }

    #[test]
    fn push_empty_string() {
        let mut col = StringColumn::new();
        col.push("before");
        col.push("");
        col.push("after");
        assert_eq!(col.get(0), "before");
        assert_eq!(col.get(1), "");
        assert_eq!(col.get(2), "after");
    }

    #[test]
    fn push_multibyte_utf8_string() {
        let mut col = StringColumn::new();
        col.push("café");
        col.push("日本語");
        col.push("plain");
        assert_eq!(col.get(0), "café");
        assert_eq!(col.get(1), "日本語");
        assert_eq!(col.get(2), "plain");
    }

    #[test]
    fn get_first_and_last_index() {
        let mut col = StringColumn::new();
        col.push("first");
        col.push("middle");
        col.push("last");
        assert_eq!(col.get(0), "first");
        assert_eq!(col.get(col.len() - 1), "last");
    }

    #[test]
    #[should_panic(expected = "Mask length does not match number of rows")]
    fn filter_mask_length_mismatch_panics() {
        let mut col = StringColumn::new();
        col.push("a");
        col.push("b");
        col.filter(&[true]);
    }

    #[test]
    fn filter_all_true_returns_equal_clone() {
        let mut col = StringColumn::new();
        col.push("a");
        col.push("b");
        col.push("c");
        let original = col.clone();

        let result = col.filter(&[true, true, true]);

        assert_eq!(result, original);
        assert_eq!(col, original);
    }

    #[test]
    fn filter_all_false_returns_empty() {
        let mut col = StringColumn::new();
        col.push("a");
        col.push("b");
        col.push("c");

        let result = col.filter(&[false, false, false]);

        assert_eq!(result, StringColumn::new());
    }

    #[test]
    fn filter_mixed_mask_keeps_selected_rows_in_order() {
        let mut col = StringColumn::new();
        col.push("a");
        col.push("b");
        col.push("c");
        col.push("d");

        let result = col.filter(&[true, false, true, false]);

        assert_eq!(result.len(), 2);
        assert_eq!(result.get(0), "a");
        assert_eq!(result.get(1), "c");
    }

    #[test]
    fn filter_single_row_true_and_false() {
        let mut true_col = StringColumn::new();
        true_col.push("only");
        let true_result = true_col.filter(&[true]);
        assert_eq!(true_result.len(), 1);
        assert_eq!(true_result.get(0), "only");

        let mut false_col = StringColumn::new();
        false_col.push("only");
        let false_result = false_col.filter(&[false]);
        assert_eq!(false_result.len(), 0);
    }

    #[test]
    fn filter_does_not_mutate_original() {
        let mut col = StringColumn::new();
        col.push("a");
        col.push("b");
        col.push("c");
        let original = col.clone();

        let _ = col.filter(&[true, false, true]);

        assert_eq!(col, original);
        assert_eq!(col.len(), 3);
    }

    #[test]
    fn filter_empty_column_with_empty_mask() {
        let col = StringColumn::new();
        let result = col.filter(&[]);
        assert_eq!(result, StringColumn::new());
    }

    #[test]
    fn equal_columns_built_the_same_way_are_eq() {
        let mut a = StringColumn::new();
        a.push("x");
        a.push("y");

        let mut b = StringColumn::new();
        b.push("x");
        b.push("y");

        assert_eq!(a, b);
    }

    #[test]
    fn columns_with_different_contents_are_not_eq() {
        let mut a = StringColumn::new();
        a.push("x");
        a.push("y");

        let mut b = StringColumn::new();
        b.push("x");
        b.push("z");

        let mut c = StringColumn::new();
        c.push("x");

        assert_ne!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn clone_is_independent() {
        let mut original = StringColumn::new();
        original.push("a");
        original.push("b");

        let mut cloned = original.clone();
        cloned.push("c");

        assert_eq!(original.len(), 2);
        assert_eq!(cloned.len(), 3);
        assert_eq!(original.get(0), "a");
        assert_eq!(original.get(1), "b");
    }
}
