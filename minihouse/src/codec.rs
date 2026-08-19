use lz4_flex::block::{compress, decompress, get_maximum_output_size};
use std::io::{Read, Write};
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Codec {
    None = 0,
    Lz4 = 1,
}

impl Codec {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Codec::None => "none",
            Codec::Lz4 => "lz4",
        }
    }
}

impl FromStr for Codec {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "none" => Ok(Codec::None),
            "lz4" => Ok(Codec::Lz4),
            _ => Err(format!("invalid codec: {}", s)),
        }
    }
}

impl TryFrom<u8> for Codec {
    type Error = CodecError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        const _: () = {
            match Codec::None {
                Codec::None | Codec::Lz4 => {}
            }
        };
        match value {
            x if x == Codec::None as u8 => Ok(Codec::None),
            x if x == Codec::Lz4 as u8 => Ok(Codec::Lz4),
            _ => Err(CodecError::Corrupt(format!("unknown codec byte {value}"))),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum CodecError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("corrupt block: {0}")]
    Corrupt(String),
}

pub(crate) const MAX_BLOCK_SIZE: usize = 16 * 1024 * 1024;

pub(crate) fn write_block(w: &mut impl Write, raw: &[u8], codec: Codec) -> Result<(), CodecError> {
    assert!(
        raw.len() <= MAX_BLOCK_SIZE,
        "write_block: {} bytes exceeds MAX_BLOCK_SIZE — caller bug",
        raw.len()
    );

    let compressed_owned;
    let (compressed, codec) = match codec {
        Codec::Lz4 => {
            compressed_owned = compress(raw);
            if compressed_owned.len() >= raw.len() {
                (raw, Codec::None)
            } else {
                (compressed_owned.as_slice(), Codec::Lz4)
            }
        }
        Codec::None => (raw, Codec::None),
    };
    let c_len = u32::try_from(compressed.len()).expect("compressed size exceeds u32");
    let r_len = u32::try_from(raw.len()).expect("raw size exceeds u32");

    w.write_all(&[codec as u8])?;
    w.write_all(&c_len.to_le_bytes())?;
    w.write_all(&r_len.to_le_bytes())?;
    w.write_all(&compressed)?;

    Ok(())
}

