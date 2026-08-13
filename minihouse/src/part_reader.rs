use crate::codec::CodecError;
use crate::column_io::{read_f64_chunk, read_i64_chunk, read_str_chunk};
use crate::{Block, Column, DataType};
use std::fs;
use std::fs::File;
use std::io::{BufReader, ErrorKind};
use std::path::{Path, PathBuf};

pub(crate) struct PartReader {
    schema: Vec<(String, DataType)>,
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
    pub(crate) fn open(dir: &Path, columns: &[&str]) -> Result<PartReader, PartError> {
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
            return Err(PartError::NotFound(dir.to_path_buf()));
        }

        let text = match fs::read_to_string(dir.join("schema.txt")) {
            Ok(text) => text,
            Err(err) if err.kind() == ErrorKind::NotFound => {
                return Err(PartError::Corrupt("schema.txt missing".into()));
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
                    .ok_or_else(|| PartError::ColumnNotFound(name.to_string()))?;
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
            .collect::<Result<Vec<_>, PartError>>()?;
        readers.sort_by_key(|(idx, _)| *idx);

        Ok(PartReader {
            schema,
            num_rows,
            rows_read: 0,
            readers,
        })
    }

    pub(crate) fn next_block(&mut self) -> Result<Option<Block>, PartError> {
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
                _ => unreachable!("column '{name}': reader shape mismatch with {dt:?} — broken by open"),
            };

            match column {
                None => ended += 1,
                Some(c) => {
                    match len {
                        None => len = Some(c.len()),
                        Some(l) if l != c.len() => {
                            return Err(PartError::Corrupt(format!(
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
                // все кончились — финальная сверка
                if self.rows_read != self.num_rows {
                    return Err(PartError::Corrupt(format!(
                        "part truncated: schema declares {} rows, files contain {}",
                        self.num_rows, self.rows_read
                    )));
                }
                Ok(None)
            }
            _ => Err(PartError::Corrupt(
                "column files out of sync: some ended, some continue".into(),
            )),
        }
    }
}

fn open_file(dir: &Path, name: &str) -> Result<BufReader<File>, PartError> {
    match File::open(dir.join(name)) {
        Ok(file) => Ok(BufReader::new(file)),
        Err(e) if e.kind() == ErrorKind::NotFound => {
            Err(PartError::Corrupt(format!("column file '{name}' missing")))
        }
        Err(err) => Err(err.into()),
    }
}

fn parse_schema(text: &str) -> Result<(Vec<(String, DataType)>, usize), PartError> {
    if text.is_empty() {
        return Err(PartError::Corrupt("empty schema".into()));
    }

    let mut lines = text.lines();
    let version = lines
        .next()
        .ok_or_else(|| PartError::Corrupt("missing version line".into()))?
        .strip_prefix("version=")
        .ok_or_else(|| PartError::Corrupt("invalid version line".into()))?;
    if version != "1" {
        return Err(PartError::UnsupportedVersion {
            found: version.to_string(),
            expected: 1,
        });
    }

    let num_rows = lines
        .next()
        .ok_or_else(|| PartError::Corrupt("missing num rows line".into()))?
        .strip_prefix("num_rows=")
        .ok_or_else(|| PartError::Corrupt("invalid num rows line".into()))?
        .parse::<usize>()
        .map_err(|_| PartError::Corrupt("invalid num rows".into()))?;

    let schema = lines
        .enumerate()
        .map(|(i, l)| {
            let line_no = i + 3;
            l.strip_prefix("column=")
                .ok_or_else(|| PartError::Corrupt(format!("line {line_no}: expected column=...")))
                .and_then(|d| {
                    d.split_once(':')
                        .ok_or_else(|| {
                            PartError::Corrupt(format!(
                                "line {line_no}: expected column=<name>:<type>"
                            ))
                        })
                        .and_then(|(name, type_str)| {
                            type_str
                                .parse::<DataType>()
                                .map(|dt| (name.to_string(), dt))
                                .map_err(|_| {
                                    PartError::Corrupt(format!(
                                        "line {line_no}: invalid data type '{type_str}'"
                                    ))
                                })
                        })
                })
        })
        .collect::<Result<Vec<(String, DataType)>, PartError>>()?;

    if schema.is_empty() && num_rows > 0 {
        return Err(PartError::Corrupt(format!(
            "num_rows={num_rows} but schema has no columns"
        )));
    }

    for i in 1..schema.len() {
        if schema[..i].iter().any(|(n, _)| n == &schema[i].0) {
            return Err(PartError::Corrupt(format!(
                "duplicate column name '{}'",
                schema[i].0
            )));
        }
    }

    Ok((schema, num_rows))
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum PartError {
    #[error("part not found: {0}")]
    NotFound(PathBuf),
    #[error("unsupported part version {found}, expected {expected}")]
    UnsupportedVersion { found: String, expected: u32 },
    #[error("corrupt part: {0}")]
    Corrupt(String),
    #[error(transparent)]
    Codec(#[from] CodecError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("column '{0}' not found in part schema")]
    ColumnNotFound(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::part_writer::PartWriter;
    use crate::string_column::StringColumn;
    use crate::{Block, Column};
    use tempfile::TempDir;

    /// Every failure path returns `PartError`; tests assert on the rendered message.
    fn err(text: &str) -> String {
        parse_schema(text).unwrap_err().to_string()
    }

    fn col(name: &str, dt: DataType) -> (String, DataType) {
        (name.to_string(), dt)
    }

    // ---- `open` fixtures -----------------------------------------------

    /// A temp root plus the part directory inside it. The root must stay alive
    /// for the whole test — bind it, don't discard it.
    fn part_dir() -> (TempDir, PathBuf) {
        let root = TempDir::new().unwrap();
        let dir = root.path().join("part_0");
        (root, dir)
    }

    /// The staging directory `PartWriter` writes into before `finish`.
    fn staging_of(dir: &Path) -> PathBuf {
        let mut name = dir.file_name().unwrap().to_os_string();
        name.push(".tmp");
        dir.with_file_name(name)
    }

    /// One column per `DataType`, so every fixture exercises both the single-file
    /// and the paired-file branch.
    fn sample_schema() -> Vec<(String, DataType)> {
        vec![
            col("id", DataType::Int64),
            col("name", DataType::String),
            col("score", DataType::Float64),
        ]
    }

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

        let mut writer = PartWriter::new(dir.clone(), &sample_schema()).unwrap();
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
    fn open_error(dir: &Path, columns: &[&str]) -> PartError {
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
            vec![
                col("id", DataType::Int64),
                col("name", DataType::String),
                col("score", DataType::Float64),
            ]
        );
        assert_eq!(num_rows, 3);
    }

    /// `PartWriter`'s output for an empty schema — a part with no columns at all.
    #[test]
    fn parses_a_header_only_schema() {
        let (schema, num_rows) = parse_schema("version=1\nnum_rows=0\n").unwrap();

        assert_eq!(schema, vec![]);
        assert_eq!(num_rows, 0);
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
            vec![
                col("zulu", DataType::Float64),
                col("alpha", DataType::String),
                col("mike", DataType::Int64),
            ]
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

        assert_eq!(schema, vec![col("id", DataType::Int64)]);
        assert_eq!(num_rows, 0);
    }

    #[test]
    fn final_newline_is_optional() {
        let (schema, num_rows) = parse_schema("version=1\nnum_rows=1\ncolumn=id:Int64").unwrap();

        assert_eq!(schema, vec![col("id", DataType::Int64)]);
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

        assert_eq!(e.to_string(), "unsupported part version 2, expected 1");
        assert!(matches!(
            e,
            PartError::UnsupportedVersion { ref found, expected: 1 } if found == "2"
        ));
    }

    #[test]
    fn rejects_an_empty_version_value() {
        let e = parse_schema("version=\nnum_rows=0\n").unwrap_err();

        assert!(matches!(
            e,
            PartError::UnsupportedVersion { ref found, expected: 1 } if found.is_empty()
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
            PartError::UnsupportedVersion { ref found, expected: 1 } if found == "1 "
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

        assert_eq!(schema, vec![col("id", DataType::Int64)]);
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

    /// Empty column names are not rejected, though `PartWriter` would emit a file
    /// literally named `.bin` for one. Documents current behavior.
    #[test]
    fn accepts_an_empty_column_name() {
        let (schema, _) = parse_schema("version=1\nnum_rows=1\ncolumn=:Int64\n").unwrap();

        assert_eq!(schema, vec![col("", DataType::Int64)]);
    }

    /// Only the leading `column=` is stripped, so later `=` characters belong to
    /// the name. Documents current behavior.
    #[test]
    fn accepts_a_column_name_containing_an_equals_sign() {
        let (schema, _) = parse_schema("version=1\nnum_rows=1\ncolumn=a=b:Int64\n").unwrap();

        assert_eq!(schema, vec![col("a=b", DataType::Int64)]);
    }

    // ---- cross-line validation -----------------------------------------

    /// Rows without columns to hold them means the schema lines were lost.
    #[test]
    fn rejects_positive_num_rows_with_no_columns() {
        assert!(err("version=1\nnum_rows=5\n").contains("num_rows=5 but schema has no columns"));
    }

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
            vec![col("id", DataType::Int64), col("ID", DataType::Int64)]
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
            matches!(e, PartError::NotFound(ref p) if *p == dir),
            "{e:?}"
        );
        assert!(e.to_string().starts_with("part not found: "));
    }

    /// `is_dir` folds "wrong kind of thing" into the same error as "nothing
    /// there". Documents current behavior.
    #[test]
    fn a_regular_file_in_place_of_a_part_dir_is_not_found() {
        let (_root, dir) = part_dir();
        fs::write(&dir, b"not a part").unwrap();

        let e = open_error(&dir, &["id"]);

        assert!(matches!(e, PartError::NotFound(_)), "{e:?}");
    }

    /// The payoff of the writer's staging directory: a part still being written is
    /// invisible, so a reader never sees a torn one.
    #[test]
    fn an_unfinished_part_is_not_found() {
        let (_root, dir) = part_dir();
        let mut writer = PartWriter::new(dir.clone(), &sample_schema()).unwrap();
        writer.write_block(&sample_block(1)).unwrap();

        let e = open_error(&dir, &["id"]);

        assert!(matches!(e, PartError::NotFound(_)), "{e:?}");
    }

    /// Reaching past the staging directory's name does not get you a readable
    /// part either: `schema.txt` is written only by `finish`.
    #[test]
    fn a_staging_directory_is_not_a_readable_part() {
        let (_root, dir) = part_dir();
        let writer = PartWriter::new(dir.clone(), &sample_schema()).unwrap();
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

        assert!(matches!(e, PartError::Corrupt(_)), "{e:?}");
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

        assert!(matches!(e, PartError::Io(_)), "{e:?}");
    }

    /// One representative case — `parse_schema` itself is covered exhaustively
    /// above; this only pins that `open` propagates rather than swallows.
    #[test]
    fn schema_parse_errors_are_propagated() {
        let (_root, dir) = part_with_schema_text("version=2\nnum_rows=0\ncolumn=id:Int64\n");

        let e = open_error(&dir, &["id"]);

        assert!(
            matches!(e, PartError::UnsupportedVersion { ref found, .. } if found == "2"),
            "{e:?}"
        );
    }

    // ---- open: column resolution ---------------------------------------

    #[test]
    fn an_unknown_column_is_column_not_found() {
        let (_root, dir) = written_part(1);

        let e = open_error(&dir, &["nope"]);

        assert!(
            matches!(e, PartError::ColumnNotFound(ref n) if n == "nope"),
            "{e:?}"
        );
        assert_eq!(e.to_string(), "column 'nope' not found in part schema");
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

    /// A part written with an empty schema is unreadable in every direction: there
    /// is no column to project, and the empty projection is a panic. Pins the gap
    /// the `open` assertion's backlog note refers to.
    #[test]
    fn a_part_with_no_columns_cannot_be_projected() {
        let (_root, dir) = part_dir();
        PartWriter::new(dir.clone(), &[]).unwrap().finish().unwrap();

        assert!(open_err(&dir, &["id"]).contains("column 'id' not found"));
    }

    /// `parse_schema` accepts an empty column name and `PartWriter` emits a file
    /// literally named `.bin` for it, so the round trip holds. Documents current
    /// behavior.
    #[test]
    fn opens_a_column_with_an_empty_name() {
        let (_root, dir) = part_dir();
        PartWriter::new(dir.clone(), &[col("", DataType::Int64)])
            .unwrap()
            .finish()
            .unwrap();

        let reader = PartReader::open(&dir, &[""]).unwrap();

        assert_eq!(reader_indices(&reader), vec![0]);
    }

    // ---- open: corrupt column files ------------------------------------

    #[test]
    fn a_missing_numeric_column_file_is_corrupt() {
        let (_root, dir) = written_part(1);
        fs::remove_file(dir.join("id.bin")).unwrap();

        let e = open_error(&dir, &["id"]);

        assert!(matches!(e, PartError::Corrupt(_)), "{e:?}");
        assert_eq!(e.to_string(), "corrupt part: column file 'id.bin' missing");
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

        assert!(matches!(e, PartError::Io(_)), "{e:?}");
    }
}
