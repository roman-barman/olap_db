use crate::column_io::{read_f64_chunk, read_i64_chunk, read_str_chunk};
use crate::schema::Schema;
use crate::storage_error::StorageError;
use crate::{Block, Column, DataType};
use std::fs;
use std::fs::File;
use std::io::{BufReader, ErrorKind};
use std::path::Path;

pub(crate) struct PartReader {
    schema: Schema,
    num_rows: usize,
    rows_read: usize,
    readers: Vec<(usize, ColumnReaders)>,
}

enum ColumnReaders {
    Single(BufReader<File>),
    Pair {
        data: BufReader<File>,
        offsets: BufReader<File>,
    },
}

impl PartReader {
    pub(crate) fn open(dir: &Path, columns: &[&str]) -> Result<PartReader, StorageError> {
        assert!(
            !columns.is_empty(),
            "PartReader: empty column projection not supported yet — see backlog (count via mask)"
        );

        for i in 1..columns.len() {
            let name = &columns[i];
            assert!(
                !columns[..i].iter().any(|n| n == name),
                "duplicate column name: {name}"
            );
        }

        if !dir.is_dir() {
            return Err(StorageError::NotFound(dir.to_path_buf()));
        }

        let text = match fs::read_to_string(dir.join("schema.txt")) {
            Ok(text) => text,
            Err(err) if err.kind() == ErrorKind::NotFound => {
                return Err(StorageError::Corrupt("schema.txt missing".into()));
            }
            Err(err) => return Err(err.into()),
        };

        let (schema, num_rows) = parse_schema(&text)?;

        let mut readers = columns
            .iter()
            .map(|name| {
                let idx = schema
                    .iter()
                    .position(|(n, _)| n == name)
                    .ok_or_else(|| StorageError::ColumnNotFound(name.to_string()))?;
                let reader = match schema[idx].1 {
                    DataType::Int64 | DataType::Float64 => {
                        ColumnReaders::Single(open_file(dir, &format!("{name}.bin"))?)
                    }
                    DataType::String => ColumnReaders::Pair {
                        data: open_file(dir, &format!("{name}.data.bin"))?,
                        offsets: open_file(dir, &format!("{name}.offsets.bin"))?,
                    },
                };

                Ok((idx, reader))
            })
            .collect::<Result<Vec<_>, StorageError>>()?;
        readers.sort_by_key(|(idx, _)| *idx);

        Ok(PartReader {
            schema,
            num_rows,
            rows_read: 0,
            readers,
        })
    }

    pub(crate) fn next_block(&mut self) -> Result<Option<Block>, StorageError> {
        let mut columns: Vec<(String, Column)> = Vec::with_capacity(self.readers.len());
        let mut len: Option<usize> = None;
        let mut ended = 0usize;

        for (idx, reader) in self.readers.iter_mut() {
            let (name, dt) = &self.schema[*idx];
            let column: Option<Column> = match (dt, reader) {
                (DataType::Int64, ColumnReaders::Single(r)) => {
                    read_i64_chunk(r)?.map(Column::Int64)
                }
                (DataType::Float64, ColumnReaders::Single(r)) => {
                    read_f64_chunk(r)?.map(Column::Float64)
                }
                (DataType::String, ColumnReaders::Pair { data, offsets }) => {
                    read_str_chunk(data, offsets)?.map(Column::String)
                }
                _ => unreachable!(
                    "column '{name}': reader shape mismatch with {dt:?} — broken by open"
                ),
            };

            match column {
                None => ended += 1,
                Some(c) => {
                    match len {
                        None => len = Some(c.len()),
                        Some(l) if l != c.len() => {
                            return Err(StorageError::Corrupt(format!(
                                "column files out of sync: '{name}' chunk has {} rows, expected {l}",
                                c.len()
                            )));
                        }
                        _ => {}
                    }
                    columns.push((name.clone(), c));
                }
            }
        }

        match (ended, columns.len()) {
            (0, _) => {
                let len = len.expect("non-empty readers");
                self.rows_read += len;
                Ok(Some(Block::new(columns, len)))
            }
            (e, 0) if e == self.readers.len() => {
                if self.rows_read != self.num_rows {
                    return Err(StorageError::Corrupt(format!(
                        "part truncated: schema declares {} rows, files contain {}",
                        self.num_rows, self.rows_read
                    )));
                }
                Ok(None)
            }
            _ => Err(StorageError::Corrupt(
                "column files out of sync: some ended, some continue".into(),
            )),
        }
    }
}

fn open_file(dir: &Path, name: &str) -> Result<BufReader<File>, StorageError> {
    match File::open(dir.join(name)) {
        Ok(file) => Ok(BufReader::new(file)),
        Err(e) if e.kind() == ErrorKind::NotFound => Err(StorageError::Corrupt(format!(
            "column file '{name}' missing"
        ))),
        Err(err) => Err(err.into()),
    }
}

