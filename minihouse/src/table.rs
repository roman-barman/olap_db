use crate::DataType;
use crate::block::Block;
use crate::helpers;

pub struct Table {
    schema: Vec<(String, DataType)>,
    blocks: Vec<Block>,
}

impl Table {
    pub fn new(schema: Vec<(String, DataType)>) -> Self {
        assert!(!schema.is_empty(), "Schema must have at least one column");
        helpers::assert_unique_names(&schema);

        Self {
            schema,
            blocks: Vec::new(),
        }
    }

    pub fn insert(&mut self, block: Block) {
        let columns = block.columns();
        assert_eq!(
            columns.len(),
            self.schema.len(),
            "block has different number of columns than table"
        );
        for (i, ((s_name, s_type), (b_name, b_col))) in self.schema.iter().zip(columns).enumerate()
        {
            assert!(
                s_name == b_name && b_col.data_type() == *s_type,
                "insert: column {i} expected ('{s_name}', {s_type:?}), got ('{b_name}', {:?})",
                b_col.data_type()
            );
        }

        if block.num_rows() == 0 {
            return;
        }

        self.blocks.push(block);
    }

    pub(crate) fn scan(&self) -> impl Iterator<Item = &Block> {
        self.blocks.iter()
    }

    pub(crate) fn schema(&self) -> &[(String, DataType)] {
        &self.schema
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::column::Column;

    fn sample_schema() -> Vec<(String, DataType)> {
        vec![
            ("id".to_string(), DataType::Int64),
            ("name".to_string(), DataType::String),
            ("score".to_string(), DataType::Float64),
        ]
    }

    fn sample_block(ids: Vec<i64>, names: Vec<&str>, scores: Vec<f64>) -> Block {
        let n = ids.len();
        Block::new(
            vec![
                ("id".to_string(), Column::Int64(ids)),
                (
                    "name".to_string(),
                    Column::String(names.into_iter().map(String::from).collect()),
                ),
                ("score".to_string(), Column::Float64(scores)),
            ],
            n,
        )
    }

    #[test]
    fn new_with_single_column_schema_succeeds() {
        let schema = vec![("id".to_string(), DataType::Int64)];
        let table = Table::new(schema.clone());
        assert_eq!(table.schema(), schema.as_slice());
    }

    #[test]
    fn new_with_multi_column_schema_succeeds() {
        let schema = sample_schema();
        let table = Table::new(schema.clone());
        assert_eq!(table.schema(), schema.as_slice());
    }

    #[test]
    #[should_panic(expected = "Schema must have at least one column")]
    fn new_empty_schema_panics() {
        Table::new(vec![]);
    }

    #[test]
    #[should_panic(expected = "duplicate column name: a")]
    fn new_duplicate_column_name_adjacent_panics() {
        Table::new(vec![
            ("a".to_string(), DataType::Int64),
            ("a".to_string(), DataType::Int64),
        ]);
    }

    #[test]
    #[should_panic(expected = "duplicate column name: a")]
    fn new_duplicate_column_name_non_adjacent_panics() {
        Table::new(vec![
            ("a".to_string(), DataType::Int64),
            ("b".to_string(), DataType::Int64),
            ("a".to_string(), DataType::Float64),
        ]);
    }

    #[test]
    fn insert_single_valid_block_is_scanned_back() {
        let schema = sample_schema();
        let mut table = Table::new(schema.clone());
        table.insert(sample_block(
            vec![1, 2, 3],
            vec!["a", "b", "c"],
            vec![1.5, 2.5, 3.5],
        ));

        let blocks: Vec<&Block> = table.scan().collect();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].num_rows(), 3);
        assert_eq!(blocks[0].column("id"), Some(&Column::Int64(vec![1, 2, 3])));
        assert_eq!(
            blocks[0].column("name"),
            Some(&Column::String(vec!["a".into(), "b".into(), "c".into()]))
        );
        assert_eq!(
            blocks[0].column("score"),
            Some(&Column::Float64(vec![1.5, 2.5, 3.5]))
        );
        assert_eq!(table.schema(), schema.as_slice());
    }

    #[test]
    fn insert_multiple_blocks_accumulate_in_insertion_order() {
        let mut table = Table::new(sample_schema());
        table.insert(sample_block(vec![1, 2], vec!["a", "b"], vec![1.0, 2.0]));
        table.insert(sample_block(
            vec![3, 4, 5],
            vec!["c", "d", "e"],
            vec![3.0, 4.0, 5.0],
        ));
        table.insert(sample_block(vec![6], vec!["f"], vec![6.0]));

        let blocks: Vec<&Block> = table.scan().collect();
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0].column("id"), Some(&Column::Int64(vec![1, 2])));
        assert_eq!(blocks[1].column("id"), Some(&Column::Int64(vec![3, 4, 5])));
        assert_eq!(blocks[2].column("id"), Some(&Column::Int64(vec![6])));
    }

    #[test]
    #[should_panic(expected = "block has different number of columns than table")]
    fn insert_wrong_number_of_columns_too_few_panics() {
        let mut table = Table::new(sample_schema());
        let block = Block::new(
            vec![
                ("id".to_string(), Column::Int64(vec![1])),
                ("name".to_string(), Column::String(vec!["a".into()])),
            ],
            1,
        );
        table.insert(block);
    }

    #[test]
    #[should_panic(expected = "block has different number of columns than table")]
    fn insert_wrong_number_of_columns_too_many_panics() {
        let mut table = Table::new(vec![
            ("id".to_string(), DataType::Int64),
            ("name".to_string(), DataType::String),
        ]);
        let block = Block::new(
            vec![
                ("id".to_string(), Column::Int64(vec![1])),
                ("name".to_string(), Column::String(vec!["a".into()])),
                ("extra".to_string(), Column::Float64(vec![1.0])),
            ],
            1,
        );
        table.insert(block);
    }

    #[test]
    #[should_panic(expected = "insert: column 0 expected ('id', Int64), got ('wrong', Int64)")]
    fn insert_column_name_mismatch_at_index_zero_panics() {
        let mut table = Table::new(sample_schema());
        let block = Block::new(
            vec![
                ("wrong".to_string(), Column::Int64(vec![1])),
                ("name".to_string(), Column::String(vec!["a".into()])),
                ("score".to_string(), Column::Float64(vec![1.0])),
            ],
            1,
        );
        table.insert(block);
    }

    #[test]
    #[should_panic(expected = "insert: column 2 expected ('score', Float64), got ('bad', Float64)")]
    fn insert_column_name_mismatch_at_later_index_panics() {
        let mut table = Table::new(sample_schema());
        let block = Block::new(
            vec![
                ("id".to_string(), Column::Int64(vec![1])),
                ("name".to_string(), Column::String(vec!["a".into()])),
                ("bad".to_string(), Column::Float64(vec![1.0])),
            ],
            1,
        );
        table.insert(block);
    }

    #[test]
    #[should_panic(expected = "insert: column 0 expected ('id', Int64), got ('id', Float64)")]
    fn insert_column_type_mismatch_with_matching_name_panics() {
        let mut table = Table::new(sample_schema());
        let block = Block::new(
            vec![
                ("id".to_string(), Column::Float64(vec![1.0])),
                ("name".to_string(), Column::String(vec!["a".into()])),
                ("score".to_string(), Column::Float64(vec![1.0])),
            ],
            1,
        );
        table.insert(block);
    }

    #[test]
    fn insert_zero_row_block_is_silently_dropped() {
        let mut table = Table::new(sample_schema());
        let empty_block = Block::new(
            vec![
                ("id".to_string(), Column::Int64(vec![])),
                ("name".to_string(), Column::String(vec![])),
                ("score".to_string(), Column::Float64(vec![])),
            ],
            0,
        );
        table.insert(empty_block);

        assert_eq!(table.scan().count(), 0);
    }

    #[test]
    fn insert_zero_row_block_among_valid_inserts_only_empty_one_skipped() {
        let mut table = Table::new(sample_schema());
        table.insert(sample_block(vec![1, 2], vec!["a", "b"], vec![1.0, 2.0]));
        let empty_block = Block::new(
            vec![
                ("id".to_string(), Column::Int64(vec![])),
                ("name".to_string(), Column::String(vec![])),
                ("score".to_string(), Column::Float64(vec![])),
            ],
            0,
        );
        table.insert(empty_block);
        table.insert(sample_block(vec![3], vec!["c"], vec![3.0]));

        let blocks: Vec<&Block> = table.scan().collect();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].column("id"), Some(&Column::Int64(vec![1, 2])));
        assert_eq!(blocks[1].column("id"), Some(&Column::Int64(vec![3])));
    }

    #[test]
    #[should_panic(expected = "insert: column 0 expected ('id', Int64), got ('name', String)")]
    fn insert_permuted_column_order_panics() {
        let mut table = Table::new(sample_schema());
        let block = Block::new(
            vec![
                ("name".to_string(), Column::String(vec!["a".into()])),
                ("id".to_string(), Column::Int64(vec![1])),
                ("score".to_string(), Column::Float64(vec![1.0])),
            ],
            1,
        );
        table.insert(block);
    }

    #[test]
    #[should_panic(expected = "block has different number of columns than table")]
    fn insert_empty_block_with_wrong_column_count_still_panics() {
        let mut table = Table::new(sample_schema());
        let empty_block = Block::new(vec![], 0);
        table.insert(empty_block);
    }

    #[test]
    fn insert_large_block_many_rows_succeeds() {
        let mut table = Table::new(vec![("id".to_string(), DataType::Int64)]);
        let ids: Vec<i64> = (0..1000).collect();
        table.insert(Block::new(
            vec![("id".to_string(), Column::Int64(ids.clone()))],
            1000,
        ));

        let blocks: Vec<&Block> = table.scan().collect();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].num_rows(), 1000);
        assert_eq!(blocks[0].column("id"), Some(&Column::Int64(ids)));
    }

    #[test]
    fn scan_on_empty_table_yields_no_blocks() {
        let table = Table::new(sample_schema());
        assert_eq!(table.scan().count(), 0);
    }

    #[test]
    fn scan_yields_blocks_in_insertion_order() {
        let mut table = Table::new(vec![("id".to_string(), DataType::Int64)]);
        table.insert(Block::new(
            vec![("id".to_string(), Column::Int64(vec![1]))],
            1,
        ));
        table.insert(Block::new(
            vec![("id".to_string(), Column::Int64(vec![2, 3]))],
            2,
        ));
        table.insert(Block::new(
            vec![("id".to_string(), Column::Int64(vec![4, 5, 6]))],
            3,
        ));

        let blocks: Vec<&Block> = table.scan().collect();
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0].num_rows(), 1);
        assert_eq!(blocks[0].column("id"), Some(&Column::Int64(vec![1])));
        assert_eq!(blocks[1].num_rows(), 2);
        assert_eq!(blocks[1].column("id"), Some(&Column::Int64(vec![2, 3])));
        assert_eq!(blocks[2].num_rows(), 3);
        assert_eq!(blocks[2].column("id"), Some(&Column::Int64(vec![4, 5, 6])));
    }

    #[test]
    fn scan_can_be_called_multiple_times_non_consuming() {
        let mut table = Table::new(sample_schema());
        table.insert(sample_block(vec![1, 2], vec!["a", "b"], vec![1.0, 2.0]));
        table.insert(sample_block(vec![3], vec!["c"], vec![3.0]));

        let first: Vec<i64> = table.scan().map(|b| b.num_rows() as i64).collect();
        let second: Vec<i64> = table.scan().map(|b| b.num_rows() as i64).collect();
        assert_eq!(first, second);
        assert_eq!(first, vec![2, 1]);
    }

    #[test]
    fn schema_unaffected_by_inserts() {
        let schema = sample_schema();
        let mut table = Table::new(schema.clone());
        table.insert(sample_block(vec![1, 2], vec!["a", "b"], vec![1.0, 2.0]));
        table.insert(sample_block(vec![3], vec!["c"], vec![3.0]));

        assert_eq!(table.schema(), schema.as_slice());
    }

    #[test]
    fn schema_preserves_declared_column_order() {
        let schema = vec![
            ("z".to_string(), DataType::Int64),
            ("a".to_string(), DataType::String),
            ("m".to_string(), DataType::Float64),
        ];
        let table = Table::new(schema.clone());
        assert_eq!(table.schema(), schema.as_slice());
    }
}
