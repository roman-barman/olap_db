use crate::block::Block;
use crate::helpers;
use crate::types::DataType;

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

    pub fn scan(&self) -> impl Iterator<Item = &Block> {
        self.blocks.iter()
    }

    pub fn schema(&self) -> &[(String, DataType)] {
        &self.schema
    }
}
