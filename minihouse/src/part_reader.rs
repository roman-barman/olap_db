use crate::DataType;
use crate::codec::CodecError;
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

    /// Every failure path returns `PartError`; tests assert on the rendered message.
    fn err(text: &str) -> String {
        parse_schema(text).unwrap_err().to_string()
    }

    fn col(name: &str, dt: DataType) -> (String, DataType) {
        (name.to_string(), dt)
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
}