pub(crate) fn read_block(r: &mut impl Read) -> Result<Option<Vec<u8>>, CodecError> {
    let mut header = [0u8; 9];
    let mut filled = 0;
    while filled < 9 {
        let n = r.read(&mut header[filled..])?;
        if n == 0 {
            return if filled == 0 {
                Ok(None)
            } else {
                Err(CodecError::Corrupt(format!(
                    "truncated header: got {filled} of 9 bytes"
                )))
            };
        }
        filled += n;
    }

    let codec = Codec::try_from(header[0])?;
    let c_len = u32::from_le_bytes(header[1..5].try_into().unwrap()) as usize;
    let r_len = u32::from_le_bytes(header[5..9].try_into().unwrap()) as usize;

    if r_len > MAX_BLOCK_SIZE {
        return Err(CodecError::Corrupt(format!(
            "raw length {r_len} exceeds MAX_BLOCK_SIZE {MAX_BLOCK_SIZE}"
        )));
    }

    match codec {
        Codec::Lz4 => {
            if c_len > get_maximum_output_size(r_len) {
                return Err(CodecError::Corrupt(format!(
                    "compressed length {c_len} implausible for raw length {r_len}"
                )));
            }

            let mut compressed = vec![0u8; c_len];
            r.read_exact(&mut compressed).map_err(|e| match e.kind() {
                std::io::ErrorKind::UnexpectedEof => {
                    CodecError::Corrupt(format!("truncated body: expected {c_len} bytes"))
                }
                _ => CodecError::Io(e),
            })?;

            let raw = decompress(&compressed, r_len)
                .map_err(|e| CodecError::Corrupt(format!("lz4 decompress failed: {e}")))?;

            Ok(Some(raw))
        }
        Codec::None => {
            if r_len != c_len {
                return Err(CodecError::Corrupt(format!(
                    "raw length {r_len} not equals compressed length {c_len}"
                )));
            }

            let mut raw = vec![0u8; c_len];
            r.read_exact(&mut raw).map_err(|e| match e.kind() {
                std::io::ErrorKind::UnexpectedEof => {
                    CodecError::Corrupt(format!("truncated body: expected {c_len} bytes"))
                }
                _ => CodecError::Io(e),
            })?;

            Ok(Some(raw))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    // ---- helpers -------------------------------------------------------

    fn round_trip(raw: &[u8], codec: Codec) -> Vec<u8> {
        let mut buf = Cursor::new(Vec::new());
        write_block(&mut buf, raw, codec).unwrap();
        buf.set_position(0);
        read_block(&mut buf).unwrap().unwrap()
    }

    fn written(raw: &[u8], codec: Codec) -> Vec<u8> {
        let mut buf = Cursor::new(Vec::new());
        write_block(&mut buf, raw, codec).unwrap();
        buf.into_inner()
    }

    /// Builds a frame header by hand: `[codec][c_len LE][r_len LE]`. Every
    /// crafted-stream test goes through this, so a header change is one edit
    /// here instead of one per test.
    fn header(codec: u8, c_len: u32, r_len: u32) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(9);
        bytes.push(codec);
        bytes.extend_from_slice(&c_len.to_le_bytes());
        bytes.extend_from_slice(&r_len.to_le_bytes());
        bytes
    }

    fn read_err(bytes: Vec<u8>) -> String {
        let mut buf = Cursor::new(bytes);
        read_block(&mut buf).unwrap_err().to_string()
    }

    /// Deterministic bytes with no structure lz4 can exploit. A `wrapping_mul`
    /// over `0..n` repeats every 256 bytes and compresses well despite reading
    /// like noise, so the sequence has to come from a real generator.
    fn incompressible(n: usize) -> Vec<u8> {
        let mut state = 0x2545_f491_4f6c_dd1du64;
        (0..n)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                (state >> 33) as u8
            })
            .collect()
    }

    // ---- Codec conversions ---------------------------------------------

    #[test]
    fn codec_names_each_variant() {
        assert_eq!(Codec::None.as_str(), "none");
        assert_eq!(Codec::Lz4.as_str(), "lz4");
    }

    #[test]
    fn codec_parses_back_from_its_own_name() {
        for codec in [Codec::None, Codec::Lz4] {
            assert_eq!(codec.as_str().parse::<Codec>().unwrap(), codec);
        }
    }

    #[test]
    fn codec_rejects_an_unknown_name() {
        assert_eq!(
            "zstd".parse::<Codec>().unwrap_err(),
            "invalid codec: zstd".to_string()
        );
    }

    /// The name is what lands in `schema.txt`, so parsing must not quietly
    /// accept a spelling `as_str` would never produce.
    #[test]
    fn codec_names_are_case_sensitive() {
        assert!("LZ4".parse::<Codec>().is_err());
        assert!("None".parse::<Codec>().is_err());
    }

    /// These discriminants are the on-disk header byte, not an implementation
    /// detail: changing one silently reinterprets every block ever written.
    #[test]
    fn codec_discriminants_are_the_on_disk_byte_values() {
        assert_eq!(Codec::None as u8, 0);
        assert_eq!(Codec::Lz4 as u8, 1);
    }

    #[test]
    fn codec_converts_from_its_header_byte() {
        assert_eq!(Codec::try_from(0u8).unwrap(), Codec::None);
        assert_eq!(Codec::try_from(1u8).unwrap(), Codec::Lz4);
    }

    #[test]
    fn codec_rejects_an_unknown_header_byte() {
        for byte in [2u8, 255u8] {
            let e = Codec::try_from(byte).unwrap_err();
            assert!(matches!(e, CodecError::Corrupt(_)), "{e:?}");
            assert_eq!(
                e.to_string(),
                format!("corrupt block: unknown codec byte {byte}")
            );
        }
    }

    // ---- round trip ----------------------------------------------------

    #[test]
    fn round_trip_empty_input() {
        for codec in [Codec::None, Codec::Lz4] {
            assert_eq!(round_trip(&[], codec), Vec::<u8>::new(), "{codec:?}");
        }
    }

    #[test]
    fn round_trip_small_input() {
        for codec in [Codec::None, Codec::Lz4] {
            assert_eq!(round_trip(b"hello world", codec), b"hello world".to_vec());
        }
    }

    #[test]
    fn round_trip_highly_compressible_input() {
        let raw = vec![0xABu8; 10_000];
        for codec in [Codec::None, Codec::Lz4] {
            assert_eq!(round_trip(&raw, codec), raw, "{codec:?}");
        }
    }

    #[test]
    fn round_trip_incompressible_input() {
        let raw = incompressible(4096);
        assert!(
            compress(&raw).len() >= raw.len(),
            "fixture is meant to be incompressible"
        );
        for codec in [Codec::None, Codec::Lz4] {
            assert_eq!(round_trip(&raw, codec), raw, "{codec:?}");
        }
    }

    #[test]
    fn round_trip_at_max_block_size_boundary() {
        let raw = vec![0x7u8; MAX_BLOCK_SIZE];
        for codec in [Codec::None, Codec::Lz4] {
            let got = round_trip(&raw, codec);
            assert_eq!(got.len(), MAX_BLOCK_SIZE, "{codec:?}");
            assert_eq!(got, raw, "{codec:?}");
        }
    }

    // ---- write_block ---------------------------------------------------

    #[test]
    #[should_panic(expected = "exceeds MAX_BLOCK_SIZE")]
    fn write_block_panics_when_raw_exceeds_max_block_size() {
        let raw = vec![0u8; MAX_BLOCK_SIZE + 1];
        let mut buf = Cursor::new(Vec::new());
        let _ = write_block(&mut buf, &raw, Codec::None);
    }

    /// Spelled out byte for byte rather than built through `header`, so that a
    /// change to the frame layout has to be made twice before this passes.
    #[test]
    fn write_block_emits_the_exact_frame_layout() {
        let expected: Vec<u8> = [0u8] // codec = none
            .into_iter()
            .chain(7u32.to_le_bytes()) // c_len
            .chain(7u32.to_le_bytes()) // r_len
            .chain(*b"payload") // body, stored verbatim
            .collect();

        assert_eq!(written(b"payload", Codec::None), expected);
    }

    #[test]
    fn write_block_stores_the_lz4_codec_byte_when_compression_helps() {
        let raw = vec![0xABu8; 10_000];
        let bytes = written(&raw, Codec::Lz4);

        assert_eq!(bytes[0], Codec::Lz4 as u8);
        let c_len = u32::from_le_bytes(bytes[1..5].try_into().unwrap()) as usize;
        let r_len = u32::from_le_bytes(bytes[5..9].try_into().unwrap()) as usize;
        assert_eq!(r_len, raw.len());
        assert!(c_len < r_len, "c_len {c_len} should undercut r_len {r_len}");
        assert_eq!(bytes.len(), 9 + c_len);
    }

    /// A one-byte payload cannot survive lz4's framing overhead, so the writer
    /// downgrades to `None`. The codec byte therefore records what was *done*,
    /// not what was *asked for* — which is why the reader never needs to be
    /// told the table's codec.
    #[test]
    fn write_block_falls_back_to_none_when_lz4_would_not_shrink() {
        let bytes = written(b"x", Codec::Lz4);

        let mut expected = header(Codec::None as u8, 1, 1);
        expected.push(b'x');
        assert_eq!(bytes, expected);
        assert_eq!(round_trip(b"x", Codec::Lz4), b"x".to_vec());
    }

    #[test]
    fn write_block_falls_back_to_none_for_empty_input() {
        assert_eq!(written(&[], Codec::Lz4), header(Codec::None as u8, 0, 0));
    }

    #[test]
    fn write_block_never_compresses_under_the_none_codec() {
        let raw = vec![0xABu8; 10_000];
        let mut expected = header(Codec::None as u8, raw.len() as u32, raw.len() as u32);
        expected.extend_from_slice(&raw);

        assert_eq!(written(&raw, Codec::None), expected);
    }

    // ---- read_block ----------------------------------------------------

    #[test]
    fn read_block_returns_none_on_clean_eof() {
        let mut buf = Cursor::new(Vec::new());
        assert!(matches!(read_block(&mut buf), Ok(None)));
    }

    #[test]
    fn read_block_errors_on_truncated_header() {
        let err = read_err(vec![1u8, 2, 3, 4, 5]);
        assert!(err.contains("truncated header: got 5 of 9 bytes"), "{err}");
    }

    #[test]
    fn read_block_errors_on_an_unknown_codec_byte() {
        let err = read_err(header(7, 0, 0));
        assert!(err.contains("unknown codec byte 7"), "{err}");
    }

    #[test]
    fn read_block_errors_when_raw_length_exceeds_max_block_size() {
        let mut bytes = header(Codec::Lz4 as u8, 1, MAX_BLOCK_SIZE as u32 + 1);
        bytes.push(0); // body byte, never read

        let err = read_err(bytes);
        assert!(err.contains("exceeds MAX_BLOCK_SIZE"), "{err}");
    }

    #[test]
    fn read_block_errors_when_compressed_length_implausible() {
        // c_len = 100 cannot be the lz4 encoding of a zero-byte payload.
        let err = read_err(header(Codec::Lz4 as u8, 100, 0));
        assert!(err.contains("implausible for raw length"), "{err}");
    }

    /// The `get_maximum_output_size` bound is meaningful only for lz4 bodies.
    /// An uncompressed block is guarded by `r_len == c_len` instead, so a
    /// mismatch here must report the lengths, not implausibility.
    #[test]
    fn read_block_errors_when_uncompressed_lengths_disagree() {
        let err = read_err(header(Codec::None as u8, 100, 0));
        assert!(
            err.contains("raw length 0 not equals compressed length 100"),
            "{err}"
        );
        assert!(!err.contains("implausible"), "{err}");
    }

    #[test]
    fn read_block_errors_on_truncated_lz4_body() {
        let mut bytes = header(Codec::Lz4 as u8, 10, 1); // c_len plausible for r_len = 1
        bytes.extend_from_slice(&[0u8; 3]); // only 3 of 10 body bytes present

        let err = read_err(bytes);
        assert!(err.contains("truncated body: expected 10 bytes"), "{err}");
    }

    #[test]
    fn read_block_errors_on_truncated_uncompressed_body() {
        let mut bytes = header(Codec::None as u8, 10, 10);
        bytes.extend_from_slice(&[0u8; 3]); // only 3 of 10 body bytes present

        let err = read_err(bytes);
        assert!(err.contains("truncated body: expected 10 bytes"), "{err}");
    }

    #[test]
    fn read_block_errors_on_invalid_lz4_stream() {
        let mut bytes = header(Codec::Lz4 as u8, 1, 1);
        bytes.push(0xFF); // token requiring extension bytes that aren't present

        let err = read_err(bytes);
        assert!(err.contains("lz4 decompress failed"), "{err}");
    }

    /// Blocks carry their own codec byte, so a single stream may freely mix
    /// them — as it does in practice whenever the lz4 fallback kicks in.
    #[test]
    fn read_block_reads_multiple_sequential_blocks_in_order() {
        let mut buf = Cursor::new(Vec::new());
        write_block(&mut buf, b"first", Codec::Lz4).unwrap();
        write_block(&mut buf, b"", Codec::None).unwrap();
        let third = vec![9u8; 500];
        write_block(&mut buf, &third, Codec::Lz4).unwrap();
        let fourth = incompressible(64);
        write_block(&mut buf, &fourth, Codec::None).unwrap();
        buf.set_position(0);

        assert_eq!(read_block(&mut buf).unwrap(), Some(b"first".to_vec()));
        assert_eq!(read_block(&mut buf).unwrap(), Some(Vec::new()));
        assert_eq!(read_block(&mut buf).unwrap(), Some(third));
        assert_eq!(read_block(&mut buf).unwrap(), Some(fourth));
        assert!(matches!(read_block(&mut buf), Ok(None)));
    }

    // Note: CodecError::Io passthrough (write-side `?` operators, and the
    // `_ => CodecError::Io(e)` arm in read_block) is not covered here.
    // Cursor<Vec<u8>> never produces arbitrary io::Error kinds, so exercising
    // this path would require a hand-rolled mock Read/Write.
}
