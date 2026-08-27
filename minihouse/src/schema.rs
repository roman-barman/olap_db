use crate::DataType;
use std::ops::Index;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schema(Vec<(String, DataType)>);

impl Schema {
    pub fn new(schema: Vec<(String, DataType)>) -> Result<Self, SchemaError> {
        if schema.is_empty() {
            return Err(SchemaError::Empty);
        }

        for i in 0..schema.len() {
            if schema[i].0.is_empty() {
                return Err(SchemaError::EmptyColumnName);
            }

            if schema[..i].iter().any(|(n, _)| n == &schema[i].0) {
                return Err(SchemaError::DuplicateColumnName(schema[i].0.clone()));
            }
        }

        Ok(Self(schema))
    }

    pub fn iter(&self) -> impl Iterator<Item = &(String, DataType)> {
        self.0.iter()
    }

    pub(crate) fn len(&self) -> usize {
        self.0.len()
    }
}

impl Index<usize> for Schema {
    type Output = (String, DataType);

    fn index(&self, index: usize) -> &Self::Output {
        &self.0[index]
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SchemaError {
    #[error("schema is empty")]
    Empty,
    #[error("duplicate column name '{0}'")]
    DuplicateColumnName(String),
    #[error("column name is empty")]
    EmptyColumnName,
}
