use crate::DataType;
use crate::block::Block;
use crate::codec::Codec;
use crate::part_reader::PartReader;
use crate::part_writer::PartWriter;
use crate::schema::Schema;
use crate::storage_error::StorageError;
use std::fs;
use std::fs::File;
use std::io::{BufWriter, ErrorKind, Write};
use std::path::{Path, PathBuf};

pub struct Table {
    schema: Schema,
    dir: PathBuf,
    next_part_id: usize,
    codec: Codec,
}

impl Table {
    pub fn create(dir: PathBuf, schema: Schema, codec: Codec) -> Result<Self, StorageError> {
        if dir.exists() {
            return Err(StorageError::AlreadyExists(dir));
        }

        let schema_file = dir.join("schema.txt");
        fs::create_dir_all(&dir)?;
        let mut schema_writer = BufWriter::new(File::create(schema_file)?);
        schema_writer.write_all(b"version=1\n")?;
        writeln!(schema_writer, "codec={}", codec.as_str())?;
        for (name, data_type) in schema.iter() {
            writeln!(schema_writer, "column={}:{}", name, data_type.as_str())?;
        }
        schema_writer.flush()?;

        Ok(Self {
            schema,
            dir,
            next_part_id: 0,
            codec,
        })
    }

    pub fn open(dir: PathBuf) -> Result<Table, StorageError> {
        if !Path::is_dir(&dir) {
            return Err(StorageError::NotFound(dir));
        }

        let text = match fs::read_to_string(dir.join("schema.txt")) {
            Ok(text) => text,
            Err(err) if err.kind() == ErrorKind::NotFound => {
                return Err(StorageError::Corrupt("schema.txt missing".into()));
            }
            Err(err) => return Err(err.into()),
        };

        let (schema, codec) = parse_schema(&text)?;
        let parts = list_parts(&dir)?;
        let next_part_id = parts.last().map(|(id, _)| id + 1).unwrap_or(0);

        Ok(Self {
            schema,
            dir,
            next_part_id,
            codec,
        })
    }

    pub fn insert(&mut self, blocks: &[Block]) -> Result<(), StorageError> {
        if blocks.is_empty() {
            return Ok(());
        }

        for block in blocks {
            let columns = block.columns();
            assert_eq!(
                columns.len(),
                self.schema.len(),
                "block has different number of columns than table"
            );
            for (i, ((s_name, s_type), (b_name, b_col))) in
                self.schema.iter().zip(columns).enumerate()
            {
                assert!(
                    s_name == b_name && b_col.data_type() == *s_type,
                    "insert: column {i} expected ('{s_name}', {s_type:?}), got ('{b_name}', {:?})",
                    b_col.data_type()
                );
            }
        }

        let blocks = blocks
            .iter()
            .filter(|b| b.num_rows() > 0)
            .collect::<Vec<_>>();
        if blocks.is_empty() {
            return Ok(());
        }

        let mut writer = PartWriter::new(
            self.dir.join(part_dir_name(self.next_part_id)),
            self.schema.clone(),
            self.codec,
        )?;

        for block in blocks {
            writer.write_block(block)?;
        }

        writer.finish()?;
        self.next_part_id += 1;
        Ok(())
    }

    pub(crate) fn scan<'a>(
        &self,
        columns: &'a [&'a str],
    ) -> Result<impl Iterator<Item = Result<Block, StorageError>>, StorageError> {
        let parts = list_parts(&self.dir)?;
        Ok(BlockIter {
            parts,
            columns,
            reader: None,
            pos: 0,
        })
    }

    pub(crate) fn schema(&self) -> &Schema {
        &self.schema
    }
}

struct BlockIter<'a> {
    parts: Vec<(usize, PathBuf)>,
    columns: &'a [&'a str],
    reader: Option<PartReader>,
    pos: usize,
}

impl<'a> Iterator for BlockIter<'a> {
    type Item = Result<Block, StorageError>;
    fn next(&mut self) -> Option<Self::Item> {
        while self.pos < self.parts.len() {
            let part_reader = match &mut self.reader {
                Some(reader) => reader,
                None => {
                    let (_, path) = &self.parts[self.pos];
                    let reader = PartReader::open(path, self.columns);

                    let reader = match reader {
                        Ok(reader) => reader,
                        Err(e) => {
                            self.pos = self.parts.len();
                            return Some(Err(e));
                        }
                    };

                    self.reader.insert(reader)
                }
            };

            match part_reader.next_block() {
                Ok(block) => match block {
                    Some(block) => return Some(Ok(block)),
                    None => {
                        self.pos += 1;
                        self.reader = None;
                    }
                },
                Err(e) => {
                    self.pos = self.parts.len();
                    return Some(Err(e));
                }
            }
        }

        None
    }
}

fn part_dir_name(id: usize) -> String {
    format!("part_{id:04}")
}

