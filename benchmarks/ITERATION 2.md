# Benchmark: Iteration 2, writing the on-disk tables

> Write-side results of the disk benchmark: insert timings, on-disk sizes,
> per-file compression breakdown, and codec-fallback evidence read from disk.
> Read-side matrix (cold/warm × None/LZ4) follows in a separate report.

## Environment

| Parameter | Value |
|---|---|
| Date | 2026-08-19 |
| CPU / RAM | Intel Core i7-1255U (2P+8E, 12 threads) / 16 GB |

Dataset: 10M rows, seed 42, 8,192-row blocks (1,220 full + tail), schema
`id: Int64 (sequential), ts: Int64 (uniform 0..1M), url: String (/page/{0..1000}), dur: Int64 (1..10K)`.
Each table is written as **one part** via a single `insert(&blocks)` —
fragmentation cost is deliberately excluded (it is iteration 3's subject).

## Insert timings

| Table | Time | Throughput (~220 MB raw) |
|---|---|---|
| Codec::None | 266 ms | ~830 MB/s |
| Codec::Lz4  | 704 ms | ~310 MB/s |

## On-disk sizes

| Table | Size | Ratio |
|---|---|---|
| None | 369.0 MB | 1.00 |
| LZ4  | 199.1 MB | **1.85× smaller** |

### Per-file breakdown (part_0000)

| File | None | LZ4 | Compression | Why |
|---|---|---|---|---|
| id.bin | 77 MB | 39 MB | 2.0× | sequential i64: 6–7 shared high bytes between neighbors — LZ4 finds the windows |
| ts.bin | 77 MB | 46 MB | 1.7× | uniform 0..1M in i64 → **5 high bytes are always zero**; LZ4 eats zeros |
| dur.bin | 77 MB | 38 MB | 2.0× | 0..10K in i64 → 6 zero bytes per value |
| url.data.bin | 85 MB | 32 MB | 2.7× | 1,000 repeated urls — but the LZ4 dictionary lives inside one 8,192-row block, capping the win |
| url.offsets.bin | 39 MB | 39 MB | **1.0×** | ascending u32 with variable step: no byte repeats — LZ4 gets nothing |
| schema.txt | 4 KB | 4 KB | — | |

Header overhead: 4,880 blocks × 9 bytes ≈ 43 KB per numeric column — 0.05%,
the fixed 9-byte header is effectively free.

## Codec fallback: verified on disk

`write_block` falls back to None when `compress(raw).len() >= raw.len()`,
and the block's codec byte records the **actual** codec. Read straight from
the LZ4 table with `xxd -l 1`:

| File (lz4 table) | First codec byte | Meaning |
|---|---|---|
| url.offsets.bin | `00` | **fallback fired** — incompressible, stored as None |
| url.data.bin | `01` | compressed as LZ4 |

Corroborated by sizes: url.offsets.bin is byte-identical in both tables
(39 MB). The format is self-describing per block; the reader needs no table-
level codec knowledge. (Strictly, `xxd -l 1` shows the first of 1,220 blocks;
identical file sizes make per-block fallback the only plausible story.)

## Takeaways

1. **Compression is a CPU-for-bytes trade, and page cache hides the byte
   savings at write time** — LZ4 insert is 2.6× slower with zero write-time
   benefit; the payoff, if any, is at read time.
2. **Number "compression" here is mostly zero-byte padding**: i64 for values
   under a million wastes 5 bytes/value, and LZ4 merely claws that back.
   Delta / narrower types (backlog, iteration 6+) attack the same waste
   directly — measured motivation: id at 2.0× vs Delta's potential ~10–20×.
3. **url.offsets is the ideal Delta client and LZ4's blind spot** — ascending
   values, zero LZ4 gain, fallback fires. First concrete target for custom
   codecs.
4. **Block-local dictionaries cap string compression** (2.7× vs naive 5–10×
   expectation): compression ratio depends on block size — a tunable to
   revisit.

# Benchmark: Iteration 2, subtask 2.6 — full-scan read speed

> Read-side, part 1: full table scan (all four columns) over both on-disk
> tables, cold and warm. Companion to the write-side report.
> The projected-query matrix via `execute` follows separately.

## Environment

| Parameter | Value |
|---|---|
| Date | 2026-08-21 |
| CPU / RAM | Intel Core i7-1255U (2P+8E, 12 threads) / 16 GB |

Method: scan of all columns (`id, ts, url, dur`) through `Table::scan`,
accumulating `num_rows` under `black_box`; `Table::open` outside the timed
region. Cold = first run after process start; warm = subsequent run
(page cache hot). Tables: one part each, 10M rows (see write-side report).

## Results

| Run | None (369 MB) | LZ4 (199 MB) | Faster |
|---|---|---|---|
| Cold | 449.0 ms | 279.8 ms | **LZ4, 1.6×** |
| Warm | 149.0 ms | 201.1 ms | **None, 1.35×** |

## Interpretation

1. **Cold: compression pays exactly where write-time hid it.** Reading from
   disk, None hauls 369 MB, LZ4 hauls 199 MB; the byte savings outweigh
   decompression cost. The write-side lesson inverts on the read side:
   CPU-for-bytes is a losing trade into the page cache and a winning one
   out of the disk.

2. **Warm: roles flip.** With all bytes in RAM, None is pure block assembly
   (149 ms); LZ4 adds ~52 ms of decompression over ~200 MB compressed —
   ~4 GB/s, LZ4's nameplate decompression speed observed live.

3. **Codec choice is a bet on the cache profile of the workload**: hot
   working sets favor None, cold scans favor LZ4. (Production systems
   default to LZ4 because working sets rarely fit in RAM at scale — and
   storage bytes cost money regardless.)

4. **Warm-None throughput ≈ 2.5 GB/s end-to-end** (369 MB / 149 ms) — well
   below memory bandwidth (~11.7 GB/s reference). Block assembly (column
   allocation, StringColumn reconstruction from chunks, per-block reader
   plumbing) is now a visible cost center, not just memcpy. Profiling
   candidate — backlog, not now.

# Iteration 2 — Final Report: On-Disk Storage

> Closing report. The engine went from in-memory columns to a persistent
> columnar store: string columns as `data + offsets`, an LZ4/None block
> codec with per-block self-description and fallback, immutable parts
> written atomically (tmp + rename), projected reads, and a Table that
> survives process restarts. All correctness gates (cross-checks against
> the independent row engine) passed on every measurement below.

## Environment

| Parameter | Value |
|---|---|
| Date | 2026-08-25 |
| CPU / RAM | Intel Core i7-1255U (2P+8E, 12 threads) / 16 GB |
| Dataset | 10M rows, seed 42, blocks of 8,192; one part per table |

## Storage sizes

| Format | Size | vs None |
|---|---|---|
| CSV (naive text) | 295.6 MB | 0.80× |
| Binary, Codec::None | 369.0 MB | 1.00× |
| Binary, Codec::Lz4 | 199.1 MB | **0.54×** |

Notable: naive text is *smaller* than uncompressed binary — i64 padding
(5–6 zero bytes per value at these ranges) costs more than ASCII digits
with commas. Only LZ4 beats both. Per-file: url.data 2.7×, id/dur 2.0×,
ts 1.7×, **url.offsets 1.0× — fallback fired, stored as None** (verified
by codec byte on disk: `00`). Details: write-side report.

## Write

| Codec | Insert 10M rows | Note |
|---|---|---|
| None | 266 ms (~830 MB/s) | page cache absorbs I/O |
| Lz4 | 704 ms | +438 ms = pure compress() at ~0.5 GB/s |

Compression shows zero benefit at write time (bytes saved are invisible
behind the page cache); the entire cost is CPU.

## Read: full scan, all columns

| Run | None | LZ4 | Winner |
|---|---|---|---|
| Cold | 449 ms | 280 ms | **LZ4 1.6×** |
| Warm | 149 ms | 201 ms | **None 1.35×** |

The iteration's headline: **codec choice is a bet on cache profile** —
LZ4 wins cold (fewer disk bytes), None wins warm (no decompression).

## Read: query matrix (warm; cold characterized by full scan above)

`execute` with projection, medians of 6 runs after warm-up:

| Query | Memory (iter-1) | Disk None | Disk LZ4 | Row engine |
|---|---|---|---|---|
| sum, no filter | 7.0 | 11.5 | 30.2 | ~20 |
| sum, ~1% | 18.6 | 28.8 | 65.6 | 22.4 |
| sum, ~50% | 45.6 | 58.2 | 96.7 | 46.2 |
| sum, ~99% | 20.4 | 35.0 | 75.5 | 22.6 |
| count, ~1/50/99% | 18–44 | 38–61 | 91–122 | ~21 |
| count url == const | 48.4 | 47.2 | 106.6 | 32.6 |

Readings:

1. **Persistence tax at warm None: ~1.5×** over iteration-1 memory
   (block assembly through the PartReader pipeline; warm-None end-to-end
   assembly ≈ 2.5 GB/s vs 11.7 GB/s raw memory).
2. **LZ4 warm pays full decompression per query** — deltas over None
   match LZ4's ~4 GB/s nameplate on the columns read (e.g. no-filter:
   +18.7 ms ≈ 77 MB / 4 GB/s). Decompressed data is cached nowhere;
   an uncompressed-block cache is the standard cure (backlog).
