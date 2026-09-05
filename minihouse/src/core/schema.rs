use crate::core::DataType;
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

#[cfg(test)]
mod tests {
    use super::*;

    // ---- fixtures ------------------------------------------------------

    fn col(name: &str, dt: DataType) -> (String, DataType) {
        (name.to_string(), dt)
    }

    /// One column per `DataType`, the same shape `table.rs` and `part_reader.rs`
    /// use for their fixtures.
    fn sample() -> Schema {
        Schema::new(vec![
            col("id", DataType::Int64),
            col("name", DataType::String),
            col("score", DataType::Float64),
        ])
        .unwrap()
    }

    // ---- assertions ----------------------------------------------------

    /// `Schema` derives `Debug`, so `unwrap_err` is available directly — no
    /// match-based helper is needed here, unlike `table.rs::create_error`.
    fn err(columns: Vec<(String, DataType)>) -> SchemaError {
        Schema::new(columns).unwrap_err()
    }

    fn names(schema: &Schema) -> Vec<&str> {
        schema.iter().map(|(n, _)| n.as_str()).collect()
    }

    // ---- new: accepted -------------------------------------------------

    #[test]
    fn new_accepts_a_single_column() {
        let schema = Schema::new(vec![col("id", DataType::Int64)]).unwrap();

        assert_eq!(schema.len(), 1);
        assert_eq!(schema[0], col("id", DataType::Int64));
    }

    #[test]
    fn new_accepts_several_distinct_columns() {
        assert_eq!(sample().len(), 3);
    }

    /// The load-bearing guarantee: column position is the on-disk layout.
    /// `PartWriter::write_block` and `Table::insert` zip `iter()` against the
    /// block's columns positionally, and `part_reader` selects a reader with
    /// `schema[idx].1`. Reordering here would silently mis-type every column.
    #[test]
    fn new_preserves_declaration_order() {
        let schema = Schema::new(vec![
            col("zulu", DataType::Float64),
            col("alpha", DataType::String),
            col("mike", DataType::Int64),
        ])
        .unwrap();

        assert_eq!(names(&schema), ["zulu", "alpha", "mike"]);
        assert_eq!(schema[0].1, DataType::Float64);
        assert_eq!(schema[1].1, DataType::String);
        assert_eq!(schema[2].1, DataType::Int64);
    }

    /// Names are compared byte-for-byte, so `id` and `ID` are two columns.
    /// `part_reader.rs` already round-trips exactly this schema through a part.
    #[test]
    fn new_treats_column_names_case_sensitively() {
        let schema =
            Schema::new(vec![col("id", DataType::Int64), col("ID", DataType::Int64)]).unwrap();

        assert_eq!(names(&schema), ["id", "ID"]);
    }

    /// Nothing is trimmed — only a truly empty name is rejected. A schema file
    /// with whitespace damage therefore survives validation and surfaces as a
    /// column nobody can address, rather than as a `SchemaError`.
    #[test]
    fn new_does_not_trim_column_names() {
        assert!(Schema::new(vec![col(" ", DataType::Int64)]).is_ok());

        let schema = Schema::new(vec![
            col("id", DataType::Int64),
            col(" id", DataType::Int64),
        ])
        .unwrap();

        assert_eq!(names(&schema), ["id", " id"]);
    }

    /// `Schema` does not police the `column={name}:{type}` serialization format;
    /// keeping the two concerns separate is why `part_reader` has its own tests
    /// for a name containing `=`.
    #[test]
    fn new_accepts_names_containing_schema_file_delimiters() {
        let schema = Schema::new(vec![
            col("a=b", DataType::Int64),
            col("c:d", DataType::String),
        ])
        .unwrap();

        assert_eq!(names(&schema), ["a=b", "c:d"]);
    }

    // ---- new: rejected -------------------------------------------------

    #[test]
    fn new_rejects_an_empty_schema() {
        let e = err(vec![]);

        assert!(matches!(e, SchemaError::Empty), "{e:?}");
        assert_eq!(e.to_string(), "schema is empty");
    }

    #[test]
    fn new_rejects_an_empty_name_in_the_first_column() {
        let e = err(vec![col("", DataType::Int64)]);

        assert!(matches!(e, SchemaError::EmptyColumnName), "{e:?}");
        assert_eq!(e.to_string(), "column name is empty");
    }

    #[test]
    fn new_rejects_an_empty_name_in_a_later_column() {
        let e = err(vec![
            col("id", DataType::Int64),
            col("name", DataType::String),
            col("", DataType::Float64),
        ]);

        assert!(matches!(e, SchemaError::EmptyColumnName), "{e:?}");
    }

