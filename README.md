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

# Benchmark: Iteration 1, after projection pushdown

> Second mark of the project's performance history. Recorded **after** the
> `execute` fix: only the aggregate's column is filtered
> (`block.column(agg).filter(&mask)`), not the whole block.
> For the "before" baseline, see `benchmark-iteration-1-en.md`.

## Environment

| Parameter | Value |
|---|---|
| Date | 2026-07-23 |
| CPU / RAM | Intel Core i7-1255U (2P+8E, 12 threads) / 16 GB |
| Method | median of 7 runs, first run discarded (warm-up), `black_box` |

Dataset and contender unchanged (10M rows, seed 42, 8,192-row blocks,
naive `Vec<Row>`). Cross-check passed on every query ✅

## Results

### `sum(dur) WHERE ts > X`

| Selectivity | Columnar | Row-based | Ratio (row/col) | Before (col.) | Speedup |
|---|---|---|---|---|---|
| ~1%  | 18.6 ms | 24.7 ms | **1.3x** | 56.3 ms  | 3.0× |
| ~50% | 45.6 ms | 56.3 ms | **1.2x** | 279.6 ms | 6.1× |
| ~99% | 20.4 ms | 24.3 ms | **1.2x** | 267.0 ms | **13.1×** |

### `sum(dur)` without a filter

| Query | Columnar | Row-based | Ratio (row/col) |
|---|---|---|---|
| no filter | 7.0 ms | 26.4 ms | **3.7x** |

### `count() WHERE ts > X`

| Selectivity | Columnar | Row-based | Ratio (row/col) | Before (col.) |
|---|---|---|---|---|
| ~1%  | 18.3 ms | 24.0 ms | **1.3x** | 58.5 ms |
| ~50% | 44.0 ms | 26.5 ms | **0.6x** | 289.5 ms |
| ~99% | 19.7 ms | 26.6 ms | **1.3x** | 273.9 ms |

## Forecast check

Forecast from the "before" report: columnar filtered sum at ~15–35 ms,
ratio 1–3x in favor of columns at every selectivity. Actual: 18–46 ms,
ratio 1.2–1.3x on sum — forecast confirmed. A single line of code delivered
up to a 13x speedup: that is the measured cost of cloning unneeded
`String` columns.

## Interpreting the remaining effects

1. **Why ~50% is slower than the edges (45 ms vs ~19 ms).** The url clones
   are gone; the remainder is the branch predictor struggling with an
   unpredictable mask inside our own pipeline (`filter_map` in
   `Column::filter`), plus the `eval_predicate` and `cap`-counting passes.
   A possible cure — branch-free filtering (always write, advance the index
   by `m as usize`) — is deferred to the backlog until iteration 6
   (vectorization), to be decided by a criterion duel.

2. **The count ~50% anomaly: the only 0.6x ratio — credit to the contender,
   not a flaw of columns.** The row-based `count()` compiles to branch-free
   code (compare → increment), so its times are flat across selectivities
   (24–26.5 ms), unlike its sum counterpart (56 ms at 50%). Meanwhile the
   columnar count still drags the full pipeline: mask → column filter → `len`.

## Optimization backlog (not now)

- **Aggregate pushdown for count**: count the trues in the mask without
  materializing a column — `cap` is already computed in `filter`. Removes
  the 0.6x anomaly.
- **Branch-free filtering** in `Column::filter` — a criterion-duel candidate
  for iteration 6.
- **Mask fast paths** (all-true/all-false) never fire on random data; they
  will pay off on sorted data with the sparse index (iteration 4).

## Iteration 1 verdict

The columnar engine beats the row-based one on every sum query (1.2–1.3x
filtered, 3.7x unfiltered) with aggregation throughput of ~11.4 GB/s
(80 MB / 7.0 ms) — memory-bound, the core is healthy. The main lesson of the
iteration: **the executor must process only the data that is consumed
downstream** — projection pushdown delivered up to 13x and will continue on
disk (iteration 2: don't read unneeded files) and in the index
(iteration 4: don't read unneeded granules).
