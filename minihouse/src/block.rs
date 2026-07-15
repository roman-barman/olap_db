use crate::column::Column;
use crate::helpers;

#[derive(Debug)]
pub struct Block {
    columns: Vec<(String, Column)>,
    num_rows: usize,
}

impl Block {
    pub fn new(columns: Vec<(String, Column)>, num_rows: usize) -> Self {
        helpers::assert_unique_names(&columns);

        for (name, column) in &columns {
            assert_eq!(
                column.len(),
                num_rows,
                "Block::new: column '{name}' len {} != declared num_rows {num_rows}",
                column.len()
            );
        }

        Self { columns, num_rows }
    }

    pub fn num_rows(&self) -> usize {
        self.num_rows
    }

    pub fn column(&self, name: &str) -> Option<&Column> {
        self.columns.iter().find(|(n, _)| n == name).map(|(_, c)| c)
    }

    pub fn filter(&self, mask: &[bool]) -> Block {
        assert_eq!(
            mask.len(),
            self.num_rows,
            "Mask length does not match number of rows"
        );

        let num_rows = mask.iter().filter(|&&m| m).count();
        if num_rows == 0 {
            let empty = self
                .columns
                .iter()
                .map(|(name, col)| (name.clone(), Column::new(col.data_type())))
                .collect();
            return Self::new(empty, 0);
        }

        if num_rows == self.num_rows {
            return Self::new(self.columns.clone(), self.num_rows);
        }

        let mut new_columns = Vec::with_capacity(self.columns.len());
        for (name, column) in &self.columns {
            let new_column = column.filter(mask);
            // TODO: mask counted once per column, hoist
            new_columns.push((name.clone(), new_column));
        }
        Self::new(new_columns, num_rows)
    }