    /// The rendered message is not internal: `table.rs` and `part_reader.rs` wrap
    /// it verbatim into `StorageError::Corrupt`, and their tests assert on the
    /// full string `corrupt: duplicate column name 'a'`.
    #[test]
    fn new_rejects_a_duplicate_column_name() {
        let e = err(vec![col("id", DataType::Int64), col("id", DataType::Int64)]);

        assert!(
            matches!(e, SchemaError::DuplicateColumnName(ref n) if n == "id"),
            "{e:?}"
        );
        assert_eq!(e.to_string(), "duplicate column name 'id'");
    }

    /// Only the name is compared — a repeated name is a conflict even when the
    /// two declarations disagree about the type.
    #[test]
    fn new_rejects_a_duplicate_name_with_a_different_type() {
        let e = err(vec![
            col("id", DataType::Int64),
            col("id", DataType::String),
        ]);

        assert!(
            matches!(e, SchemaError::DuplicateColumnName(ref n) if n == "id"),
            "{e:?}"
        );
    }

    /// The duplicate scan looks back over every earlier column, not just the
    /// previous one.
    #[test]
    fn new_rejects_a_non_adjacent_duplicate() {
        let e = err(vec![
            col("id", DataType::Int64),
            col("name", DataType::String),
            col("id", DataType::Float64),
        ]);

        assert!(
            matches!(e, SchemaError::DuplicateColumnName(ref n) if n == "id"),
            "{e:?}"
        );
    }

    /// Documents the scan order: columns are validated left to right, so the
    /// reported name is the first repeat encountered, not the last.
    #[test]
    fn new_reports_the_first_duplicate_when_several_names_repeat() {
        let e = err(vec![
            col("id", DataType::Int64),
            col("name", DataType::String),
            col("id", DataType::Float64),
            col("name", DataType::Int64),
        ]);

        assert_eq!(e.to_string(), "duplicate column name 'id'");
    }

    // ---- iter ----------------------------------------------------------

    /// `iter` drives the order of the `column=` lines in `schema.txt` and the
    /// positional zip against a block's columns.
    #[test]
    fn iter_yields_every_column_in_declaration_order() {
        let schema = sample();

        let columns: Vec<&(String, DataType)> = schema.iter().collect();

        assert_eq!(
            columns,
            [
                &col("id", DataType::Int64),
                &col("name", DataType::String),
                &col("score", DataType::Float64),
            ]
        );
    }

    #[test]
    fn iter_borrows_and_can_be_called_repeatedly() {
        let schema = sample();

        assert_eq!(schema.iter().count(), schema.len());
        assert_eq!(schema.iter().count(), schema.len());
        assert_eq!(names(&schema), ["id", "name", "score"]);
    }

    // ---- len -----------------------------------------------------------

    #[test]
    fn len_matches_the_number_of_declared_columns() {
        assert_eq!(
            Schema::new(vec![col("id", DataType::Int64)]).unwrap().len(),
            1
        );
        assert_eq!(sample().len(), 3);
    }

    // ---- Index ---------------------------------------------------------

    /// `part_reader` reaches for `schema[idx]` to pick the single-file or
    /// paired-file reader, so indexing and `iter` must not drift apart.
    #[test]
    fn index_returns_each_column_and_agrees_with_iter() {
        let schema = sample();

        for (i, column) in schema.iter().enumerate() {
            assert_eq!(&schema[i], column);
        }
    }

    #[test]
    #[should_panic(expected = "index out of bounds")]
    fn index_past_the_last_column_panics() {
        let schema = sample();

        let _ = &schema[3];
    }

    // ---- derived traits ------------------------------------------------

    /// `Table::insert` clones the schema for each `PartWriter`; the clone must
    /// describe the same part layout.
    #[test]
    fn clone_equals_the_original() {
        let schema = sample();

        assert_eq!(schema.clone(), schema);
    }

    /// Equality is positional, matching the on-disk meaning: the same columns in
    /// a different order describe a different part.
    #[test]
    fn schemas_differing_only_in_column_order_are_not_equal() {
        let a = Schema::new(vec![
            col("id", DataType::Int64),
            col("name", DataType::String),
        ])
        .unwrap();
        let b = Schema::new(vec![
            col("name", DataType::String),
            col("id", DataType::Int64),
        ])
        .unwrap();

        assert_ne!(a, b);
    }

    #[test]
    fn schemas_differing_only_in_a_column_type_are_not_equal() {
        let a = Schema::new(vec![col("id", DataType::Int64)]).unwrap();
        let b = Schema::new(vec![col("id", DataType::Float64)]).unwrap();

        assert_ne!(a, b);
    }
}