fn parse_schema(text: &str) -> Result<(Schema, Codec), StorageError> {
    if text.is_empty() {
        return Err(StorageError::Corrupt("empty schema".into()));
    }

    let mut lines = text.lines();
    let version = lines
        .next()
        .ok_or_else(|| StorageError::Corrupt("missing version line".into()))?
        .strip_prefix("version=")
        .ok_or_else(|| StorageError::Corrupt("invalid version line".into()))?;
    if version != "1" {
        return Err(StorageError::UnsupportedVersion {
            found: version.to_string(),
            expected: 1,
        });
    }

    let codec = lines
        .next()
        .ok_or_else(|| StorageError::Corrupt("missing codec line".into()))?
        .strip_prefix("codec=")
        .ok_or_else(|| StorageError::Corrupt("invalid codec line".into()))?
        .parse::<Codec>()
        .map_err(|e| StorageError::Corrupt(format!("line 2: {e}")))?;

    let schema = lines
        .enumerate()
        .map(|(i, l)| {
            let line_no = i + 3;
            l.strip_prefix("column=")
                .ok_or_else(|| {
                    StorageError::Corrupt(format!("line {line_no}: expected column=..."))
                })
                .and_then(|d| {
                    d.split_once(':')
                        .ok_or_else(|| {
                            StorageError::Corrupt(format!(
                                "line {line_no}: expected column=<name>:<type>"
                            ))
                        })
                        .and_then(|(name, type_str)| {
                            type_str
                                .parse::<DataType>()
                                .map(|dt| (name.to_string(), dt))
                                .map_err(|_| {
                                    StorageError::Corrupt(format!(
                                        "line {line_no}: invalid data type '{type_str}'"
                                    ))
                                })
                        })
                })
        })
        .collect::<Result<Vec<(String, DataType)>, StorageError>>()?;

    let schema = Schema::new(schema).map_err(|e| StorageError::Corrupt(e.to_string()))?;

    Ok((schema, codec))
}