    pub fn columns(&self) -> &[(String, Column)] {
        &self.columns
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_columns() -> Vec<(String, Column)> {
        vec![
            ("id".to_string(), Column::Int64(vec![1, 2, 3])),
            (
                "name".to_string(),
                Column::String(vec!["a".into(), "b".into(), "c".into()]),
            ),
            ("score".to_string(), Column::Float64(vec![1.5, 2.5, 3.5])),
        ]
    }

    #[test]
    fn new_with_matching_lengths_succeeds() {
        let block = Block::new(sample_columns(), 3);
        assert_eq!(block.num_rows(), 3);
        assert_eq!(block.column("id"), Some(&Column::Int64(vec![1, 2, 3])));
        assert_eq!(
            block.column("name"),
            Some(&Column::String(vec!["a".into(), "b".into(), "c".into()]))
        );
        assert_eq!(
            block.column("score"),
            Some(&Column::Float64(vec![1.5, 2.5, 3.5]))
        );
    }

    #[test]
    fn new_with_no_columns_and_zero_rows() {
        let block = Block::new(vec![], 0);
        assert_eq!(block.num_rows(), 0);
    }

    #[test]
    fn new_with_no_columns_accepts_any_num_rows() {
        let block = Block::new(vec![], 5);
        assert_eq!(block.num_rows(), 5);
    }

    #[test]
    #[should_panic(expected = "duplicate column name: a")]
    fn new_duplicate_column_name_panics() {
        Block::new(
            vec![
                ("a".to_string(), Column::Int64(vec![1])),
                ("a".to_string(), Column::Int64(vec![2])),
            ],
            1,
        );
    }

    #[test]
    #[should_panic(expected = "duplicate column name: a")]
    fn new_duplicate_column_name_not_adjacent_panics() {
        Block::new(
            vec![
                ("a".to_string(), Column::Int64(vec![1])),
                ("b".to_string(), Column::Int64(vec![2])),
                ("a".to_string(), Column::Int64(vec![3])),
            ],
            1,
        );
    }

    #[test]
    #[should_panic(expected = "column 'a' len 2 != declared num_rows 3")]
    fn new_column_length_mismatch_panics() {
        Block::new(vec![("a".to_string(), Column::Int64(vec![1, 2]))], 3);
    }

    #[test]
    #[should_panic(expected = "column 'b' len 1 != declared num_rows 2")]
    fn new_column_length_mismatch_on_non_first_column_panics() {
        Block::new(
            vec![
                ("a".to_string(), Column::Int64(vec![1, 2])),
                ("b".to_string(), Column::Int64(vec![1])),
            ],
            2,
        );
    }

    #[test]
    fn num_rows_reflects_constructed_value() {
        let block = Block::new(sample_columns(), 3);
        assert_eq!(block.num_rows(), 3);
    }

    #[test]
    fn column_returns_existing_column_by_name() {
        let block = Block::new(sample_columns(), 3);
        assert_eq!(block.column("id"), Some(&Column::Int64(vec![1, 2, 3])));
        assert_eq!(
            block.column("score"),
            Some(&Column::Float64(vec![1.5, 2.5, 3.5]))
        );
    }

    #[test]
    fn column_returns_none_for_missing_name() {
        let block = Block::new(sample_columns(), 3);
        assert_eq!(block.column("missing"), None);
    }

    #[test]
    fn column_lookup_is_case_sensitive() {
        let block = Block::new(sample_columns(), 3);
        assert_eq!(block.column("Id"), None);
        assert_eq!(block.column("ID"), None);
    }

    #[test]
    #[should_panic(expected = "Mask length does not match number of rows")]
    fn filter_mask_length_mismatch_shorter_panics() {
        Block::new(sample_columns(), 3).filter(&[true, false]);
    }

    #[test]
    #[should_panic(expected = "Mask length does not match number of rows")]
    fn filter_mask_length_mismatch_longer_panics() {
        Block::new(sample_columns(), 3).filter(&[true, false, true, false]);
    }

    #[test]
    fn filter_all_true_returns_same_rows() {
        let block = Block::new(sample_columns(), 3);
        let filtered = block.filter(&[true, true, true]);
        assert_eq!(filtered.num_rows(), 3);
        assert_eq!(filtered.column("id"), Some(&Column::Int64(vec![1, 2, 3])));
        assert_eq!(block.num_rows(), 3);
        assert_eq!(block.column("id"), Some(&Column::Int64(vec![1, 2, 3])));
    }

    #[test]
    fn filter_all_false_returns_empty_block_preserving_schema() {
        let block = Block::new(sample_columns(), 3);
        let filtered = block.filter(&[false, false, false]);
        assert_eq!(filtered.num_rows(), 0);
        assert_eq!(filtered.column("id"), Some(&Column::Int64(vec![])));
        assert_eq!(filtered.column("name"), Some(&Column::String(vec![])));
        assert_eq!(filtered.column("score"), Some(&Column::Float64(vec![])));
    }

    #[test]
    fn filter_mixed_mask_filters_rows_consistently_across_columns() {
        let block = Block::new(sample_columns(), 3);
        let filtered = block.filter(&[true, false, true]);
        assert_eq!(filtered.num_rows(), 2);
        assert_eq!(filtered.column("id"), Some(&Column::Int64(vec![1, 3])));
        assert_eq!(
            filtered.column("name"),
            Some(&Column::String(vec!["a".into(), "c".into()]))
        );
        assert_eq!(
            filtered.column("score"),
            Some(&Column::Float64(vec![1.5, 3.5]))
        );
    }

    #[test]
    fn filter_preserves_column_order_and_names() {
        let block = Block::new(sample_columns(), 3);
        let filtered = block.filter(&[true, false, true]);
        assert!(filtered.column("id").is_some());
        assert!(filtered.column("name").is_some());
        assert!(filtered.column("score").is_some());
    }

    #[test]
    fn filter_does_not_mutate_original_block() {
        let block = Block::new(sample_columns(), 3);
        let _ = block.filter(&[true, false, true]);
        assert_eq!(block.num_rows(), 3);
        assert_eq!(block.column("id"), Some(&Column::Int64(vec![1, 2, 3])));
        assert_eq!(
            block.column("name"),
            Some(&Column::String(vec!["a".into(), "b".into(), "c".into()]))
        );
    }

    #[test]
    fn filter_empty_block_zero_columns() {
        let block = Block::new(vec![], 3);
        let filtered = block.filter(&[true, false, true]);
        assert_eq!(filtered.num_rows(), 2);

        let block = Block::new(vec![], 3);
        let filtered = block.filter(&[false, false, false]);
        assert_eq!(filtered.num_rows(), 0);
    }

    #[test]
    fn filter_single_row_true_and_false() {
        let columns = vec![("id".to_string(), Column::Int64(vec![42]))];
        let block = Block::new(columns.clone(), 1);
        assert_eq!(block.filter(&[true]).num_rows(), 1);

        let block = Block::new(columns, 1);
        assert_eq!(block.filter(&[false]).num_rows(), 0);
    }
}
