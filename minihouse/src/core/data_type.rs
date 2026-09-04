use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataType {
    Int64,
    Float64,
    String,
}

impl DataType {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            DataType::Int64 => "Int64",
            DataType::Float64 => "Float64",
            DataType::String => "String",
        }
    }
}

impl FromStr for DataType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Int64" => Ok(DataType::Int64),
            "Float64" => Ok(DataType::Float64),
            "String" => Ok(DataType::String),
            _ => Err(format!("invalid data type: {}", s)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every variant. Adding one to `DataType` should mean adding it here — the
    /// round-trip test below is only as complete as this list.
    const ALL: [DataType; 3] = [DataType::Int64, DataType::Float64, DataType::String];

    /// The rendered error is part of the corrupt-schema message a reader shows.
    fn err(s: &str) -> String {
        s.parse::<DataType>().unwrap_err()
    }

    // ---- as_str --------------------------------------------------------

    #[test]
    fn as_str_matches_on_disk_schema_tokens() {
        assert_eq!(DataType::Int64.as_str(), "Int64");
        assert_eq!(DataType::Float64.as_str(), "Float64");
        assert_eq!(DataType::String.as_str(), "String");
    }

    // ---- FromStr -------------------------------------------------------

    #[test]
    fn from_str_parses_every_on_disk_schema_token() {
        assert_eq!("Int64".parse::<DataType>().unwrap(), DataType::Int64);
        assert_eq!("Float64".parse::<DataType>().unwrap(), DataType::Float64);
        assert_eq!("String".parse::<DataType>().unwrap(), DataType::String);
    }

    #[test]
    fn from_str_rejects_an_unknown_token() {
        assert_eq!(err("Int32"), "invalid data type: Int32");
    }

    #[test]
    fn from_str_rejects_the_empty_string() {
        assert_eq!(err(""), "invalid data type: ");
    }

    /// The tokens are matched literally, so a differently-cased schema file is
    /// rejected rather than silently reinterpreted.
    #[test]
    fn from_str_is_case_sensitive() {
        for s in ["int64", "INT64", "float64", "string", "STRING"] {
            assert_eq!(err(s), format!("invalid data type: {s}"));
        }
    }

    /// Nothing is trimmed. `parse_schema` hands the type token straight through,
    /// so whitespace damage in a `schema.txt` surfaces here rather than parsing
    /// into the wrong type.
    #[test]
    fn from_str_rejects_surrounding_whitespace() {
        for s in [" Int64", "Int64 ", "\tInt64", "Int64\n"] {
            assert!(s.parse::<DataType>().is_err(), "{s:?} should not parse");
        }
    }

    // ---- round trip ----------------------------------------------------

    /// `as_str` is the write side (`PartWriter::finish`) and `FromStr` the read
    /// side (`part_reader::parse_schema`). If the two ever disagree, every part
    /// written by this version becomes unreadable by it.
    #[test]
    fn every_data_type_round_trips_through_as_str() {
        for dt in ALL {
            assert_eq!(dt.as_str().parse::<DataType>().unwrap(), dt);
        }
    }

    /// Distinct variants must not collapse onto the same on-disk token, or the
    /// round trip above would hold while still losing type information.
    #[test]
    fn data_types_have_distinct_on_disk_tokens() {
        let mut tokens: Vec<&str> = ALL.iter().map(|dt| dt.as_str()).collect();
        tokens.sort_unstable();
        let count = tokens.len();
        tokens.dedup();

        assert_eq!(tokens.len(), count, "duplicate token in {tokens:?}");
    }
}