3. **The row engine now leads all filtered warm queries** — its data
   sits in RAM for free. This is the honest price of persistence, and
   the cures are measured, not hypothetical: read less (sparse index,
   iteration 4), assemble cheaper (buffer reuse, backlog), skip columns
   entirely (count via mask — its disk multiplier is now visible: count
   drags the id column it never looks at).

## Backlog (accumulated, with measured motivation)

- **Sparse index (iteration 4)**: the only cure for reading fewer bytes.
- **Uncompressed-block cache**: removes per-query decompression at warm LZ4.
- **Aggregate pushdown (count via mask)**: −1 column of disk reads.
- **Delta/narrow types**: ts/dur/id compress 1.7–2× via zero-padding only;
  url.offsets (1.0×) is the ideal Delta client.
- **Block assembly cost** (2.5 GB/s ceiling): buffer reuse, profiling.
- Long strings (chunk-to-block mapping via marks), QueryError split,
  parser core dedup, cap threading — from earlier reviews.

## Iteration verdict

The engine is now a real, restart-surviving columnar store with an honest
measured profile: 1.85× storage compression, sub-second writes, cold reads
where compression pays and warm reads where its cost is exposed. Every
architectural IOU (index, merges, caches) now has a price tag attached.
Next: **iteration 3 — parts and merges**, whose entry condition this
iteration deliberately created: many small immutable parts and a table
that only ever grows by adding them.