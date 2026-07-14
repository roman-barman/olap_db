use crate::column::Column;

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
}
