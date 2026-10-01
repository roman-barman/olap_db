use crate::Schema;
use crate::core::Column;

#[derive(Debug)]
pub struct Block {
    columns: Vec<(String, Column)>,
    num_rows: usize,
}

impl Block {
    pub fn new(columns: Vec<(String, Column)>, num_rows: usize) -> Self {
        for i in 1..columns.len() {
            let (name, _) = &columns[i];
            assert!(
                !columns[..i].iter().any(|(n, _)| n == name),
                "duplicate column name: {name}"
            );
        }

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

    pub(crate) fn num_rows(&self) -> usize {
        self.num_rows
    }

    pub(crate) fn column(&self, name: &str) -> Option<&Column> {
        self.columns.iter().find(|(n, _)| n == name).map(|(_, c)| c)
    }

    #[cfg(test)]
    pub(crate) fn filter(&self, mask: &[bool]) -> Block {
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
            // TODO: Block → Column → StringColumn»
            let new_column = column.filter(mask);
            new_columns.push((name.clone(), new_column));
        }
        Self::new(new_columns, num_rows)
    }

    pub(crate) fn columns(&self) -> &[(String, Column)] {
        &self.columns
    }
}

pub(crate) fn sort_blocks(
    blocks: &[&Block],
    key: &str,
    schema: &Schema,
    block_size: usize,
) -> Vec<Block> {
    let total_rows_count: usize = blocks.iter().map(|block| block.num_rows).sum();

    let mut result_columns: Vec<(String, Column)> = schema
        .iter()
        .map(|(name, dt)| (name.clone(), Column::with_capacity(*dt, total_rows_count)))
        .collect();

    for block in blocks {
        block
            .columns
            .iter()
            .zip(&mut result_columns)
            .for_each(|((_, source_cl), (_, result_cl))| result_cl.append(source_cl));
    }

    let key_index = schema
        .iter()
        .position(|(n, _)| n == key)
        .expect("key not found");
    let mut perm: Vec<usize> = (0..result_columns[key_index].1.len()).collect();
    let Column::Int64(key_column) = &result_columns[key_index].1 else {
        unreachable!("sort key '{key}' is not Int64 — validated at create/open");
    };
    perm.sort_by_key(|&i| key_column[i]);

    let mut result = Vec::new();
    for chunk in perm.chunks(block_size) {
        let cols: Vec<(String, Column)> = result_columns
            .iter()
            .map(|(name, col)| (name.clone(), col.gather(chunk)))
            .collect();
        result.push(Block::new(cols, chunk.len()));
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DataType;
    use crate::core::StringColumn;

    fn sample_columns() -> Vec<(String, Column)> {
        vec![
            ("id".to_string(), Column::Int64(vec![1, 2, 3])),
            (
                "name".to_string(),
                Column::String(StringColumn::new_with_values(&["a", "b", "c"])),
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
            Some(&Column::String(StringColumn::new_with_values(&[
                "a", "b", "c"
            ])))
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
        assert_eq!(
            filtered.column("name"),
            Some(&Column::String(StringColumn::new()))
        );
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
            Some(&Column::String(StringColumn::new_with_values(&["a", "c"])))
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
            Some(&Column::String(StringColumn::new_with_values(&[
                "a", "b", "c"
            ])))
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

    // ---- sort_blocks ---------------------------------------------------

    fn sort_schema() -> Schema {
        Schema::new(vec![
            ("id".to_string(), DataType::Int64),
            ("name".to_string(), DataType::String),
            ("score".to_string(), DataType::Float64),
        ])
        .unwrap()
    }

    fn sort_block(ids: &[i64], names: &[&str], scores: &[f64]) -> Block {
        Block::new(
            vec![
                ("id".to_string(), Column::Int64(ids.to_vec())),
                (
                    "name".to_string(),
                    Column::String(StringColumn::new_with_values(names)),
                ),
                ("score".to_string(), Column::Float64(scores.to_vec())),
            ],
            ids.len(),
        )
    }

    fn ids_of(block: &Block) -> Vec<i64> {
        match block.column("id") {
            Some(Column::Int64(v)) => v.clone(),
            other => panic!("unexpected id column: {other:?}"),
        }
    }

    fn names_of(block: &Block) -> Vec<String> {
        match block.column("name") {
            Some(Column::String(s)) => (0..s.len()).map(|i| s.get(i).to_string()).collect(),
            other => panic!("unexpected name column: {other:?}"),
        }
    }

    fn scores_of(block: &Block) -> Vec<f64> {
        match block.column("score") {
            Some(Column::Float64(v)) => v.clone(),
            other => panic!("unexpected score column: {other:?}"),
        }
    }

    #[test]
    fn sort_blocks_single_block_sorts_rows_by_key() {
        let b = sort_block(&[3, 1, 2], &["c", "a", "b"], &[3.0, 1.0, 2.0]);
        let out = sort_blocks(&[&b], "id", &sort_schema(), 10);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].num_rows(), 3);
        assert_eq!(ids_of(&out[0]), vec![1, 2, 3]);
        assert_eq!(names_of(&out[0]), vec!["a", "b", "c"]);
        assert_eq!(scores_of(&out[0]), vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn sort_blocks_merges_rows_from_multiple_blocks() {
        let b1 = sort_block(&[5, 1], &["e", "a"], &[5.0, 1.0]);
        let b2 = sort_block(&[4, 2, 3], &["d", "b", "c"], &[4.0, 2.0, 3.0]);
        let out = sort_blocks(&[&b1, &b2], "id", &sort_schema(), 10);
        assert_eq!(out.len(), 1);
        assert_eq!(ids_of(&out[0]), vec![1, 2, 3, 4, 5]);
        assert_eq!(names_of(&out[0]), vec!["a", "b", "c", "d", "e"]);
        assert_eq!(scores_of(&out[0]), vec![1.0, 2.0, 3.0, 4.0, 5.0]);
    }

    #[test]
    fn sort_blocks_splits_output_into_block_size_chunks() {
        let b1 = sort_block(&[5, 3, 1], &["e", "c", "a"], &[5.0, 3.0, 1.0]);
        let b2 = sort_block(&[4, 2], &["d", "b"], &[4.0, 2.0]);
        let out = sort_blocks(&[&b1, &b2], "id", &sort_schema(), 2);
        let sizes: Vec<usize> = out.iter().map(|b| b.num_rows()).collect();
        assert_eq!(sizes, vec![2, 2, 1]);
        assert_eq!(ids_of(&out[0]), vec![1, 2]);
        assert_eq!(ids_of(&out[1]), vec![3, 4]);
        assert_eq!(ids_of(&out[2]), vec![5]);
        assert_eq!(names_of(&out[2]), vec!["e"]);
        assert_eq!(scores_of(&out[1]), vec![3.0, 4.0]);
    }

    #[test]
    fn sort_blocks_total_rows_multiple_of_block_size_has_no_empty_tail() {
        let b = sort_block(&[4, 3, 2, 1], &["d", "c", "b", "a"], &[4.0, 3.0, 2.0, 1.0]);
        let out = sort_blocks(&[&b], "id", &sort_schema(), 2);
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|b| b.num_rows() == 2));
    }

    #[test]
    fn sort_blocks_block_size_one_yields_one_row_per_block() {
        let b = sort_block(&[2, 3, 1], &["b", "c", "a"], &[2.0, 3.0, 1.0]);
        let out = sort_blocks(&[&b], "id", &sort_schema(), 1);
        assert_eq!(out.len(), 3);
        let ids: Vec<i64> = out.iter().flat_map(ids_of).collect();
        assert_eq!(ids, vec![1, 2, 3]);
    }

    #[test]
    fn sort_blocks_is_stable_for_equal_keys() {
        let b1 = sort_block(&[2, 1, 2], &["x", "a", "y"], &[1.0, 2.0, 3.0]);
        let b2 = sort_block(&[1, 2], &["b", "z"], &[4.0, 5.0]);
        let out = sort_blocks(&[&b1, &b2], "id", &sort_schema(), 10);
        assert_eq!(ids_of(&out[0]), vec![1, 1, 2, 2, 2]);
        assert_eq!(names_of(&out[0]), vec!["a", "b", "x", "y", "z"]);
        assert_eq!(scores_of(&out[0]), vec![2.0, 4.0, 1.0, 3.0, 5.0]);
    }

    #[test]
    fn sort_blocks_handles_negative_and_extreme_keys() {
        let b = sort_block(
            &[0, i64::MAX, -5, i64::MIN],
            &["zero", "max", "neg", "min"],
            &[0.0, 1.0, 2.0, 3.0],
        );
        let out = sort_blocks(&[&b], "id", &sort_schema(), 10);
        assert_eq!(ids_of(&out[0]), vec![i64::MIN, -5, 0, i64::MAX]);
        assert_eq!(names_of(&out[0]), vec!["min", "neg", "zero", "max"]);
    }

    #[test]
    fn sort_blocks_key_not_first_column() {
        let schema = Schema::new(vec![
            ("name".to_string(), DataType::String),
            ("ts".to_string(), DataType::Int64),
        ])
        .unwrap();
        let b = Block::new(
            vec![
                (
                    "name".to_string(),
                    Column::String(StringColumn::new_with_values(&["c", "a", "b"])),
                ),
                ("ts".to_string(), Column::Int64(vec![30, 10, 20])),
            ],
            3,
        );
        let out = sort_blocks(&[&b], "ts", &schema, 10);
        assert_eq!(out[0].column("ts"), Some(&Column::Int64(vec![10, 20, 30])));
        assert_eq!(
            out[0].column("name"),
            Some(&Column::String(StringColumn::new_with_values(&[
                "a", "b", "c"
            ])))
        );
    }

    #[test]
    fn sort_blocks_preserves_schema_column_order() {
        let b = sort_block(&[2, 1], &["b", "a"], &[2.0, 1.0]);
        let out = sort_blocks(&[&b], "id", &sort_schema(), 10);
        let names: Vec<&str> = out[0].columns().iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["id", "name", "score"]);
    }

    #[test]
    fn sort_blocks_skips_empty_input_blocks() {
        let empty = sort_block(&[], &[], &[]);
        let b = sort_block(&[2, 1], &["b", "a"], &[2.0, 1.0]);
        let out = sort_blocks(&[&empty, &b, &empty], "id", &sort_schema(), 10);
        assert_eq!(out.len(), 1);
        assert_eq!(ids_of(&out[0]), vec![1, 2]);
        assert_eq!(names_of(&out[0]), vec!["a", "b"]);
    }

    #[test]
    fn sort_blocks_no_input_blocks_returns_empty() {
        let out = sort_blocks(&[], "id", &sort_schema(), 10);
        assert!(out.is_empty());
    }

    #[test]
    fn sort_blocks_only_empty_blocks_returns_empty() {
        let empty = sort_block(&[], &[], &[]);
        let out = sort_blocks(&[&empty, &empty], "id", &sort_schema(), 10);
        assert!(out.is_empty());
    }

    #[test]
    fn sort_blocks_does_not_mutate_input_blocks() {
        let b = sort_block(&[3, 1, 2], &["c", "a", "b"], &[3.0, 1.0, 2.0]);
        let _ = sort_blocks(&[&b], "id", &sort_schema(), 2);
        assert_eq!(ids_of(&b), vec![3, 1, 2]);
        assert_eq!(names_of(&b), vec!["c", "a", "b"]);
    }

    #[test]
    #[should_panic(expected = "key not found")]
    fn sort_blocks_unknown_key_panics() {
        let b = sort_block(&[1], &["a"], &[1.0]);
        sort_blocks(&[&b], "missing", &sort_schema(), 10);
    }

    #[test]
    #[should_panic(expected = "is not Int64")]
    fn sort_blocks_non_int64_key_panics() {
        let b = sort_block(&[1], &["a"], &[1.0]);
        sort_blocks(&[&b], "name", &sort_schema(), 10);
    }
}
