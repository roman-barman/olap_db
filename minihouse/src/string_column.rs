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

    pub(crate) fn with_capacity(capacity: usize, bytes: usize) -> Self {
        let mut offset = Vec::with_capacity(capacity + 1);
        offset.push(0);

        Self {
            data: Vec::with_capacity(bytes),
            offsets: offset,
        }
    }

    pub(crate) fn try_from_parts(data: Vec<u8>, offsets: Vec<u32>) -> Result<Self, String> {
        if offsets.is_empty() {
            return Err("offsets must not be empty".into());
        }
        if offsets[0] != 0 {
            return Err(format!("offsets must start with 0, got {}", offsets[0]));
        }
        for w in offsets.windows(2) {
            if w[0] > w[1] {
                return Err(format!("offsets not monotonic: {} > {}", w[0], w[1]));
            }
        }
        let last = *offsets.last().unwrap() as usize;
        if last != data.len() {
            return Err(format!("last offset {last} != data length {}", data.len()));
        }
        Ok(Self { data, offsets })
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

    pub(crate) fn bytes_at(&self, i: usize) -> &[u8] {
        let start = self.offsets[i] as usize;
        let end = self.offsets[i + 1] as usize;
        &self.data[start..end]
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

    pub(crate) fn data_len(&self) -> usize {
        self.data.len()
    }

    pub(crate) fn data(&self) -> &[u8] {
        &self.data
    }

    pub(crate) fn offsets(&self) -> &[u32] {
        &self.offsets
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

    #[test]
    fn with_capacity_reserves_capacity() {
        let col = StringColumn::with_capacity(4, 32);
        assert!(col.offsets.capacity() >= 5);
        assert!(col.data.capacity() >= 32);
        assert_eq!(col.len(), 0);
        assert!(col.is_empty());
    }

    #[test]
    fn with_capacity_zero_then_push_behaves_like_new() {
        let mut with_cap = StringColumn::with_capacity(0, 0);
        with_cap.push("a");
        with_cap.push("b");

        let mut new_col = StringColumn::new();
        new_col.push("a");
        new_col.push("b");

        assert_eq!(with_cap, new_col);
    }

    #[test]
    fn bytes_at_returns_same_bytes_as_get() {
        let mut col = StringColumn::new();
        col.push("hello");
        col.push("café");
        col.push("日本語");

        for i in 0..col.len() {
            assert_eq!(col.bytes_at(i), col.get(i).as_bytes());
        }
    }

    #[test]
    fn bytes_at_empty_string() {
        let mut col = StringColumn::new();
        col.push("before");
        col.push("");
        col.push("after");

        assert_eq!(col.bytes_at(1), b"".as_slice());
    }

    #[test]
    fn bytes_at_first_and_last_index() {
        let mut col = StringColumn::new();
        col.push("first");
        col.push("middle");
        col.push("last");

        assert_eq!(col.bytes_at(0), b"first".as_slice());
        assert_eq!(col.bytes_at(col.len() - 1), b"last".as_slice());
    }

    #[test]
    fn bytes_at_after_filter() {
        let mut col = StringColumn::new();
        col.push("a");
        col.push("b");
        col.push("c");
        col.push("d");

        let result = col.filter(&[true, false, true, false]);

        assert_eq!(result.bytes_at(0), b"a".as_slice());
        assert_eq!(result.bytes_at(1), b"c".as_slice());
    }

    #[test]
    fn try_from_parts_accepts_empty_column() {
        let col = StringColumn::try_from_parts(Vec::new(), vec![0]).unwrap();

        assert_eq!(col.len(), 0);
        assert!(col.is_empty());
        assert_eq!(col, StringColumn::new());
    }

    #[test]
    fn try_from_parts_single_value() {
        let col = StringColumn::try_from_parts(b"abc".to_vec(), vec![0, 3]).unwrap();

        assert_eq!(col.len(), 1);
        assert_eq!(col.get(0), "abc");
    }

    #[test]
    fn try_from_parts_multiple_values_in_order() {
        let col = StringColumn::try_from_parts(b"abcdef".to_vec(), vec![0, 1, 3, 6]).unwrap();

        assert_eq!(col.len(), 3);
        assert_eq!(col.get(0), "a");
        assert_eq!(col.get(1), "bc");
        assert_eq!(col.get(2), "def");
    }

    #[test]
    fn try_from_parts_accepts_repeated_offsets_as_empty_strings() {
        let col = StringColumn::try_from_parts(b"ab".to_vec(), vec![0, 1, 1, 1, 2]).unwrap();

        assert_eq!(col.len(), 4);
        assert_eq!(col.get(0), "a");
        assert_eq!(col.get(1), "");
        assert_eq!(col.get(2), "");
        assert_eq!(col.get(3), "b");
    }

    #[test]
    fn try_from_parts_accepts_all_empty_strings_with_empty_data() {
        let col = StringColumn::try_from_parts(Vec::new(), vec![0, 0, 0]).unwrap();

        assert_eq!(col.len(), 2);
        assert_eq!(col.get(0), "");
        assert_eq!(col.get(1), "");
    }

    #[test]
    fn try_from_parts_preserves_multibyte_utf8() {
        let col =
            StringColumn::try_from_parts("café日本".as_bytes().to_vec(), vec![0, 5, 11]).unwrap();

        assert_eq!(col.len(), 2);
        assert_eq!(col.get(0), "café");
        assert_eq!(col.get(1), "日本");
    }

    #[test]
    fn try_from_parts_round_trips_pushed_column() {
        let mut col = StringColumn::new();
        col.push("hello");
        col.push("");
        col.push("日本語");

        let rebuilt =
            StringColumn::try_from_parts(col.data().to_vec(), col.offsets().to_vec()).unwrap();

        assert_eq!(rebuilt, col);
    }

    #[test]
    fn try_from_parts_round_trips_empty_column() {
        let col = StringColumn::new();

        let rebuilt =
            StringColumn::try_from_parts(col.data().to_vec(), col.offsets().to_vec()).unwrap();

        assert_eq!(rebuilt, col);
    }

    #[test]
    fn try_from_parts_column_supports_filter_and_bytes_at() {
        let col = StringColumn::try_from_parts(b"abcd".to_vec(), vec![0, 1, 2, 3, 4]).unwrap();

        let result = col.filter(&[true, false, true, false]);

        assert_eq!(result.len(), 2);
        assert_eq!(result.get(0), "a");
        assert_eq!(result.get(1), "c");
        assert_eq!(result.bytes_at(0), b"a".as_slice());
        assert_eq!(result.bytes_at(1), b"c".as_slice());
    }

    #[test]
    fn try_from_parts_column_can_be_pushed_to() {
        let mut col = StringColumn::try_from_parts(b"abc".to_vec(), vec![0, 1, 3]).unwrap();

        col.push("new");

        assert_eq!(col.len(), 3);
        assert_eq!(col.get(0), "a");
        assert_eq!(col.get(1), "bc");
        assert_eq!(col.get(2), "new");
    }

    #[test]
    fn try_from_parts_rejects_empty_offsets() {
        let err = StringColumn::try_from_parts(Vec::new(), Vec::new()).unwrap_err();
        assert!(err.contains("offsets must not be empty"));
    }

    #[test]
    fn try_from_parts_rejects_offsets_not_starting_at_zero() {
        let err = StringColumn::try_from_parts(b"abc".to_vec(), vec![1, 3]).unwrap_err();
        assert!(err.contains("offsets must start with 0, got 1"));
    }

    #[test]
    fn try_from_parts_rejects_decreasing_offsets() {
        let err = StringColumn::try_from_parts(b"abcd".to_vec(), vec![0, 3, 2, 4]).unwrap_err();
        assert!(err.contains("offsets not monotonic: 3 > 2"));
    }

    #[test]
    fn try_from_parts_rejects_last_offset_below_data_len() {
        let err = StringColumn::try_from_parts(b"abcd".to_vec(), vec![0, 2]).unwrap_err();
        assert!(err.contains("last offset 2 != data length 4"));
    }

    #[test]
    fn try_from_parts_rejects_last_offset_above_data_len() {
        let err = StringColumn::try_from_parts(b"ab".to_vec(), vec![0, 5]).unwrap_err();
        assert!(err.contains("last offset 5 != data length 2"));
    }

    #[test]
    fn try_from_parts_rejects_nonempty_data_with_only_sentinel_offset() {
        let err = StringColumn::try_from_parts(b"abc".to_vec(), vec![0]).unwrap_err();
        assert!(err.contains("last offset 0 != data length 3"));
    }

    #[test]
    fn try_from_parts_reports_empty_offsets_before_length_mismatch() {
        let err = StringColumn::try_from_parts(b"abc".to_vec(), Vec::new()).unwrap_err();
        assert!(err.contains("offsets must not be empty"));
    }

    #[test]
    fn try_from_parts_reports_start_error_before_monotonic_error() {
        let err = StringColumn::try_from_parts(b"abc".to_vec(), vec![5, 3]).unwrap_err();
        assert!(err.contains("offsets must start with 0, got 5"));
    }

    #[test]
    fn try_from_parts_reports_monotonic_error_before_length_error() {
        let err = StringColumn::try_from_parts(b"abc".to_vec(), vec![0, 10, 3]).unwrap_err();
        assert!(err.contains("offsets not monotonic: 10 > 3"));
    }
}