fn parse_schema(text: &str) -> Result<(Schema, usize), StorageError> {
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

    let num_rows = lines
        .next()
        .ok_or_else(|| StorageError::Corrupt("missing num rows line".into()))?
        .strip_prefix("num_rows=")
        .ok_or_else(|| StorageError::Corrupt("invalid num rows line".into()))?
        .parse::<usize>()
        .map_err(|_| StorageError::Corrupt("invalid num rows".into()))?;

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

    let schema = Schema::new(schema).map_err(|error| StorageError::Corrupt(error.to_string()))?;

    Ok((schema, num_rows))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::{Codec, write_block};
    use crate::column_io::{write_f64_chunk, write_i64_chunk, write_str_chunk};
    use crate::part_writer::PartWriter;
    use crate::string_column::StringColumn;
    use crate::test_fixture::{column_names, part_dir, sample_schema, staging_of};
    use crate::{Block, Column};
    use std::fs::OpenOptions;
    use std::io::Write;
    use std::path::PathBuf;
    use tempfile::TempDir;

    /// Every failure path returns `StorageError`; tests assert on the rendered message.
    fn err(text: &str) -> String {
        parse_schema(text).unwrap_err().to_string()
    }

    fn col(name: &str, dt: DataType) -> (String, DataType) {
        (name.to_string(), dt)
    }

    // ---- `open` fixtures -----------------------------------------------

    fn sample_block(rows: usize) -> Block {
        let mut names = StringColumn::new();
        for i in 0..rows {
            names.push(&format!("n{i}"));
        }
        Block::new(
            vec![
                ("id".to_string(), Column::Int64((0..rows as i64).collect())),
                ("name".to_string(), Column::String(names)),
                (
                    "score".to_string(),
                    Column::Float64((0..rows).map(|i| i as f64).collect()),
                ),
            ],
            rows,
        )
    }

    /// A finished part holding `rows` rows of `sample_schema` data. Going through
    /// the real `PartWriter` is the point: the `{name}.bin` / `{name}.data.bin` /
    /// `{name}.offsets.bin` convention is spelled out independently in
    /// `PartWriter::new` and in `open`, and nothing but a round trip holds the two
    /// spellings together.
    fn written_part(rows: usize) -> (TempDir, PathBuf) {
        let (root, dir) = part_dir();

        let mut writer = PartWriter::new(dir.clone(), sample_schema(), Codec::Lz4).unwrap();
        if rows > 0 {
            writer.write_block(&sample_block(rows)).unwrap();
        }
        writer.finish().unwrap();

        (root, dir)
    }

    /// A part directory containing nothing but a hand-written `schema.txt` — for
    /// the corrupt shapes `PartWriter` cannot produce.
    fn part_with_schema_text(text: &str) -> (TempDir, PathBuf) {
        let (root, dir) = part_dir();
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("schema.txt"), text).unwrap();
        (root, dir)
    }

    /// The error from an `open` that must fail. `unwrap_err` is unavailable —
    /// `PartReader` holds open file handles and deliberately has no `Debug`.
    fn open_error(dir: &Path, columns: &[&str]) -> StorageError {
        match PartReader::open(dir, columns) {
            Ok(_) => panic!("expected opening {dir:?} with {columns:?} to fail"),
            Err(e) => e,
        }
    }

    /// Rendered message of a failed `open`, mirroring `err` above.
    fn open_err(dir: &Path, columns: &[&str]) -> String {
        open_error(dir, columns).to_string()
    }

    /// The schema index each opened reader is bound to, in `readers` order.
    fn reader_indices(reader: &PartReader) -> Vec<usize> {
        reader.readers.iter().map(|(i, _)| *i).collect()
    }

    // ---- happy path ----------------------------------------------------

    /// This literal is byte-identical to the one `PartWriter::finish` produces
    /// (see `part_writer.rs::schema_file_has_exact_expected_contents`). It is the
    /// writer/reader contract in a single line: change the writer's format and
    /// this test fails.
    #[test]
    fn parses_the_schema_part_writer_emits() {
        let (schema, num_rows) = parse_schema(
            "version=1\nnum_rows=3\ncolumn=id:Int64\ncolumn=name:String\ncolumn=score:Float64\n",
        )
        .unwrap();

        assert_eq!(
            schema,
            Schema::new(vec![
                col("id", DataType::Int64),
                col("name", DataType::String),
                col("score", DataType::Float64),
            ])
            .unwrap()
        );
        assert_eq!(num_rows, 3);
    }

    /// Columns are matched positionally against the column files, so declaration
    /// order is data, not presentation.
    #[test]
    fn preserves_column_declaration_order() {
        let (schema, _) = parse_schema(
            "version=1\nnum_rows=0\ncolumn=zulu:Float64\ncolumn=alpha:String\ncolumn=mike:Int64\n",
        )
        .unwrap();

        assert_eq!(
            schema,
            Schema::new(vec![
                col("zulu", DataType::Float64),
                col("alpha", DataType::String),
                col("mike", DataType::Int64),
            ])
            .unwrap()
        );
    }

    #[test]
    fn parses_all_three_data_type_tokens() {
        let (schema, _) = parse_schema(
            "version=1\nnum_rows=0\ncolumn=a:Int64\ncolumn=b:Float64\ncolumn=c:String\n",
        )
        .unwrap();

        assert_eq!(
            schema.iter().map(|(_, dt)| *dt).collect::<Vec<_>>(),
            vec![DataType::Int64, DataType::Float64, DataType::String]
        );
    }

    /// A part written by a `PartWriter` that never received a block: columns exist,
    /// rows do not.
    #[test]
    fn zero_rows_with_columns_is_valid() {
        let (schema, num_rows) = parse_schema("version=1\nnum_rows=0\ncolumn=id:Int64\n").unwrap();

        assert_eq!(
            schema,
            Schema::new(vec![col("id", DataType::Int64)]).unwrap()
        );
        assert_eq!(num_rows, 0);
    }

    #[test]
    fn final_newline_is_optional() {
        let (schema, num_rows) = parse_schema("version=1\nnum_rows=1\ncolumn=id:Int64").unwrap();

        assert_eq!(
            schema,
            Schema::new(vec![col("id", DataType::Int64)]).unwrap()
        );
        assert_eq!(num_rows, 1);
    }

    #[test]
    fn parses_num_rows_at_the_usize_ceiling() {
        let text = format!("version=1\nnum_rows={}\ncolumn=id:Int64\n", usize::MAX);

        let (_, num_rows) = parse_schema(&text).unwrap();

        assert_eq!(num_rows, usize::MAX);
    }

    // ---- version line --------------------------------------------------

    #[test]
    fn rejects_empty_input() {
        assert!(err("").contains("empty schema"));
    }

    #[test]
    fn rejects_version_line_without_prefix() {
        assert!(err("v=1\nnum_rows=0\n").contains("invalid version line"));
    }

    #[test]
    fn rejects_a_leading_blank_line() {
        assert!(err("\nversion=1\nnum_rows=0\n").contains("invalid version line"));
    }

    #[test]
    fn rejects_unsupported_version() {
        let e = parse_schema("version=2\nnum_rows=0\n").unwrap_err();

        assert_eq!(e.to_string(), "unsupported format version 2, expected 1");
        assert!(matches!(
            e,
            StorageError::UnsupportedVersion { ref found, expected: 1 } if found == "2"
        ));
    }

    #[test]
    fn rejects_an_empty_version_value() {
        let e = parse_schema("version=\nnum_rows=0\n").unwrap_err();

        assert!(matches!(
            e,
            StorageError::UnsupportedVersion { ref found, expected: 1 } if found.is_empty()
        ));
    }

    /// Nothing is trimmed: a stray trailing space makes the whole token the
    /// version, so a whitespace problem surfaces as a version mismatch.
    /// Documents current behavior.
    #[test]
    fn version_value_is_compared_literally() {
        let e = parse_schema("version=1 \nnum_rows=0\n").unwrap_err();

        assert!(matches!(
            e,
            StorageError::UnsupportedVersion { ref found, expected: 1 } if found == "1 "
        ));
    }

    // ---- num_rows line -------------------------------------------------

    #[test]
    fn rejects_input_with_only_a_version_line() {
        assert!(err("version=1\n").contains("missing num rows line"));
    }

    #[test]
    fn rejects_num_rows_line_without_prefix() {
        assert!(err("version=1\nrows=3\n").contains("invalid num rows line"));
    }

    #[test]
    fn rejects_non_numeric_num_rows() {
        assert!(err("version=1\nnum_rows=abc\n").contains("invalid num rows"));
    }

    #[test]
    fn rejects_negative_num_rows() {
        assert!(err("version=1\nnum_rows=-1\n").contains("invalid num rows"));
    }

    /// Overflow and outright garbage collapse to the same error — `parse::<usize>`
    /// failure is not classified further.
    #[test]
    fn rejects_num_rows_overflowing_usize() {
        assert!(
            err("version=1\nnum_rows=99999999999999999999999999\n").contains("invalid num rows")
        );
    }

    // ---- column lines --------------------------------------------------

    #[test]
    fn rejects_column_line_without_prefix() {
        assert!(err("version=1\nnum_rows=1\nid:Int64\n").contains("line 3: expected column=..."));
    }

    #[test]
    fn rejects_column_line_without_a_colon() {
        assert!(
            err("version=1\nnum_rows=1\ncolumn=id\n")
                .contains("line 3: expected column=<name>:<type>")
        );
    }

    #[test]
    fn rejects_an_unknown_data_type() {
        assert!(
            err("version=1\nnum_rows=1\ncolumn=id:Int32\n")
                .contains("line 3: invalid data type 'Int32'")
        );
    }

    /// Column lines start at line 3, so the reported number is `index + 3`. An
    /// off-by-one here would point a reader at the wrong line of a corrupt file.
    #[test]
    fn reports_the_line_number_of_the_offending_column() {
        let text = "version=1\nnum_rows=1\n\
                    column=a:Int64\ncolumn=b:Int64\ncolumn=c:Int64\ncolumn=d:Nope\n";

        assert!(err(text).contains("line 6: invalid data type 'Nope'"));
    }

    #[test]
    fn rejects_an_empty_type_token() {
        assert!(err("version=1\nnum_rows=1\ncolumn=id:\n").contains("invalid data type ''"));
    }

    /// `str::lines` strips the trailing `\r`, so a `schema.txt` written with CRLF
    /// endings parses identically to one written with LF.
    #[test]
    fn accepts_crlf_line_endings() {
        let (schema, num_rows) =
            parse_schema("version=1\r\nnum_rows=2\r\ncolumn=id:Int64\r\n").unwrap();

        assert_eq!(
            schema,
            Schema::new(vec![col("id", DataType::Int64)]).unwrap()
        );
        assert_eq!(num_rows, 2);
    }

    // ---- pinned current behavior ---------------------------------------

    /// A blank line anywhere after the header is treated as a malformed column
    /// line, including a doubled final newline. Documents current behavior.
    #[test]
    fn rejects_a_trailing_blank_line() {
        assert!(
            err("version=1\nnum_rows=1\ncolumn=id:Int64\n\n")
                .contains("line 4: expected column=...")
        );
    }

    /// `split_once(':')` cuts at the *first* colon, so a name containing one is
    /// truncated and the remainder is read as the type. Such a part is writable
    /// but not readable. Documents current behavior.
    #[test]
    fn rejects_a_column_name_containing_a_colon() {
        assert!(
            err("version=1\nnum_rows=1\ncolumn=a:b:Int64\n")
                .contains("line 3: invalid data type 'b:Int64'")
        );
    }

    /// Only the leading `column=` is stripped, so later `=` characters belong to
    /// the name. Documents current behavior.
    #[test]
    fn accepts_a_column_name_containing_an_equals_sign() {
        let (schema, _) = parse_schema("version=1\nnum_rows=1\ncolumn=a=b:Int64\n").unwrap();

        assert_eq!(
            schema,
            Schema::new(vec![col("a=b", DataType::Int64)]).unwrap()
        );
    }

    // ---- cross-line validation -----------------------------------------
    #[test]
    fn rejects_duplicate_column_names() {
        assert!(
            err("version=1\nnum_rows=1\ncolumn=id:Int64\ncolumn=id:Int64\n")
                .contains("duplicate column name 'id'")
        );
    }

    /// The check scans every earlier name, not just the preceding one, so
    /// duplicates separated by other columns are still caught.
    #[test]
    fn detects_non_adjacent_duplicate_column_names() {
        let text =
            "version=1\nnum_rows=1\ncolumn=id:Int64\ncolumn=name:String\ncolumn=id:Float64\n";

        assert!(err(text).contains("duplicate column name 'id'"));
    }

    /// Names differing only in case are distinct — they map to distinct files on a
    /// case-sensitive filesystem. Documents current behavior.
    #[test]
    fn duplicate_detection_is_case_sensitive() {
        let (schema, _) =
            parse_schema("version=1\nnum_rows=1\ncolumn=id:Int64\ncolumn=ID:Int64\n").unwrap();

        assert_eq!(
            schema,
            Schema::new(vec![col("id", DataType::Int64), col("ID", DataType::Int64)]).unwrap()
        );
    }

    /// Per-line parsing short-circuits before the duplicate scan runs, so a file
    /// with both faults reports the type error. Pins the error precedence.
    #[test]
    fn an_invalid_type_is_reported_before_a_duplicate_name() {
        let text = "version=1\nnum_rows=1\ncolumn=id:Nope\ncolumn=id:Int64\n";

        assert!(err(text).contains("line 3: invalid data type 'Nope'"));
    }

    // Note: the `"missing version line"` branch is unreachable and therefore
    // untested — `text.is_empty()` is rejected first, and `str::lines()` on any
    // non-empty string always yields at least one item.

    // ---- open: round trip ----------------------------------------------

    /// The end-to-end contract: a part `PartWriter` produced is a part
    /// `PartReader` opens. If either side renames a column file, this fails and
    /// nothing else does.
    #[test]
    fn open_reads_a_part_written_by_part_writer() {
        let (_root, dir) = written_part(3);

        let reader = PartReader::open(&dir, &["id", "name", "score"]).unwrap();

        assert_eq!(reader.schema, sample_schema());
        assert_eq!(reader.num_rows, 3);
        assert_eq!(reader.rows_read, 0);
        assert_eq!(reader_indices(&reader), vec![0, 1, 2]);
    }

    /// `Float64` shares the single-file branch with `Int64`; only `String` gets a
    /// pair. A reader shape that disagreed with the writer's file layout would
    /// decode the wrong stream.
    #[test]
    fn numeric_columns_get_one_reader_and_string_columns_a_pair() {
        let (_root, dir) = written_part(1);

        let reader = PartReader::open(&dir, &["id", "name", "score"]).unwrap();

        assert!(matches!(reader.readers[0].1, ColumnReaders::Single(_)));
        assert!(matches!(reader.readers[1].1, ColumnReaders::Pair { .. }));
        assert!(matches!(reader.readers[2].1, ColumnReaders::Single(_)));
    }

    #[test]
    fn opens_a_single_column_projection() {
        let (_root, dir) = written_part(2);

        let reader = PartReader::open(&dir, &["name"]).unwrap();

        assert_eq!(reader_indices(&reader), vec![1]);
    }

    /// `readers` holds indices into the *whole* part schema, not into the
    /// projection, so the schema must be kept whole. A future `next_block` that
    /// indexed a projected-down schema would read the wrong column's type.
    #[test]
    fn schema_field_holds_the_whole_part_schema_not_the_projection() {
        let (_root, dir) = written_part(1);

        let reader = PartReader::open(&dir, &["score"]).unwrap();

        assert_eq!(reader.schema, sample_schema());
        assert_eq!(reader_indices(&reader), vec![2]);
    }

    /// Readers come back in schema order whatever order they were asked for. This
    /// is the one thing about `open` a caller is most likely to assume the other
    /// way round.
    #[test]
    fn readers_are_ordered_by_schema_index_not_by_projection_order() {
        let (_root, dir) = written_part(1);

        let reader = PartReader::open(&dir, &["score", "id"]).unwrap();

        assert_eq!(reader_indices(&reader), vec![0, 2]);
    }

    /// A part whose writer never received a block: the column files exist but are
    /// empty, and that is a valid part, not a corrupt one.
    #[test]
    fn opens_a_part_with_zero_rows() {
        let (_root, dir) = written_part(0);

        let reader = PartReader::open(&dir, &["id", "name", "score"]).unwrap();

        assert_eq!(reader.num_rows, 0);
        assert_eq!(reader.readers.len(), 3);
    }

    /// Opening is read-only, so overlapping readers over one part are independent.
    #[test]
    fn two_readers_can_open_the_same_part_at_once() {
        let (_root, dir) = written_part(2);

        let first = PartReader::open(&dir, &["id"]).unwrap();
        let second = PartReader::open(&dir, &["id", "name"]).unwrap();

        assert_eq!(first.num_rows, 2);
        assert_eq!(second.num_rows, 2);
    }

    // ---- open: projection contract -------------------------------------

    #[test]
    #[should_panic(expected = "empty column projection not supported yet")]
    fn panics_on_an_empty_projection() {
        let (_root, dir) = written_part(1);

        let _ = PartReader::open(&dir, &[]);
    }

    #[test]
    #[should_panic(expected = "duplicate column name: id")]
    fn panics_on_a_duplicate_requested_column() {
        let (_root, dir) = written_part(1);

        let _ = PartReader::open(&dir, &["id", "id"]);
    }

    /// The check scans every earlier request, not just the preceding one — the
    /// projection counterpart of `detects_non_adjacent_duplicate_column_names`.
    #[test]
    #[should_panic(expected = "duplicate column name: id")]
    fn detects_non_adjacent_duplicate_requested_columns() {
        let (_root, dir) = written_part(1);

        let _ = PartReader::open(&dir, &["id", "name", "id"]);
    }

    /// The projection is a caller-contract violation, so it is rejected before any
    /// filesystem call — a bad projection panics rather than being masked by a
    /// `NotFound` for the directory.
    #[test]
    #[should_panic(expected = "empty column projection not supported yet")]
    fn the_projection_is_validated_before_the_filesystem_is_touched() {
        let _ = PartReader::open(Path::new("/nonexistent/part_0"), &[]);
    }

    // ---- open: missing or unreadable part ------------------------------

    #[test]
    fn a_missing_directory_is_not_found() {
        let (_root, dir) = part_dir();

        let e = open_error(&dir, &["id"]);

        assert!(
            matches!(e, StorageError::NotFound(ref p) if *p == dir),
            "{e:?}"
        );
        assert!(e.to_string().starts_with("not found: "));
    }

    /// `is_dir` folds "wrong kind of thing" into the same error as "nothing
    /// there". Documents current behavior.
    #[test]
    fn a_regular_file_in_place_of_a_part_dir_is_not_found() {
        let (_root, dir) = part_dir();
        fs::write(&dir, b"not a part").unwrap();

        let e = open_error(&dir, &["id"]);

        assert!(matches!(e, StorageError::NotFound(_)), "{e:?}");
    }

    /// The payoff of the writer's staging directory: a part still being written is
    /// invisible, so a reader never sees a torn one.
    #[test]
    fn an_unfinished_part_is_not_found() {
        let (_root, dir) = part_dir();
        let mut writer = PartWriter::new(dir.clone(), sample_schema(), Codec::Lz4).unwrap();
        writer.write_block(&sample_block(1)).unwrap();

        let e = open_error(&dir, &["id"]);

        assert!(matches!(e, StorageError::NotFound(_)), "{e:?}");
    }

    /// Reaching past the staging directory's name does not get you a readable
    /// part either: `schema.txt` is written only by `finish`.
    #[test]
    fn a_staging_directory_is_not_a_readable_part() {
        let (_root, dir) = part_dir();
        let writer = PartWriter::new(dir.clone(), sample_schema(), Codec::Lz4).unwrap();
        drop(writer);

        assert!(open_err(&staging_of(&dir), &["id"]).contains("schema.txt missing"));
    }

    /// A directory that exists but has no schema is corrupt, not absent — the part
    /// was found, it just cannot be interpreted.
    #[test]
    fn a_dir_without_a_schema_file_is_corrupt() {
        let (_root, dir) = part_dir();
        fs::create_dir_all(&dir).unwrap();

        let e = open_error(&dir, &["id"]);

        assert!(matches!(e, StorageError::Corrupt(_)), "{e:?}");
        assert!(e.to_string().contains("schema.txt missing"));
    }

    /// Only `NotFound` is reclassified as a corrupt part; every other read failure
    /// keeps its `io::Error`, so a permissions or encoding problem is not
    /// misreported as a damaged part.
    #[test]
    fn a_non_utf8_schema_file_surfaces_as_io() {
        let (_root, dir) = part_dir();
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("schema.txt"), [0xff, 0xfe, 0xfd]).unwrap();

        let e = open_error(&dir, &["id"]);

        assert!(matches!(e, StorageError::Io(_)), "{e:?}");
    }

    /// One representative case — `parse_schema` itself is covered exhaustively
    /// above; this only pins that `open` propagates rather than swallows.
    #[test]
    fn schema_parse_errors_are_propagated() {
        let (_root, dir) = part_with_schema_text("version=2\nnum_rows=0\ncolumn=id:Int64\n");

        let e = open_error(&dir, &["id"]);

        assert!(
            matches!(e, StorageError::UnsupportedVersion { ref found, .. } if found == "2"),
            "{e:?}"
        );
    }

    // ---- open: column resolution ---------------------------------------

    #[test]
    fn an_unknown_column_is_column_not_found() {
        let (_root, dir) = written_part(1);

        let e = open_error(&dir, &["nope"]);

        assert!(
            matches!(e, StorageError::ColumnNotFound(ref n) if n == "nope"),
            "{e:?}"
        );
        assert_eq!(e.to_string(), "column 'nope' not found");
    }

    /// Names differing only in case are distinct columns, matching
    /// `duplicate_detection_is_case_sensitive` on the schema side.
    #[test]
    fn column_lookup_is_case_sensitive() {
        let (_root, dir) = written_part(1);

        assert!(open_err(&dir, &["ID"]).contains("column 'ID' not found"));
    }

    /// Resolution short-circuits in *projection* order, not schema order, so the
    /// column the caller listed first is the one reported.
    #[test]
    fn the_first_unknown_column_in_projection_order_is_reported() {
        let (_root, dir) = written_part(1);

        assert!(open_err(&dir, &["zzz", "aaa"]).contains("column 'zzz' not found"));
    }

    // ---- open: corrupt column files ------------------------------------

    #[test]
    fn a_missing_numeric_column_file_is_corrupt() {
        let (_root, dir) = written_part(1);
        fs::remove_file(dir.join("id.bin")).unwrap();

        let e = open_error(&dir, &["id"]);

        assert!(matches!(e, StorageError::Corrupt(_)), "{e:?}");
        assert_eq!(e.to_string(), "corrupt: column file 'id.bin' missing");
    }

    #[test]
    fn a_missing_string_data_file_is_corrupt() {
        let (_root, dir) = written_part(1);
        fs::remove_file(dir.join("name.data.bin")).unwrap();

        assert!(open_err(&dir, &["name"]).contains("column file 'name.data.bin' missing"));
    }

    /// Both halves of a string column need their own case — a reader that opened
    /// only the data file would still pass the test above while being unable to
    /// decode a single value.
    #[test]
    fn a_missing_string_offsets_file_is_corrupt() {
        let (_root, dir) = written_part(1);
        fs::remove_file(dir.join("name.offsets.bin")).unwrap();

        assert!(open_err(&dir, &["name"]).contains("column file 'name.offsets.bin' missing"));
    }

    /// With both gone the data file is named, because the struct literal evaluates
    /// `data` before `offsets`. Pins which file the message points at.
    #[test]
    fn the_data_file_is_reported_when_both_string_files_are_missing() {
        let (_root, dir) = written_part(1);
        fs::remove_file(dir.join("name.data.bin")).unwrap();
        fs::remove_file(dir.join("name.offsets.bin")).unwrap();

        assert!(open_err(&dir, &["name"]).contains("column file 'name.data.bin' missing"));
    }

    /// The positive form of the tests above: `open` touches nothing outside the
    /// projection, so damage to an unread column costs nothing.
    #[test]
    fn only_projected_columns_are_opened() {
        let (_root, dir) = written_part(1);
        fs::remove_file(dir.join("score.bin")).unwrap();
        fs::remove_file(dir.join("name.data.bin")).unwrap();

        let reader = PartReader::open(&dir, &["id"]).unwrap();

        assert_eq!(reader_indices(&reader), vec![0]);
    }

    /// Lookup and file-opening are interleaved per column rather than run as two
    /// passes, so a broken early column outranks an unknown later one.
    /// `ColumnNotFound` only wins *within* a single column. Pins the precedence.
    #[test]
    fn a_missing_file_is_reported_before_a_later_unknown_column() {
        let (_root, dir) = written_part(1);
        fs::remove_file(dir.join("id.bin")).unwrap();

        assert!(open_err(&dir, &["id", "nope"]).contains("column file 'id.bin' missing"));
    }

    // ---- open: pinned current behavior ---------------------------------

    /// Nothing is decoded at open time, so garbage in a column file is a read-time
    /// failure. Documents current behavior.
    #[test]
    fn open_does_not_validate_column_file_contents() {
        let (_root, dir) = written_part(2);
        fs::write(dir.join("id.bin"), b"not a chunk").unwrap();

        assert!(PartReader::open(&dir, &["id"]).is_ok());
    }

    /// `num_rows` is taken from the header on trust — it is never checked against
    /// the bytes on disk. Documents current behavior.
    #[test]
    fn open_does_not_cross_check_num_rows_against_the_data() {
        let (_root, dir) = part_with_schema_text("version=1\nnum_rows=7\ncolumn=id:Int64\n");
        fs::write(dir.join("id.bin"), b"").unwrap();

        let reader = PartReader::open(&dir, &["id"]).unwrap();

        assert_eq!(reader.num_rows, 7);
    }

    /// The counterpart of `a_missing_numeric_column_file_is_corrupt`: a file that
    /// is present but cannot be opened keeps its `io::Error` instead of being
    /// reported as a missing file.
    #[test]
    #[cfg(unix)]
    fn an_unreadable_column_file_surfaces_as_io() {
        use std::os::unix::fs::PermissionsExt;

        let (_root, dir) = written_part(1);
        let path = dir.join("id.bin");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();

        // `root` ignores the mode bits, leaving nothing to assert.
        if File::open(&path).is_ok() {
            return;
        }

        let e = open_error(&dir, &["id"]);

        assert!(matches!(e, StorageError::Io(_)), "{e:?}");
    }

    // ---- `next_block` fixtures -----------------------------------------

    /// `sample_block`, but with every value derived from `start`, so consecutive
    /// blocks are distinguishable. `sample_block(n) == sample_block_at(0, n)`.
    fn sample_block_at(start: i64, rows: usize) -> Block {
        let ids: Vec<i64> = (start..start + rows as i64).collect();
        let mut names = StringColumn::new();
        for id in &ids {
            names.push(&format!("n{id}"));
        }
        Block::new(
            vec![
                ("id".to_string(), Column::Int64(ids.clone())),
                ("name".to_string(), Column::String(names)),
                (
                    "score".to_string(),
                    Column::Float64(ids.iter().map(|&i| i as f64).collect()),
                ),
            ],
            rows,
        )
    }

    /// A finished part whose blocks have the given row counts, in order.
    /// `PartWriter::write_block` never splits or merges a block, so repeated calls
    /// are the only honest way to get more than one chunk per column file.
    fn written_part_blocks(rows_per_block: &[usize]) -> (TempDir, PathBuf) {
        let (root, dir) = part_dir();

        let mut writer = PartWriter::new(dir.clone(), sample_schema(), Codec::Lz4).unwrap();
        let mut start = 0i64;
        for &rows in rows_per_block {
            writer.write_block(&sample_block_at(start, rows)).unwrap();
            start += rows as i64;
        }
        writer.finish().unwrap();

        (root, dir)
    }

    /// A part with a hand-written `schema.txt` and an empty file for every column
    /// it declares. The empty files are not incidental: `open` refuses a part with
    /// a missing column file, so even a file the test never appends to must exist.
    fn hand_built_part(num_rows: usize, schema: &Schema) -> (TempDir, PathBuf) {
        let mut text = format!("version=1\nnum_rows={num_rows}\n");
        for (name, dt) in schema.iter() {
            text.push_str(&format!("column={name}:{}\n", dt.as_str()));
        }

        let (root, dir) = part_with_schema_text(&text);
        for (name, dt) in schema.iter() {
            match dt {
                DataType::Int64 | DataType::Float64 => {
                    File::create(dir.join(format!("{name}.bin"))).unwrap();
                }
                DataType::String => {
                    File::create(dir.join(format!("{name}.data.bin"))).unwrap();
                    File::create(dir.join(format!("{name}.offsets.bin"))).unwrap();
                }
            }
        }

        (root, dir)
    }

    fn append(dir: &Path, file: &str) -> File {
        OpenOptions::new()
            .append(true)
            .open(dir.join(file))
            .unwrap()
    }

    /// Appends one chunk to a column, framed exactly as `PartWriter` frames it —
    /// so a test can spell out a per-column chunk sequence the writer could never
    /// produce, such as two columns of differing chunk counts.
    fn append_chunk(dir: &Path, name: &str, column: &Column) {
        match column {
            Column::Int64(v) => {
                write_i64_chunk(&mut append(dir, &format!("{name}.bin")), v, Codec::Lz4).unwrap()
            }
            Column::Float64(v) => {
                write_f64_chunk(&mut append(dir, &format!("{name}.bin")), v, Codec::Lz4).unwrap()
            }
            Column::String(sc) => write_str_chunk(
                &mut append(dir, &format!("{name}.data.bin")),
                &mut append(dir, &format!("{name}.offsets.bin")),
                sc,
                Codec::Lz4,
            )
            .unwrap(),
        }
    }

    fn append_raw(dir: &Path, file: &str, bytes: &[u8]) {
        append(dir, file).write_all(bytes).unwrap();
    }

    /// One well-framed block around an arbitrary payload — for chunks whose *body*
    /// must be invalid in a way `write_*_chunk` would never emit.
    fn framed(raw: &[u8]) -> Vec<u8> {
        let mut buf = Vec::new();
        write_block(&mut buf, raw, Codec::Lz4).unwrap();
        buf
    }

    fn truncate_by(dir: &Path, file: &str, bytes: u64) {
        let path = dir.join(file);
        let len = fs::metadata(&path).unwrap().len();
        OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(len - bytes)
            .unwrap();
    }

    /// A part over `[id:Int64, score:Float64]` declaring `num_rows`, holding
    /// exactly the given chunk sequences. Most desync and truncation fixtures are
    /// a single call to this.
    fn numeric_part(num_rows: usize, ids: &[&[i64]], scores: &[&[f64]]) -> (TempDir, PathBuf) {
        let schema = Schema::new(vec![
            col("id", DataType::Int64),
            col("score", DataType::Float64),
        ])
        .unwrap();
        let (root, dir) = hand_built_part(num_rows, &schema);

        for chunk in ids {
            append_chunk(&dir, "id", &Column::Int64(chunk.to_vec()));
        }
        for chunk in scores {
            append_chunk(&dir, "score", &Column::Float64(chunk.to_vec()));
        }

        (root, dir)
    }

    /// The error from a `next_block` that must fail, mirroring `open_error`.
    fn next_error(reader: &mut PartReader) -> StorageError {
        match reader.next_block() {
            Ok(block) => panic!("expected next_block to fail, got {block:?}"),
            Err(e) => e,
        }
    }

    /// Rendered message of a failed `next_block`, mirroring `open_err`.
    fn next_err(reader: &mut PartReader) -> String {
        next_error(reader).to_string()
    }

    /// Every block until the reader ends.
    fn read_all(reader: &mut PartReader) -> Vec<Block> {
        let mut blocks = Vec::new();
        while let Some(block) = reader.next_block().unwrap() {
            blocks.push(block);
        }
        blocks
    }

    fn row_counts(blocks: &[Block]) -> Vec<usize> {
        blocks.iter().map(|b| b.num_rows()).collect()
    }

    /// `StringColumn` has no iterator, so values come out by index.
    fn strs(block: &Block, name: &str) -> Vec<String> {
        match block.column(name) {
            Some(Column::String(sc)) => (0..sc.len()).map(|i| sc.get(i).to_string()).collect(),
            other => panic!("column '{name}' is not a string column: {other:?}"),
        }
    }

    /// `NaN != NaN`, so a float column carrying one cannot be compared with `==`.
    fn bits(values: &[f64]) -> Vec<u64> {
        values.iter().map(|v| v.to_bits()).collect()
    }

    // ---- next_block: round trip ----------------------------------------

    /// The read half of the writer/reader contract, across all three types at
    /// once: what went into `write_block` is what comes out of `next_block`.
    #[test]
    fn next_block_reads_back_the_block_part_writer_wrote() {
        let (_root, dir) = written_part(3);
        let mut reader = PartReader::open(&dir, &["id", "name", "score"]).unwrap();

        let block = reader.next_block().unwrap().unwrap();

        assert_eq!(block.num_rows(), 3);
        assert_eq!(block.column("id"), Some(&Column::Int64(vec![0, 1, 2])));
        assert_eq!(
            block.column("name"),
            Some(&Column::String(StringColumn::new_with_values(&[
                "n0", "n1", "n2"
            ])))
        );
        assert_eq!(
            block.column("score"),
            Some(&Column::Float64(vec![0.0, 1.0, 2.0]))
        );
        assert!(reader.next_block().unwrap().is_none());
    }

    /// Chunk boundaries are block boundaries: nothing is merged, nothing is split.
    #[test]
    fn next_block_returns_one_block_per_written_block() {
        let (_root, dir) = written_part_blocks(&[2, 1, 3]);
        let mut reader = PartReader::open(&dir, &["id", "name", "score"]).unwrap();

        assert_eq!(row_counts(&read_all(&mut reader)), vec![2, 1, 3]);
    }

    /// Row counts alone would pass even if the blocks came back reversed — this
    /// pins the values, so ordering is actually checked.
    #[test]
    fn blocks_come_back_in_write_order() {
        let (_root, dir) = written_part_blocks(&[2, 2]);
        let mut reader = PartReader::open(&dir, &["id", "name"]).unwrap();

        let blocks = read_all(&mut reader);

        assert_eq!(blocks[0].column("id"), Some(&Column::Int64(vec![0, 1])));
        assert_eq!(strs(&blocks[0], "name"), vec!["n0", "n1"]);
        assert_eq!(blocks[1].column("id"), Some(&Column::Int64(vec![2, 3])));
        assert_eq!(strs(&blocks[1], "name"), vec!["n2", "n3"]);
    }

    #[test]
    fn next_block_returns_none_after_the_last_block() {
        let (_root, dir) = written_part(1);
        let mut reader = PartReader::open(&dir, &["id"]).unwrap();

        assert!(reader.next_block().unwrap().is_some());
        assert!(reader.next_block().unwrap().is_none());
    }

    /// The end of a part is not a one-shot signal: a caller that polls again gets
    /// `None` again, not an error and not a phantom block. Every reader re-hits a
    /// clean EOF and `rows_read` still matches, so the truncation check keeps
    /// passing.
    #[test]
    fn next_block_keeps_returning_none_after_the_end() {
        let (_root, dir) = written_part(1);
        let mut reader = PartReader::open(&dir, &["id", "name"]).unwrap();

        assert!(reader.next_block().unwrap().is_some());
        for _ in 0..3 {
            assert!(reader.next_block().unwrap().is_none());
        }
    }

    /// A writer that never received a block leaves empty column files, and that is
    /// an empty part, not a corrupt one.
    #[test]
    fn a_part_with_no_blocks_yields_none_immediately() {
        let (_root, dir) = written_part(0);
        let mut reader = PartReader::open(&dir, &["id", "name", "score"]).unwrap();

        assert!(reader.next_block().unwrap().is_none());
    }

    /// The read-side counterpart of `part_writer::empty_block_writes_a_zero_length_chunk`:
    /// a framed zero-length chunk is a *block*, distinct from EOF. Confusing the
    /// two would make an empty block silently truncate the part.
    #[test]
    fn a_zero_row_block_comes_back_as_an_empty_block() {
        let (_root, dir) = written_part_blocks(&[0]);
        let mut reader = PartReader::open(&dir, &["id", "name", "score"]).unwrap();

        let block = reader.next_block().unwrap().unwrap();

        assert_eq!(block.num_rows(), 0);
        assert_eq!(column_names(&block), vec!["id", "name", "score"]);
        assert!(block.columns().iter().all(|(_, c)| c.is_empty()));
        assert!(reader.next_block().unwrap().is_none());
    }

    /// An empty block neither ends the iteration nor collapses into its
    /// neighbours.
    #[test]
    fn zero_row_blocks_between_data_blocks_are_preserved() {
        let (_root, dir) = written_part_blocks(&[2, 0, 1]);
        let mut reader = PartReader::open(&dir, &["id", "name", "score"]).unwrap();

        assert_eq!(row_counts(&read_all(&mut reader)), vec![2, 0, 1]);
    }

    /// N empty blocks yield N blocks, and `rows_read == 0` still satisfies the
    /// declared `num_rows=0`.
    #[test]
    fn a_part_of_only_zero_row_blocks_yields_one_block_each() {
        let (_root, dir) = written_part_blocks(&[0, 0]);
        let mut reader = PartReader::open(&dir, &["id", "name", "score"]).unwrap();

        assert_eq!(row_counts(&read_all(&mut reader)), vec![0, 0]);
    }

    /// The `Pair` branch has to rebuild the offsets as well as the bytes. An empty
    /// value and a multibyte one are where an off-by-one or a char/byte confusion
    /// would show.
    #[test]
    fn string_values_survive_the_round_trip() {
        let (_root, dir) = part_dir();
        let names = StringColumn::new_with_values(&["", "日本語", "a"]);
        let mut writer = PartWriter::new(dir.clone(), sample_schema(), Codec::Lz4).unwrap();
        writer
            .write_block(&Block::new(
                vec![
                    ("id".to_string(), Column::Int64(vec![0, 1, 2])),
                    ("name".to_string(), Column::String(names.clone())),
                    ("score".to_string(), Column::Float64(vec![0.0, 0.0, 0.0])),
                ],
                3,
            ))
            .unwrap();
        writer.finish().unwrap();

        let mut reader = PartReader::open(&dir, &["name"]).unwrap();
        let block = reader.next_block().unwrap().unwrap();

        assert_eq!(block.column("name"), Some(&Column::String(names)));
    }

    /// Values are moved as little-endian bytes, never through an intermediate
    /// wider or narrower type — the ends of each range prove it.
    #[test]
    fn extreme_numeric_values_survive_the_round_trip() {
        let (_root, dir) = part_dir();
        let scores = vec![f64::NAN, f64::INFINITY, f64::NEG_INFINITY];
        let mut writer = PartWriter::new(dir.clone(), sample_schema(), Codec::Lz4).unwrap();
        writer
            .write_block(&Block::new(
                vec![
                    ("id".to_string(), Column::Int64(vec![i64::MIN, 0, i64::MAX])),
                    (
                        "name".to_string(),
                        Column::String(StringColumn::new_with_values(&["a", "b", "c"])),
                    ),
                    ("score".to_string(), Column::Float64(scores.clone())),
                ],
                3,
            ))
            .unwrap();
        writer.finish().unwrap();

        let mut reader = PartReader::open(&dir, &["id", "score"]).unwrap();
        let block = reader.next_block().unwrap().unwrap();

        assert_eq!(
            block.column("id"),
            Some(&Column::Int64(vec![i64::MIN, 0, i64::MAX]))
        );
        let Some(Column::Float64(read)) = block.column("score") else {
            panic!("score is not a float column");
        };
        assert_eq!(bits(read), bits(&scores));
    }

    // ---- next_block: projection -----------------------------------------

    #[test]
    fn a_block_holds_only_the_projected_columns() {
        let (_root, dir) = written_part(2);
        let mut reader = PartReader::open(&dir, &["id"]).unwrap();

        let block = reader.next_block().unwrap().unwrap();

        assert_eq!(column_names(&block), vec!["id"]);
        assert_eq!(block.num_rows(), 2);
        assert!(block.column("name").is_none());
    }

    /// The headline invariant of this method, and the counterpart of
    /// `readers_are_ordered_by_schema_index_not_by_projection_order`: `open` sorts
    /// the readers, so the block is in schema order however the caller asked. A
    /// caller that indexed `columns()` positionally against its own projection
    /// would silently read the wrong column.
    #[test]
    fn block_columns_are_in_schema_order_not_projection_order() {
        let (_root, dir) = written_part(2);
        let mut reader = PartReader::open(&dir, &["score", "id"]).unwrap();

        let block = reader.next_block().unwrap().unwrap();

        assert_eq!(column_names(&block), vec!["id", "score"]);
    }

    /// Row counts come from the chunks, so they are a property of the part, not of
    /// what was asked for.
    #[test]
    fn projection_does_not_change_row_counts() {
        let (_root, dir) = written_part_blocks(&[2, 1]);
        let mut reader = PartReader::open(&dir, &["score"]).unwrap();

        assert_eq!(row_counts(&read_all(&mut reader)), vec![2, 1]);
    }

    /// The read-time counterpart of `only_projected_columns_are_opened`: a column
    /// outside the projection is never decoded, so damage to it costs nothing.
    #[test]
    fn damage_to_an_unprojected_column_is_never_read() {
        let (_root, dir) = written_part(2);
        fs::write(dir.join("name.data.bin"), b"garbage").unwrap();
        let mut reader = PartReader::open(&dir, &["id", "score"]).unwrap();

        assert_eq!(row_counts(&read_all(&mut reader)), vec![2]);
    }

    /// Each reader owns its own file cursors, so one draining the part does not
    /// move another along with it.
    #[test]
    fn two_readers_over_one_part_advance_independently() {
        let (_root, dir) = written_part_blocks(&[1, 1]);
        let mut first = PartReader::open(&dir, &["id"]).unwrap();
        let mut second = PartReader::open(&dir, &["id"]).unwrap();

        first.next_block().unwrap().unwrap();

        assert_eq!(
            first.next_block().unwrap().unwrap().column("id"),
            Some(&Column::Int64(vec![1]))
        );
        assert_eq!(
            second.next_block().unwrap().unwrap().column("id"),
            Some(&Column::Int64(vec![0]))
        );
    }

    /// The `Pair` branch driving both of its files to the end on its own, with no
    /// numeric column alongside to keep it honest.
    #[test]
    fn a_string_only_projection_reads_to_the_end() {
        let (_root, dir) = written_part_blocks(&[2, 1]);
        let mut reader = PartReader::open(&dir, &["name"]).unwrap();

        let blocks = read_all(&mut reader);

        assert_eq!(row_counts(&blocks), vec![2, 1]);
        assert_eq!(strs(&blocks[0], "name"), vec!["n0", "n1"]);
        assert_eq!(strs(&blocks[1], "name"), vec!["n2"]);
    }

    // ---- next_block: row accounting -------------------------------------

    /// `rows_read` is the counter the end-of-part check is built on, so it has to
    /// track the chunks actually handed out.
    #[test]
    fn rows_read_accumulates_across_blocks() {
        let (_root, dir) = written_part_blocks(&[2, 1, 3]);
        let mut reader = PartReader::open(&dir, &["id"]).unwrap();

        assert_eq!(reader.rows_read, 0);
        for expected in [2, 3, 6] {
            reader.next_block().unwrap().unwrap();
            assert_eq!(reader.rows_read, expected);
        }
        assert!(reader.next_block().unwrap().is_none());
        assert_eq!(reader.rows_read, 6);
    }

    /// The declared row count is trusted at `open`
    /// (`open_does_not_cross_check_num_rows_against_the_data`) and enforced here.
    /// This is the only place it is ever checked.
    #[test]
    fn a_part_holding_fewer_rows_than_declared_is_corrupt() {
        let (_root, dir) = numeric_part(5, &[&[1, 2]], &[&[1.0, 2.0]]);
        let mut reader = PartReader::open(&dir, &["id", "score"]).unwrap();

        assert_eq!(reader.next_block().unwrap().unwrap().num_rows(), 2);
        let e = next_error(&mut reader);

        assert!(matches!(e, StorageError::Corrupt(_)), "{e:?}");
        assert_eq!(
            e.to_string(),
            "corrupt: part truncated: schema declares 5 rows, files contain 2"
        );
    }

    /// The check runs only once every reader has ended, so the short block is
    /// handed to the caller *before* the part is known to be broken. A consumer
    /// that stops early — a future `LIMIT` — never learns of the damage.
    /// Documents current behavior.
    #[test]
    fn the_truncation_check_runs_only_at_the_end_of_the_part() {
        let (_root, dir) = numeric_part(5, &[&[1, 2]], &[&[1.0, 2.0]]);
        let mut reader = PartReader::open(&dir, &["id", "score"]).unwrap();

        let block = reader.next_block().unwrap().unwrap();

        assert_eq!(block.column("id"), Some(&Column::Int64(vec![1, 2])));
    }

    /// Empty files with a positive `num_rows` fail on the very first call rather
    /// than passing for an empty part.
    #[test]
    fn a_part_with_empty_files_but_declared_rows_is_corrupt() {
        let (_root, dir) = numeric_part(3, &[], &[]);
        let mut reader = PartReader::open(&dir, &["id", "score"]).unwrap();

        assert!(next_err(&mut reader).contains("schema declares 3 rows, files contain 0"));
    }

    /// The check is an equality, not a floor — surplus data is as much a mismatch
    /// as missing data. Note the message still says "truncated"; see the note
    /// below. Documents current behavior.
    #[test]
    fn a_part_holding_more_rows_than_declared_is_corrupt() {
        let (_root, dir) = numeric_part(1, &[&[1, 2, 3]], &[&[1.0, 2.0, 3.0]]);
        let mut reader = PartReader::open(&dir, &["id", "score"]).unwrap();

        assert_eq!(reader.next_block().unwrap().unwrap().num_rows(), 3);

        assert!(
            next_err(&mut reader)
                .contains("part truncated: schema declares 1 rows, files contain 3")
        );
    }

    /// The files stay at EOF and `rows_read` stops moving, so the failure is
    /// stable rather than degrading into a different one.
    #[test]
    fn the_truncation_error_repeats_on_a_subsequent_call() {
        let (_root, dir) = numeric_part(5, &[&[1, 2]], &[&[1.0, 2.0]]);
        let mut reader = PartReader::open(&dir, &["id", "score"]).unwrap();
        reader.next_block().unwrap().unwrap();

        let first = next_err(&mut reader);
        let second = next_err(&mut reader);

        assert_eq!(first, second);
    }

    // ---- next_block: cross-column desync --------------------------------

    /// Column files advance in lockstep or the part is corrupt. This check is also
    /// what keeps `Block::new`'s length assert from firing — without it a
    /// mismatched part would panic instead of returning an error.
    #[test]
    fn a_shorter_chunk_in_a_later_column_is_out_of_sync() {
        let (_root, dir) = numeric_part(2, &[&[1, 2]], &[&[1.0]]);
        let mut reader = PartReader::open(&dir, &["id", "score"]).unwrap();

        let e = next_error(&mut reader);

        assert!(matches!(e, StorageError::Corrupt(_)), "{e:?}");
        assert_eq!(
            e.to_string(),
            "corrupt: column files out of sync: 'score' chunk has 1 rows, expected 2"
        );
    }

    /// `len` is set by the first column that produced a chunk and never revisited,
    /// so when the *earlier* column is the damaged one the message blames the
    /// healthy one. Documents current behavior.
    #[test]
    fn the_first_column_in_schema_order_defines_the_expected_length() {
        let (_root, dir) = numeric_part(2, &[&[1]], &[&[1.0, 2.0]]);
        let mut reader = PartReader::open(&dir, &["id", "score"]).unwrap();

        assert!(
            next_err(&mut reader).contains("'score' chunk has 2 rows, expected 1"),
            "the earlier column wins even when it is the broken one"
        );
    }

    /// `StringColumn::len()` is `offsets.len() - 1`, a different computation from
    /// `Vec::len` — the sync check has to see through both.
    #[test]
    fn desync_between_a_numeric_and_a_string_column_is_detected() {
        let schema = Schema::new(vec![
            col("id", DataType::Int64),
            col("name", DataType::String),
        ])
        .unwrap();
        let (_root, dir) = hand_built_part(2, &schema);
        append_chunk(&dir, "id", &Column::Int64(vec![1, 2]));
        append_chunk(
            &dir,
            "name",
            &Column::String(StringColumn::new_with_values(&["x"])),
        );
        let mut reader = PartReader::open(&dir, &["id", "name"]).unwrap();

        assert!(next_err(&mut reader).contains("'name' chunk has 1 rows, expected 2"));
    }

    /// With two columns disagreeing, the earlier one in schema order is reported —
    /// the scan short-circuits rather than collecting every mismatch.
    #[test]
    fn the_first_disagreeing_column_is_the_one_reported() {
        let (_root, dir) = hand_built_part(3, &sample_schema());
        append_chunk(&dir, "id", &Column::Int64(vec![1, 2, 3]));
        append_chunk(
            &dir,
            "name",
            &Column::String(StringColumn::new_with_values(&["x"])),
        );
        append_chunk(&dir, "score", &Column::Float64(vec![1.0, 2.0]));
        let mut reader = PartReader::open(&dir, &["id", "name", "score"]).unwrap();

        assert!(next_err(&mut reader).contains("'name' chunk has 1 rows, expected 3"));
    }

    /// One file running out while another still has chunks is corruption, not the
    /// end of the part — the distinction between `ended == readers.len()` and
    /// anything less.
    #[test]
    fn a_column_file_that_ends_early_is_out_of_sync() {
        let (_root, dir) = numeric_part(4, &[&[1, 2], &[3, 4]], &[&[1.0, 2.0]]);
        let mut reader = PartReader::open(&dir, &["id", "score"]).unwrap();

        assert_eq!(reader.next_block().unwrap().unwrap().num_rows(), 2);
        let e = next_error(&mut reader);

        assert!(matches!(e, StorageError::Corrupt(_)), "{e:?}");
        assert_eq!(
            e.to_string(),
            "corrupt: column files out of sync: some ended, some continue"
        );
    }

    /// The mirror image: a surplus chunk in a later column fails the same way a
    /// missing one does.
    #[test]
    fn a_column_file_with_a_surplus_chunk_is_out_of_sync() {
        let (_root, dir) = numeric_part(2, &[&[1, 2]], &[&[1.0, 2.0], &[3.0]]);
        let mut reader = PartReader::open(&dir, &["id", "score"]).unwrap();

        assert_eq!(reader.next_block().unwrap().unwrap().num_rows(), 2);

        assert!(next_err(&mut reader).contains("some ended, some continue"));
    }

    /// Error precedence: the declared-row-count check lives on the arm where
    /// *every* reader ended, so a part that is both desynced and short reports the
    /// desync. The stronger signal about the file layout wins.
    #[test]
    fn an_out_of_sync_part_is_reported_before_the_truncation_check() {
        let (_root, dir) = numeric_part(99, &[&[1, 2], &[3]], &[&[1.0, 2.0]]);
        let mut reader = PartReader::open(&dir, &["id", "score"]).unwrap();
        reader.next_block().unwrap().unwrap();

        let message = next_err(&mut reader);

        assert!(message.contains("some ended, some continue"), "{message}");
        assert!(!message.contains("part truncated"), "{message}");
    }

    // ---- next_block: codec errors ---------------------------------------

    /// Nothing is decoded at open time (`open_does_not_validate_column_file_contents`),
    /// so undecodable bytes surface here — as `Codec`, not as `Corrupt`. Five
    /// bytes is deliberate: a longer run would be read as a header and take a
    /// different branch.
    #[test]
    fn garbage_in_a_column_file_is_a_codec_error() {
        let (_root, dir) = numeric_part(0, &[], &[]);
        append_raw(&dir, "id.bin", &[1, 2, 3, 4, 5]);
        let mut reader = PartReader::open(&dir, &["id"]).unwrap();

        let e = next_error(&mut reader);

        assert!(matches!(e, StorageError::Codec(_)), "{e:?}");
        assert!(e.to_string().contains("truncated header: got 5 of 9 bytes"));
    }

    /// A torn tail is a *codec* truncation, not the `part truncated` row-count
    /// mismatch. The two failures share a word and nothing else; conflating them
    /// would send an operator looking for the wrong damage.
    #[test]
    fn a_torn_final_chunk_is_a_codec_error() {
        let (_root, dir) = written_part(3);
        truncate_by(&dir, "id.bin", 4);
        let mut reader = PartReader::open(&dir, &["id"]).unwrap();

        let e = next_error(&mut reader);

        assert!(matches!(e, StorageError::Codec(_)), "{e:?}");
        assert!(e.to_string().contains("truncated body"), "{e}");
        assert!(!e.to_string().contains("part truncated"), "{e}");
    }

    /// `StorageError::Codec` is `#[error(transparent)]`, so a codec failure keeps
    /// `CodecError`'s own `corrupt block: ` prefix and never gains the
    /// `corrupt: ` one. Pins the wrapping any message assertion depends on.
    #[test]
    fn codec_errors_render_as_corrupt_block_not_corrupt() {
        let (_root, dir) = numeric_part(0, &[], &[]);
        append_raw(&dir, "id.bin", &[1, 2, 3, 4, 5]);
        let mut reader = PartReader::open(&dir, &["id"]).unwrap();

        let e = next_error(&mut reader);

        assert!(matches!(e, StorageError::Codec(_)), "{e:?}");
        assert!(e.to_string().starts_with("corrupt block: "), "{e}");
    }

    /// Framing-level damage is caught by `read_block`; this is the layer above it,
    /// where a well-framed chunk holds the wrong number of bytes for its element
    /// type. Both reach the caller unmodified.
    #[test]
    fn a_chunk_of_the_wrong_element_width_is_a_codec_error() {
        let (_root, dir) = numeric_part(0, &[], &[]);
        append_raw(&dir, "id.bin", &framed(&[0u8; 12]));
        let mut reader = PartReader::open(&dir, &["id"]).unwrap();

        let e = next_error(&mut reader);

        assert!(matches!(e, StorageError::Codec(_)), "{e:?}");
        assert!(
            e.to_string()
                .contains("i64 chunk of 12 bytes is not a multiple of 8")
        );
    }

    /// A string column's two files can desync between themselves, independently of
    /// the cross-column check. That belongs to `read_str_chunk`, and `next_block`
    /// passes it through rather than reclassifying it as its own out-of-sync error.
    #[test]
    fn string_stream_desync_surfaces_as_a_codec_error() {
        let (_root, dir) = hand_built_part(
            1,
            &Schema::new(vec![col("name", DataType::String)]).unwrap(),
        );
        append_chunk(
            &dir,
            "name",
            &Column::String(StringColumn::new_with_values(&["x"])),
        );
        append_raw(&dir, "name.data.bin", &framed(b"y"));
        let mut reader = PartReader::open(&dir, &["name"]).unwrap();

        assert_eq!(reader.next_block().unwrap().unwrap().num_rows(), 1);
        let e = next_error(&mut reader);

        assert!(matches!(e, StorageError::Codec(_)), "{e:?}");
        assert!(
            e.to_string()
                .contains("string streams out of sync: data present, offsets ended")
        );
    }

    /// The `?` inside the loop fires before the `(ended, columns.len())` match, so
    /// a decode failure outranks the out-of-sync bookkeeping even when an earlier
    /// column has already ended.
    #[test]
    fn a_codec_error_outranks_the_out_of_sync_check() {
        let schema = Schema::new(vec![
            col("id", DataType::Int64),
            col("name", DataType::String),
        ])
        .unwrap();
        let (_root, dir) = hand_built_part(1, &schema);
        append_chunk(&dir, "id", &Column::Int64(vec![1]));
        append_chunk(
            &dir,
            "name",
            &Column::String(StringColumn::new_with_values(&["x"])),
        );
        append_raw(&dir, "name.data.bin", &framed(b"y"));
        let mut reader = PartReader::open(&dir, &["id", "name"]).unwrap();
        reader.next_block().unwrap().unwrap();

        let e = next_error(&mut reader);

        assert!(matches!(e, StorageError::Codec(_)), "{e:?}");
        assert!(!e.to_string().contains("some ended"), "{e}");
    }

    /// Codec failures short-circuit in schema order too — the loop stops at the
    /// first broken column rather than reporting the last.
    #[test]
    fn an_error_in_an_earlier_column_short_circuits_later_ones() {
        let (_root, dir) = numeric_part(0, &[], &[]);
        append_raw(&dir, "id.bin", &framed(&[0u8; 12]));
        append_raw(&dir, "score.bin", &framed(&[0u8; 12]));
        let mut reader = PartReader::open(&dir, &["id", "score"]).unwrap();

        let message = next_err(&mut reader);

        assert!(message.contains("i64 chunk"), "{message}");
        assert!(!message.contains("f64 chunk"), "{message}");
    }

    // ---- next_block: state after a failure ------------------------------

    /// The sharpest consequence of there being no failure fuse. The first call
    /// fails on `name` and returns before `score` is ever read, leaving the three
    /// files at different chunk offsets. The second call then finds three chunks
    /// that happen to agree on length and builds a perfectly valid-looking block
    /// out of rows from two different row groups: `score` here belongs to the
    /// *first* group, not to `id=3` / `name="y"`.
    ///
    /// A caller that logs the error and keeps going gets silently wrong data
    /// rather than a repeated failure. Documents current behavior.
    #[test]
    fn a_block_read_after_an_error_can_mix_misaligned_chunks() {
        let (_root, dir) = hand_built_part(3, &sample_schema());
        append_chunk(&dir, "id", &Column::Int64(vec![1, 2]));
        append_chunk(&dir, "id", &Column::Int64(vec![3]));
        append_chunk(
            &dir,
            "name",
            &Column::String(StringColumn::new_with_values(&["x"])),
        );
        append_chunk(
            &dir,
            "name",
            &Column::String(StringColumn::new_with_values(&["y"])),
        );
        append_chunk(&dir, "score", &Column::Float64(vec![9.0]));
        let mut reader = PartReader::open(&dir, &["id", "name", "score"]).unwrap();

        assert!(next_err(&mut reader).contains("'name' chunk has 1 rows, expected 2"));
        let block = reader.next_block().unwrap().unwrap();

        assert_eq!(block.num_rows(), 1);
        assert_eq!(block.column("id"), Some(&Column::Int64(vec![3])));
        assert_eq!(strs(&block, "name"), vec!["y"]);
        assert_eq!(block.column("score"), Some(&Column::Float64(vec![9.0])));
    }

    /// There is no poison flag, so a failed reader is not fused: the next call
    /// reports whatever the files say now, which is a *different* error.
    /// Documents current behavior.
    #[test]
    fn a_reader_is_not_fused_after_an_out_of_sync_error() {
        let (_root, dir) = numeric_part(4, &[&[1, 2], &[3, 4]], &[&[1.0, 2.0]]);
        let mut reader = PartReader::open(&dir, &["id", "score"]).unwrap();
        reader.next_block().unwrap().unwrap();

        assert!(next_err(&mut reader).contains("some ended, some continue"));
        assert!(next_err(&mut reader).contains("part truncated"));
    }

    /// `rows_read` moves only on the success arm, so the count quoted in the
    /// truncation message counts rows actually handed to the caller.
    #[test]
    fn the_row_counter_does_not_advance_on_a_failed_read() {
        let (_root, dir) = numeric_part(2, &[&[1, 2]], &[&[1.0]]);
        let mut reader = PartReader::open(&dir, &["id", "score"]).unwrap();

        next_error(&mut reader);

        assert_eq!(reader.rows_read, 0);
    }

    // ---- next_block: pinned current behavior ----------------------------

    /// `num_rows` is header state and `rows_read` is cursor state; only the latter
    /// moves. Reading must not "fix up" a part's declared size.
    #[test]
    fn next_block_does_not_mutate_the_declared_row_count() {
        let (_root, dir) = written_part_blocks(&[2, 1]);
        let mut reader = PartReader::open(&dir, &["id"]).unwrap();

        while reader.next_block().unwrap().is_some() {
            assert_eq!(reader.num_rows, 3);
        }
        assert_eq!(reader.num_rows, 3);
    }

    // Note: several `next_block` paths are deliberately untested.
    //
    // `StorageError::Io` — reads go through `File`, and `read_block` already maps
    // `UnexpectedEof` to `Corrupt`, so no ordinary filesystem produces a mid-read
    // `io::Error`. `codec.rs` carries the same note.
    //
    // The `(e, 0) if e == self.readers.len()` guard evaluating false — the loop
    // maintains `columns.len() + ended == readers.len()`, so `columns.len() == 0`
    // forces `ended == readers.len()`. No fixture can tell the guarded arm from an
    // unguarded one.
    //
    // A chunk at `MAX_BLOCK_SIZE` — constructible, but slow here and already
    // pinned at the boundary in `codec.rs` and `column_io.rs`.
    //
    // `Block::new`'s length assert firing from inside `next_block` — the length
    // check makes it unreachable, which is the point of the check.
    //
    // The `unreachable!("reader shape mismatch")` and `expect("non-empty readers")`
    // arms — both are unreachable through `open`, and reaching them would mean
    // building a `PartReader` from its private fields, pinning the struct's layout
    // rather than its behavior.
}
