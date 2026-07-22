# Benchmark: Iteration 1, baseline (before projection pushdown)

> Zero mark of the project's performance history. Recorded **before** fixing
> `execute` (it filters every column of the block instead of the single one needed).

## Environment

| Parameter | Value |
|---|---|
| Date | 2026-07-22 |
| CPU / RAM | Intel Core i7-1255U (2P+8E, 12 threads) / 16 GB |
| Method | median of 7 runs, first run discarded (warm-up), `black_box` |

## Dataset

- 10,000,000 rows, deterministic seed = 42
- Schema: `id: Int64` (sequential), `ts: Int64` (uniform 0..1M), `url: String` (`/page/{0..1000}`), `dur: Int64` (1..10,000)
- Columnar table: blocks of 8,192 rows (1,220 full blocks + a 5,440-row tail)
- Contender: naive `Vec<Row>` with a straightforward loop
- Cross-check: both engines returned identical answers on every query ✅

## Results

### `sum(dur) WHERE ts > X`

| Selectivity | Columnar | Row-based | Ratio (row/col) |
|---|---|---|---|
| ~1%  | 56.3 ms  | 25.6 ms | **0.5x** |
| ~50% | 279.6 ms | 45.8 ms | **0.2x** |
| ~99% | 267.0 ms | 27.0 ms | **0.1x** |

### `sum(dur)` without a filter

| Query | Columnar | Row-based | Ratio (row/col) |
|---|---|---|---|
| no filter | 6.8 ms | 23.4 ms | **3.4x** |

Columnar aggregation throughput: 80 MB / 6.8 ms ≈ **11.7 GB/s** —
memory speed; SIMD is working, the aggregation core is healthy.

### `count() WHERE ts > X`

| Selectivity | Columnar | Row-based | Ratio (row/col) |
|---|---|---|---|
| ~1%  | 58.5 ms  | 21.8 ms | **0.4x** |
| ~50% | 289.5 ms | 26.1 ms | **0.1x** |
| ~99% | 273.9 ms | 21.8 ms | **0.1x** |

## Interpretation

**The columnar engine loses on every filtered query — and this is the measured
cost of a known issue, not of the architecture.**

1. **Diagnosis.** `execute` calls `Block::filter(&mask)`, which copies
   **all four** columns of the block, including the heavy `url: String`.
   Filtering a string column means a `clone()` (an allocation) for every
   surviving row. At 99% selectivity that is ~9.9M `String` allocations whose
   results nobody uses: `sum(dur)` only looks at `dur`, and `count` only at
   the length.

2. **Confirmation from the numbers.** Without a filter (no `Block::filter`)
   the columnar engine is 3.4x faster. With a filter, time explodes from
   6.8 ms to 270–290 ms — even though the `ts` pass itself (building the mask)
   costs ~7 ms. The difference is the cost of cloning columns nobody needs.

3. **Selectivity dependence.** 1% (56 ms) is faster than 50–99% (~280 ms)
   because there are proportionally fewer clones. The row engine shows its own
   effect: 50% (45.8 ms) is slower than the edges (~26 ms) — the `r.ts > x`
   branch is unpredictable there, and branch-predictor misses take their toll.

4. **The fast paths in `Column::filter`** (all-true / all-false mask) never
   fired on random data — as expected; they are designed for sorted data and
   will pay off with the sparse index (iteration 4).

## Planned fix

Projection pushdown in `execute`: filter only the aggregate's column instead
of the whole block — `block.column(agg).filter(&mask)`. `Block::filter` stays
in the API as the general mechanism for consumers that need several columns.

Post-fix forecast: columnar filtered sum at ~15–35 ms,
ratio 1–3x in favor of columns at every selectivity, peaking at ~1%.

## Lessons

- The benchmark paid for itself on the very first run: it turned a code-review
  note ("all columns get filtered") into a measured cost of ~270 ms.
- "The executor must filter only what is consumed downstream" — a hands-on
  discovery of projection pushdown, one of the key optimizations in analytical
  databases. To be continued in iteration 2: don't read unneeded files from disk.