fn list_parts(dir: &Path) -> Result<Vec<(usize, PathBuf)>, StorageError> {
    let mut parts = fs::read_dir(dir)?
        .filter_map(|entry| {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => return Some(Err(e.into())),
            };
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == "schema.txt" || name.ends_with(".tmp") {
                return None; // известное — пропустить
            }
            Some(
                name.strip_prefix("part_")
                    .and_then(|s| s.parse::<usize>().ok())
                    .filter(|&id| part_dir_name(id) == name)
                    .map(|id| (id, entry.path()))
                    .ok_or_else(|| {
                        StorageError::Corrupt(format!(
                            "unexpected entry '{name}' in table directory"
                        ))
                    }),
            )
        })
        .collect::<Result<Vec<_>, StorageError>>()?;

    parts.sort_by_key(|(id, _)| *id);
    Ok(parts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::column::Column;
    use crate::string_column::StringColumn;
    use crate::test_fixture::{column_names, entries, sample_block, sample_schema};
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use tempfile::TempDir;
    // ---- fixtures ------------------------------------------------------

    /// A temp root plus the table directory inside it. `Table::create` refuses a
    /// path that already exists, so the directory is only *joined* here, never
    /// created. The root must stay alive for the whole test — bind it, don't
    /// discard it.
    fn table_dir() -> (TempDir, PathBuf) {
        let root = TempDir::new().unwrap();
        let dir = root.path().join("tbl");
        (root, dir)
    }

    fn empty_block() -> Block {
        sample_block(&[], &[], &[])
    }

    fn created(schema: Schema) -> (TempDir, PathBuf, Table) {
        let (root, dir) = table_dir();
        let table = Table::create(dir.clone(), schema, Codec::Lz4).unwrap();
        (root, dir, table)
    }

    fn sample_table() -> (TempDir, PathBuf, Table) {
        created(sample_schema())
    }

    /// A table directory containing nothing but a hand-written `schema.txt` —
    /// for the corrupt shapes `create` cannot produce.
    fn table_with_schema_text(text: &str) -> (TempDir, PathBuf) {
        let (root, dir) = table_dir();
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("schema.txt"), text).unwrap();
        (root, dir)
    }

    // ---- assertions ----------------------------------------------------

    /// `Table` has no `Debug`, so `unwrap_err` is unavailable — same shape as
    /// `part_reader.rs::open_error`.
    fn create_error(dir: PathBuf, schema: Schema) -> StorageError {
        match Table::create(dir.clone(), schema, Codec::Lz4) {
            Ok(_) => panic!("expected creating {dir:?} to fail"),
            Err(e) => e,
        }
    }

    fn open_error(dir: PathBuf) -> StorageError {
        match Table::open(dir.clone()) {
            Ok(_) => panic!("expected opening {dir:?} to fail"),
            Err(e) => e,
        }
    }

    fn open_err(dir: PathBuf) -> String {
        open_error(dir).to_string()
    }

    /// Every block a scan yields, or the first error it hits. `scan` returns
    /// `Result<impl Iterator, _>`, which is likewise not `Debug`.
    fn scan_all(table: &Table, columns: &[&str]) -> Result<Vec<Block>, StorageError> {
        table.scan(columns)?.collect()
    }

    fn scan_error(table: &Table, columns: &[&str]) -> StorageError {
        match scan_all(table, columns) {
            Ok(blocks) => panic!("expected scanning {columns:?} to fail, got {blocks:?}"),
            Err(e) => e,
        }
    }

    fn scan_err(table: &Table, columns: &[&str]) -> String {
        scan_error(table, columns).to_string()
    }

    // ---- readers -------------------------------------------------------

    fn schema_text(dir: &Path) -> String {
        fs::read_to_string(dir.join("schema.txt")).unwrap()
    }

    fn row_counts(blocks: &[Block]) -> Vec<usize> {
        blocks.iter().map(|b| b.num_rows()).collect()
    }

    fn i64s(block: &Block, name: &str) -> Vec<i64> {
        match block
            .column(name)
            .unwrap_or_else(|| panic!("no column '{name}'"))
        {
            Column::Int64(v) => v.clone(),
            other => panic!("column '{name}' is {:?}", other.data_type()),
        }
    }

    fn f64s(block: &Block, name: &str) -> Vec<f64> {
        match block
            .column(name)
            .unwrap_or_else(|| panic!("no column '{name}'"))
        {
            Column::Float64(v) => v.clone(),
            other => panic!("column '{name}' is {:?}", other.data_type()),
        }
    }

    /// `StringColumn` has no iterator, so it is indexed one row at a time.
    fn strs(block: &Block, name: &str) -> Vec<String> {
        match block
            .column(name)
            .unwrap_or_else(|| panic!("no column '{name}'"))
        {
            Column::String(sc) => (0..sc.len()).map(|i| sc.get(i).to_string()).collect(),
            other => panic!("column '{name}' is {:?}", other.data_type()),
        }
    }

    /// The `id` column of every block a scan yields, in order.
    fn scanned_ids(table: &Table) -> Vec<Vec<i64>> {
        scan_all(table, &["id"])
            .unwrap()
            .iter()
            .map(|b| i64s(b, "id"))
            .collect()
    }

    // ---- create --------------------------------------------------------

    /// The table-level `schema.txt` deliberately has no `num_rows=` line — that
    /// belongs to the part-level file `PartWriter::finish` writes (see
    /// `part_writer.rs::schema_file_has_exact_expected_contents`). Two different
    /// formats share one file name; this pins them apart.
    #[test]
    fn create_writes_the_schema_file_verbatim() {
        let (_root, dir, _table) = sample_table();

        assert_eq!(
            schema_text(&dir),
            "version=1\ncodec=lz4\ncolumn=id:Int64\ncolumn=name:String\ncolumn=score:Float64\n"
        );
    }

    #[test]
    fn create_writes_nothing_but_the_schema_file() {
        let (_root, dir, _table) = sample_table();

        assert_eq!(entries(&dir), vec!["schema.txt"]);
    }

    #[test]
    fn create_makes_missing_parent_directories() {
        let root = TempDir::new().unwrap();
        let dir = root.path().join("nested/deeper/tbl");

        let table = Table::create(dir.clone(), sample_schema(), Codec::Lz4).unwrap();

        assert!(dir.join("schema.txt").is_file());
        assert_eq!(*table.schema(), sample_schema());
    }

    #[test]
    fn create_returns_the_declared_schema_in_declaration_order() {
        let schema = Schema::new(vec![
            ("zulu".to_string(), DataType::Float64),
            ("alpha".to_string(), DataType::String),
            ("mike".to_string(), DataType::Int64),
        ])
        .unwrap();
        let (_root, dir, table) = created(schema.clone());

        assert_eq!(*table.schema(), schema);
        assert_eq!(
            schema_text(&dir),
            "version=1\ncodec=lz4\ncolumn=zulu:Float64\ncolumn=alpha:String\ncolumn=mike:Int64\n"
        );
    }

    #[test]
    fn create_with_a_single_column_schema_succeeds() {
        let schema = Schema::new(vec![("id".to_string(), DataType::Int64)]).unwrap();
        let (_root, _dir, table) = created(schema.clone());

        assert_eq!(*table.schema(), schema);
    }

    #[test]
    fn create_rejects_an_existing_directory() {
        let (_root, dir) = table_dir();
        fs::create_dir_all(&dir).unwrap();

        let e = create_error(dir.clone(), sample_schema());

        assert!(
            matches!(e, StorageError::AlreadyExists(ref p) if *p == dir),
            "{e:?}"
        );
        assert!(e.to_string().starts_with("already exists: "), "{e}");
    }

    /// `Path::exists` does not distinguish a file from a directory, so a
    /// non-directory squatting on the path is rejected the same way rather than
    /// producing a confusing io error later.
    #[test]
    fn create_rejects_an_existing_file_at_the_table_path() {
        let (_root, dir) = table_dir();
        fs::write(&dir, b"not a table").unwrap();

        let e = create_error(dir.clone(), sample_schema());

        assert!(matches!(e, StorageError::AlreadyExists(_)), "{e:?}");
        assert_eq!(fs::read(&dir).unwrap(), b"not a table");
    }

    /// Pins `next_part_id == 0` on a freshly created table.
    #[test]
    fn the_first_insert_into_a_fresh_table_is_part_0000() {
        let (_root, dir, mut table) = sample_table();

        table.insert(&[sample_block(&[1], &["a"], &[1.0])]).unwrap();

        assert_eq!(entries(&dir), vec!["part_0000", "schema.txt"]);
    }

    // ---- open ----------------------------------------------------------

    #[test]
    fn open_round_trips_a_created_table() {
        let (_root, dir, mut table) = sample_table();
        table
            .insert(&[sample_block(
                &[1, -2, i64::MAX],
                &["a", "", "日本語"],
                &[1.5, -2.5, 0.0],
            )])
            .unwrap();
        drop(table);

        let reopened = Table::open(dir).unwrap();

        assert_eq!(*reopened.schema(), sample_schema());
        let blocks = scan_all(&reopened, &["id", "name", "score"]).unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(i64s(&blocks[0], "id"), vec![1, -2, i64::MAX]);
        assert_eq!(strs(&blocks[0], "name"), vec!["a", "", "日本語"]);
        assert_eq!(f64s(&blocks[0], "score"), vec![1.5, -2.5, 0.0]);
    }

    #[test]
    fn open_rejects_a_missing_directory() {
        let (_root, dir) = table_dir();

        let e = open_error(dir.clone());

        assert!(
            matches!(e, StorageError::NotFound(ref p) if *p == dir),
            "{e:?}"
        );
        assert!(e.to_string().starts_with("not found: "), "{e}");
    }

    #[test]
    fn open_reports_a_missing_schema_file_as_corrupt() {
        let (_root, dir) = table_dir();
        fs::create_dir_all(&dir).unwrap();

        let e = open_error(dir);

        assert!(matches!(e, StorageError::Corrupt(_)), "{e:?}");
        assert_eq!(e.to_string(), "corrupt: schema.txt missing");
    }

    #[test]
    fn open_rejects_an_empty_schema_file() {
        let (_root, dir) = table_with_schema_text("");

        assert_eq!(open_err(dir), "corrupt: empty schema");
    }

    #[test]
    fn open_rejects_an_unsupported_version() {
        let (_root, dir) = table_with_schema_text("version=2\ncolumn=id:Int64\n");

        let e = open_error(dir);

        assert!(
            matches!(e, StorageError::UnsupportedVersion { ref found, expected: 1 } if found == "2"),
            "{e:?}"
        );
        assert_eq!(e.to_string(), "unsupported format version 2, expected 1");
    }

    #[test]
    fn open_rejects_a_version_line_without_the_prefix() {
        let (_root, dir) = table_with_schema_text("v=1\ncolumn=id:Int64\n");

        assert_eq!(open_err(dir), "corrupt: invalid version line");
    }

    #[test]
    fn open_rejects_a_column_line_without_the_prefix() {
        let (_root, dir) = table_with_schema_text("version=1\ncodec=lz4\nid:Int64\n");

        assert_eq!(open_err(dir), "corrupt: line 3: expected column=...");
    }

    #[test]
    fn open_rejects_a_column_line_without_a_colon() {
        let (_root, dir) = table_with_schema_text("version=1\ncodec=lz4\ncolumn=id\n");

        assert_eq!(
            open_err(dir),
            "corrupt: line 3: expected column=<name>:<type>"
        );
    }

    /// The reported line number is `i + 3` — `version=` and `codec=` occupy the
    /// first two lines, and the offending column is the second one, so the
    /// message must say line 4.
    #[test]
    fn open_reports_the_line_number_of_an_unknown_data_type() {
        let (_root, dir) =
            table_with_schema_text("version=1\ncodec=lz4\ncolumn=id:Int64\ncolumn=x:Int32\n");

        assert_eq!(open_err(dir), "corrupt: line 4: invalid data type 'Int32'");
    }

    /// `parse_schema` quotes the name; `helpers::assert_unique_names`, which
    /// guards `create`, does not. Different layer, different message.
    #[test]
    fn open_rejects_duplicate_column_names() {
        let (_root, dir) =
            table_with_schema_text("version=1\ncodec=lz4\ncolumn=a:Int64\ncolumn=a:Float64\n");

        assert_eq!(open_err(dir), "corrupt: duplicate column name 'a'");
    }

    // ---- codec ---------------------------------------------------------

    /// `schema.txt` is the only table-level record of the codec, so it has to
    /// survive a create/open cycle — and it sits on line 2, ahead of the
    /// columns, which is what shifts every column line number by one.
    #[test]
    fn create_records_the_codec_on_line_two_of_the_schema_file() {
        for (codec, line) in [(Codec::None, "codec=none"), (Codec::Lz4, "codec=lz4")] {
            let (_root, dir) = table_dir();
            Table::create(dir.clone(), sample_schema(), codec).unwrap();

            assert_eq!(schema_text(&dir).lines().nth(1), Some(line), "{codec:?}");
            Table::open(dir).unwrap();
        }
    }

    #[test]
    fn open_rejects_a_schema_missing_the_codec_line() {
        let (_root, dir) = table_with_schema_text("version=1\n");

        assert_eq!(open_err(dir), "corrupt: missing codec line");
    }

    #[test]
    fn open_rejects_a_codec_line_without_the_prefix() {
        let (_root, dir) = table_with_schema_text("version=1\nlz4\ncolumn=id:Int64\n");

        assert_eq!(open_err(dir), "corrupt: invalid codec line");
    }

    /// The codec name is parsed, not merely present — and the failure is
    /// reported against line 2, where it lives.
    #[test]
    fn open_rejects_an_unknown_codec_name() {
        let (_root, dir) = table_with_schema_text("version=1\ncodec=zstd\ncolumn=id:Int64\n");

        assert_eq!(open_err(dir), "corrupt: line 2: invalid codec: zstd");
    }

    /// Every block carries its own codec byte, so data written under either
    /// setting reads back identically — `PartReader` is never told which one to
    /// expect.
    #[test]
    fn a_block_round_trips_under_either_codec() {
        for codec in [Codec::None, Codec::Lz4] {
            let (_root, dir) = table_dir();
            let mut table = Table::create(dir, sample_schema(), codec).unwrap();

            table
                .insert(&[sample_block(&[1, 2, 3], &["a", "b", "c"], &[1.5, 2.5, 3.5])])
                .unwrap();

            let blocks = scan_all(&table, &["id", "name", "score"]).unwrap();
            assert_eq!(blocks.len(), 1, "{codec:?}");
            assert_eq!(i64s(&blocks[0], "id"), vec![1, 2, 3], "{codec:?}");
            assert_eq!(strs(&blocks[0], "name"), vec!["a", "b", "c"], "{codec:?}");
            assert_eq!(f64s(&blocks[0], "score"), vec![1.5, 2.5, 3.5], "{codec:?}");
        }
    }

    #[test]
    fn open_continues_part_numbering_after_the_highest_existing_part() {
        let (_root, dir, mut table) = sample_table();
        table.insert(&[sample_block(&[1], &["a"], &[1.0])]).unwrap();
        table.insert(&[sample_block(&[2], &["b"], &[2.0])]).unwrap();
        drop(table);

        let mut reopened = Table::open(dir.clone()).unwrap();
        reopened
            .insert(&[sample_block(&[3], &["c"], &[3.0])])
            .unwrap();

        assert_eq!(
            entries(&dir),
            vec!["part_0000", "part_0001", "part_0002", "schema.txt"]
        );
        assert_eq!(scanned_ids(&reopened), vec![vec![1], vec![2], vec![3]]);
    }

    /// `PartWriter` stages into `<part>.tmp` next to the finished part, so a
    /// crashed insert leaves one inside the table directory. It must neither
    /// fail `list_parts` nor count towards `next_part_id`.
    #[test]
    fn open_ignores_a_leftover_staging_directory() {
        let (_root, dir, mut table) = sample_table();
        table.insert(&[sample_block(&[1], &["a"], &[1.0])]).unwrap();
        fs::create_dir_all(dir.join("part_0001.tmp")).unwrap();
        fs::write(dir.join("part_0001.tmp/id.bin"), b"debris").unwrap();
        drop(table);

        let mut reopened = Table::open(dir.clone()).unwrap();
        reopened
            .insert(&[sample_block(&[2], &["b"], &[2.0])])
            .unwrap();

        assert_eq!(scanned_ids(&reopened), vec![vec![1], vec![2]]);
    }

    #[test]
    fn open_rejects_an_unexpected_entry_in_the_table_directory() {
        let (_root, dir, _table) = sample_table();
        fs::write(dir.join("notes.txt"), b"hi").unwrap();

        let e = open_error(dir);

        assert!(matches!(e, StorageError::Corrupt(_)), "{e:?}");
        assert_eq!(
            e.to_string(),
            "corrupt: unexpected entry 'notes.txt' in table directory"
        );
    }

    /// `list_parts` round-trips the parsed id back through `part_dir_name`, so
    /// an unpadded name is rejected instead of aliasing onto `part_0001`.
    #[test]
    fn open_rejects_a_non_canonical_part_directory_name() {
        let (_root, dir, _table) = sample_table();
        fs::create_dir_all(dir.join("part_1")).unwrap();

        assert_eq!(
            open_err(dir),
            "corrupt: unexpected entry 'part_1' in table directory"
        );
    }

    // ---- insert --------------------------------------------------------

    #[test]
    fn a_single_block_round_trips_through_one_part() {
        let (_root, dir, mut table) = sample_table();

        table
            .insert(&[sample_block(&[1, 2, 3], &["a", "b", "c"], &[1.5, 2.5, 3.5])])
            .unwrap();

        assert_eq!(entries(&dir), vec!["part_0000", "schema.txt"]);
        let blocks = scan_all(&table, &["id", "name", "score"]).unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(i64s(&blocks[0], "id"), vec![1, 2, 3]);
        assert_eq!(strs(&blocks[0], "name"), vec!["a", "b", "c"]);
        assert_eq!(f64s(&blocks[0], "score"), vec![1.5, 2.5, 3.5]);
    }

    /// One `insert` call is one part, however many blocks it carries — the
    /// blocks survive as separate chunks inside it, in write order.
    #[test]
    fn blocks_from_one_insert_share_a_part_and_keep_their_order() {
        let (_root, dir, mut table) = sample_table();

        table
            .insert(&[
                sample_block(&[1, 2], &["a", "b"], &[1.0, 2.0]),
                sample_block(&[3], &["c"], &[3.0]),
                sample_block(&[4, 5], &["d", "e"], &[4.0, 5.0]),
            ])
            .unwrap();

        assert_eq!(entries(&dir), vec!["part_0000", "schema.txt"]);
        let blocks = scan_all(&table, &["id"]).unwrap();
        assert_eq!(row_counts(&blocks), vec![2, 1, 2]);
        assert_eq!(
            blocks.iter().map(|b| i64s(b, "id")).collect::<Vec<_>>(),
            vec![vec![1, 2], vec![3], vec![4, 5]]
        );
    }

    #[test]
    fn each_insert_call_writes_its_own_part() {
        let (_root, dir, mut table) = sample_table();

        table.insert(&[sample_block(&[1], &["a"], &[1.0])]).unwrap();
        table.insert(&[sample_block(&[2], &["b"], &[2.0])]).unwrap();
        table.insert(&[sample_block(&[3], &["c"], &[3.0])]).unwrap();

        assert_eq!(
            entries(&dir),
            vec!["part_0000", "part_0001", "part_0002", "schema.txt"]
        );
        assert_eq!(scanned_ids(&table), vec![vec![1], vec![2], vec![3]]);
    }

    #[test]
    fn inserting_an_empty_slice_writes_no_part() {
        let (_root, dir, mut table) = sample_table();

        table.insert(&[]).unwrap();

        assert_eq!(entries(&dir), vec!["schema.txt"]);
        assert!(scan_all(&table, &["id"]).unwrap().is_empty());
    }

    /// A dropped insert must not burn a part id either, or the directory would
    /// grow gaps that `next_part_id` silently steps over.
    #[test]
    fn inserting_only_zero_row_blocks_writes_no_part_and_consumes_no_id() {
        let (_root, dir, mut table) = sample_table();

        table.insert(&[empty_block(), empty_block()]).unwrap();

        assert_eq!(entries(&dir), vec!["schema.txt"]);

        table.insert(&[sample_block(&[1], &["a"], &[1.0])]).unwrap();

        assert_eq!(entries(&dir), vec!["part_0000", "schema.txt"]);
    }

    /// Zero-row blocks are dropped before the writer sees them, so the part
    /// holds no zero-length chunks — the scan returns one block, not three.
    #[test]
    fn zero_row_blocks_are_dropped_from_a_mixed_insert() {
        let (_root, _dir, mut table) = sample_table();

        table
            .insert(&[
                empty_block(),
                sample_block(&[1, 2], &["a", "b"], &[1.0, 2.0]),
                empty_block(),
            ])
            .unwrap();

        let blocks = scan_all(&table, &["id"]).unwrap();
        assert_eq!(row_counts(&blocks), vec![2]);
        assert_eq!(i64s(&blocks[0], "id"), vec![1, 2]);
    }

    /// `PartWriter` publishes by renaming its staging directory, so a finished
    /// insert must leave no `.tmp` behind inside the table.
    #[test]
    fn a_finished_insert_leaves_no_staging_directory() {
        let (_root, dir, mut table) = sample_table();

        table.insert(&[sample_block(&[1], &["a"], &[1.0])]).unwrap();

        assert_eq!(entries(&dir), vec!["part_0000", "schema.txt"]);
    }

    #[test]
    #[should_panic(expected = "block has different number of columns than table")]
    fn insert_panics_on_too_few_columns() {
        let (_root, _dir, mut table) = sample_table();
        let block = Block::new(
            vec![
                ("id".to_string(), Column::Int64(vec![1])),
                (
                    "name".to_string(),
                    Column::String(StringColumn::new_with_values(&["a"])),
                ),
            ],
            1,
        );

        let _ = table.insert(&[block]);
    }

    #[test]
    #[should_panic(expected = "block has different number of columns than table")]
    fn insert_panics_on_too_many_columns() {
        let (_root, _dir, mut table) = created(
            Schema::new(vec![
                ("id".to_string(), DataType::Int64),
                ("name".to_string(), DataType::String),
            ])
            .unwrap(),
        );
        let block = Block::new(
            vec![
                ("id".to_string(), Column::Int64(vec![1])),
                (
                    "name".to_string(),
                    Column::String(StringColumn::new_with_values(&["a"])),
                ),
                ("extra".to_string(), Column::Float64(vec![1.0])),
            ],
            1,
        );

        let _ = table.insert(&[block]);
    }

    #[test]
    #[should_panic(expected = "insert: column 0 expected ('id', Int64), got ('wrong', Int64)")]
    fn insert_panics_on_a_column_name_mismatch_at_index_zero() {
        let (_root, _dir, mut table) = sample_table();
        let block = Block::new(
            vec![
                ("wrong".to_string(), Column::Int64(vec![1])),
                (
                    "name".to_string(),
                    Column::String(StringColumn::new_with_values(&["a"])),
                ),
                ("score".to_string(), Column::Float64(vec![1.0])),
            ],
            1,
        );

        let _ = table.insert(&[block]);
    }

    #[test]
    #[should_panic(expected = "insert: column 2 expected ('score', Float64), got ('bad', Float64)")]
    fn insert_panics_on_a_column_name_mismatch_at_a_later_index() {
        let (_root, _dir, mut table) = sample_table();
        let block = Block::new(
            vec![
                ("id".to_string(), Column::Int64(vec![1])),
                (
                    "name".to_string(),
                    Column::String(StringColumn::new_with_values(&["a"])),
                ),
                ("bad".to_string(), Column::Float64(vec![1.0])),
            ],
            1,
        );

        let _ = table.insert(&[block]);
    }

    #[test]
    #[should_panic(expected = "insert: column 0 expected ('id', Int64), got ('id', Float64)")]
    fn insert_panics_on_a_type_mismatch_with_a_matching_name() {
        let (_root, _dir, mut table) = sample_table();
        let block = Block::new(
            vec![
                ("id".to_string(), Column::Float64(vec![1.0])),
                (
                    "name".to_string(),
                    Column::String(StringColumn::new_with_values(&["a"])),
                ),
                ("score".to_string(), Column::Float64(vec![1.0])),
            ],
            1,
        );

        let _ = table.insert(&[block]);
    }

    /// Columns are matched positionally, so the right set in the wrong order is
    /// rejected rather than written to the wrong column files.
    #[test]
    #[should_panic(expected = "insert: column 0 expected ('id', Int64), got ('name', String)")]
    fn insert_panics_on_reordered_columns() {
        let (_root, _dir, mut table) = sample_table();
        let block = Block::new(
            vec![
                (
                    "name".to_string(),
                    Column::String(StringColumn::new_with_values(&["a"])),
                ),
                ("id".to_string(), Column::Int64(vec![1])),
                ("score".to_string(), Column::Float64(vec![1.0])),
            ],
            1,
        );

        let _ = table.insert(&[block]);
    }

    /// Shape validation runs before the zero-row filter, so a malformed empty
    /// block is still rejected rather than quietly discarded.
    #[test]
    #[should_panic(expected = "block has different number of columns than table")]
    fn insert_panics_on_a_zero_row_block_with_the_wrong_column_count() {
        let (_root, _dir, mut table) = sample_table();

        let _ = table.insert(&[Block::new(vec![], 0)]);
    }

    /// Every block in the slice is validated before the writer is opened, so a
    /// bad block at the end aborts the whole insert instead of half-writing a
    /// part. `catch_unwind` is the only way to inspect the directory after the
    /// panic.
    #[test]
    fn nothing_is_written_when_a_later_block_in_the_slice_is_invalid() {
        let (_root, dir, mut table) = sample_table();
        let good = sample_block(&[1], &["a"], &[1.0]);
        let bad = Block::new(vec![("id".to_string(), Column::Int64(vec![2]))], 1);

        let outcome = catch_unwind(AssertUnwindSafe(|| table.insert(&[good, bad])));

        assert!(outcome.is_err(), "expected the invalid block to panic");
        assert_eq!(entries(&dir), vec!["schema.txt"]);
    }

    #[test]
    fn a_large_block_round_trips() {
        let (_root, _dir, mut table) =
            created(Schema::new(vec![("id".to_string(), DataType::Int64)]).unwrap());
        let ids: Vec<i64> = (0..1000).collect();

        table
            .insert(&[Block::new(
                vec![("id".to_string(), Column::Int64(ids.clone()))],
                1000,
            )])
            .unwrap();

        let blocks = scan_all(&table, &["id"]).unwrap();
        assert_eq!(row_counts(&blocks), vec![1000]);
        assert_eq!(i64s(&blocks[0], "id"), ids);
    }

    // ---- scan ----------------------------------------------------------

    #[test]
    fn scanning_an_empty_table_yields_no_blocks() {
        let (_root, _dir, table) = sample_table();

        assert!(scan_all(&table, &["id"]).unwrap().is_empty());
    }

    #[test]
    fn scan_projects_only_the_requested_columns() {
        let (_root, _dir, mut table) = sample_table();
        table.insert(&[sample_block(&[1], &["a"], &[1.0])]).unwrap();

        let blocks = scan_all(&table, &["id"]).unwrap();

        assert_eq!(column_names(&blocks[0]), vec!["id"]);
        assert!(blocks[0].column("name").is_none());
    }

    /// `PartReader::open` sorts its readers by schema index, so the projection
    /// order is not the block order. Callers that index positionally depend on
    /// this.
    #[test]
    fn scan_returns_projected_columns_in_schema_order() {
        let (_root, _dir, mut table) = sample_table();
        table.insert(&[sample_block(&[1], &["a"], &[1.0])]).unwrap();

        let blocks = scan_all(&table, &["score", "id"]).unwrap();

        assert_eq!(column_names(&blocks[0]), vec!["id", "score"]);
    }

    #[test]
    fn scan_can_be_called_repeatedly() {
        let (_root, _dir, mut table) = sample_table();
        table
            .insert(&[sample_block(&[1, 2], &["a", "b"], &[1.0, 2.0])])
            .unwrap();
        table.insert(&[sample_block(&[3], &["c"], &[3.0])]).unwrap();

        assert_eq!(scanned_ids(&table), scanned_ids(&table));
        assert_eq!(scanned_ids(&table), vec![vec![1, 2], vec![3]]);
    }

    #[test]
    fn scan_yields_blocks_in_part_order_then_write_order() {
        let (_root, _dir, mut table) = sample_table();
        table
            .insert(&[
                sample_block(&[1], &["a"], &[1.0]),
                sample_block(&[2, 3], &["b", "c"], &[2.0, 3.0]),
            ])
            .unwrap();
        table
            .insert(&[sample_block(&[4, 5, 6], &["d", "e", "f"], &[4.0, 5.0, 6.0])])
            .unwrap();

        assert_eq!(
            scanned_ids(&table),
            vec![vec![1], vec![2, 3], vec![4, 5, 6]]
        );
    }

    /// The projection is resolved lazily, part by part — `scan` itself cannot
    /// fail on an unknown column, only the first step of the iterator can.
    #[test]
    fn an_unknown_column_is_reported_on_the_first_step_not_by_scan() {
        let (_root, _dir, mut table) = sample_table();
        table.insert(&[sample_block(&[1], &["a"], &[1.0])]).unwrap();

        let columns = ["nope"];
        let mut iter = table.scan(&columns).expect("scan itself must not fail");
        let e = iter.next().expect("expected an error item").unwrap_err();

        assert!(
            matches!(e, StorageError::ColumnNotFound(ref n) if n == "nope"),
            "{e:?}"
        );
        assert_eq!(e.to_string(), "column 'nope' not found");
    }

    /// The flip side of the laziness above: with no parts to open, a projection
    /// naming a column the table does not have succeeds and yields nothing.
    /// Documents current behavior — `scan` never checks the projection against
    /// `self.schema`.
    #[test]
    fn an_unknown_column_on_an_empty_table_is_not_reported() {
        let (_root, _dir, table) = sample_table();

        assert!(scan_all(&table, &["nope"]).unwrap().is_empty());
    }

    #[test]
    fn scan_propagates_a_corrupt_part() {
        let (_root, dir, mut table) = sample_table();
        table.insert(&[sample_block(&[1], &["a"], &[1.0])]).unwrap();
        fs::remove_file(dir.join("part_0000/id.bin")).unwrap();

        let e = scan_error(&table, &["id"]);

        assert!(matches!(e, StorageError::Corrupt(_)), "{e:?}");
        assert_eq!(e.to_string(), "corrupt: column file 'id.bin' missing");
    }

    /// `BlockIter` fuses itself on error by jumping `pos` past the last part, so
    /// a broken part ends the scan instead of skipping to the next one.
    #[test]
    fn the_scan_stops_after_the_first_error() {
        let (_root, dir, mut table) = sample_table();
        table.insert(&[sample_block(&[1], &["a"], &[1.0])]).unwrap();
        table.insert(&[sample_block(&[2], &["b"], &[2.0])]).unwrap();
        table.insert(&[sample_block(&[3], &["c"], &[3.0])]).unwrap();
        fs::remove_file(dir.join("part_0001/id.bin")).unwrap();

        let columns = ["id"];
        let mut iter = table.scan(&columns).unwrap();

        assert_eq!(i64s(&iter.next().unwrap().unwrap(), "id"), vec![1]);
        assert_eq!(
            iter.next().unwrap().unwrap_err().to_string(),
            "corrupt: column file 'id.bin' missing"
        );
        assert!(
            iter.next().is_none(),
            "the iterator must fuse instead of resuming at part_0002"
        );
    }

    /// `list_parts` runs eagerly inside `scan`, so a stray entry fails before
    /// any block is produced.
    #[test]
    fn scan_rejects_an_unexpected_entry_in_the_table_directory() {
        let (_root, dir, mut table) = sample_table();
        table.insert(&[sample_block(&[1], &["a"], &[1.0])]).unwrap();
        fs::write(dir.join("notes.txt"), b"hi").unwrap();

        assert_eq!(
            scan_err(&table, &["id"]),
            "corrupt: unexpected entry 'notes.txt' in table directory"
        );
    }

    #[test]
    fn scan_ignores_a_leftover_staging_directory() {
        let (_root, dir, mut table) = sample_table();
        table.insert(&[sample_block(&[1], &["a"], &[1.0])]).unwrap();
        fs::create_dir_all(dir.join("part_0001.tmp")).unwrap();

        assert_eq!(scanned_ids(&table), vec![vec![1]]);
    }

    /// Inherited from `PartReader::open`, which asserts rather than errors — and
    /// only once there is a part to open, so the panic is deferred to the first
    /// step of the iterator.
    #[test]
    #[should_panic(expected = "empty column projection not supported yet")]
    fn an_empty_projection_panics_once_a_part_exists() {
        let (_root, _dir, mut table) = sample_table();
        table.insert(&[sample_block(&[1], &["a"], &[1.0])]).unwrap();

        let columns: [&str; 0] = [];
        let _ = scan_all(&table, &columns);
    }

    #[test]
    #[should_panic(expected = "duplicate column name: id")]
    fn a_duplicated_projection_column_panics_once_a_part_exists() {
        let (_root, _dir, mut table) = sample_table();
        table.insert(&[sample_block(&[1], &["a"], &[1.0])]).unwrap();

        let _ = scan_all(&table, &["id", "id"]);
    }

    // ---- schema --------------------------------------------------------

    #[test]
    fn schema_is_unaffected_by_inserts() {
        let schema = sample_schema();
        let (_root, _dir, mut table) = created(schema.clone());

        table
            .insert(&[sample_block(&[1, 2], &["a", "b"], &[1.0, 2.0])])
            .unwrap();
        table.insert(&[sample_block(&[3], &["c"], &[3.0])]).unwrap();

        assert_eq!(*table.schema(), schema);
    }
}
