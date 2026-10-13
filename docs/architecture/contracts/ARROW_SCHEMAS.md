# Milestone 1 Arrow Schemas

Use Arrow IPC **file** format, not stream format. Every schema has metadata
`opendemand.contract=opendemand.arrow/1` and `opendemand.table=<table name>`.
Fields below are non-null unless marked nullable. Dates are `date32` Monday dates.
UUID fields are canonical UTF-8 UUID strings; SQLite still stores UUIDv7 `BLOB(16)`.

## Input

`observations.arrow` (`observations`):

```text
product_id: utf8
location_id: utf8
week_start: date32
quantity: float64 nullable
status: utf8
```

`tasks.arrow` (`tasks`):

```text
task_id: utf8
product_id: utf8
location_id: utf8
model_id: utf8
kind: utf8                         # production | backtest
origin_index: int32 nullable       # 1..4 only for backtest
train_start_week: date32
train_end_week: date32
forecast_start_week: date32
train_size: int32
horizon_weeks: int32               # exactly 12
```

Each task identifies a consecutive numeric training segment. Forecast starts exactly
one week after train end. Rust plans `k=max(0,min(4,floor((H-f)/12)))` origins.
Origin 1 is earliest; indices increase chronologically per series/model.

## Output

`task_status.arrow` (`task_status`):

```text
task_id: utf8
status: utf8                       # completed | failed
failure_code: utf8 nullable
failure_detail: utf8 nullable
```

Exactly one row for every input task. Failed tasks have a code and no metrics/points.

`origin_metrics.arrow` (`origin_metrics`):

```text
task_id: utf8
origin_week_start: date32
train_size: int32
wape: float64 nullable
mae: float64
metric_kind: utf8                  # wape | mae_fallback
metric_value: float64
```

Exactly one row per completed backtest. `origin_week_start` equals the task's
`train_end_week` (the last training Monday), not the first forecast Monday.
WAPE is `sum(abs(actual-predicted))/sum(abs(actual))`.
When denominator is zero, WAPE is null and selection uses MAE instead.
Otherwise selection uses WAPE. MAE is always retained. All metrics are nonnegative and finite.

`forecast_points.arrow` (`forecast_points`):

```text
task_id: utf8
week_start: date32
value: float64
```

Exactly 12 unique weekly points per completed task. Negative forecasts are valid
because negative net-demand observations are valid. NaN and infinity are invalid.

## Validation and Persistence

Rust verifies file hashes, schema metadata/types/nullability, exact task coverage,
unknown/duplicate task ids, origin geometry, metric consistency, and horizon completeness.
Files have fixed names; zero-row output files still carry their full schema.
Result row counts must match the IPC files. Rust maps task ids to planned series keys;
Python cannot introduce new products, locations, models, or origins.

Rust ranks each series independently, with at least two completed origins required
for champion eligibility. The champion flag records the best evaluation; selected
model records the model actually used after production fallback. Origin summaries
remain stored even when per-point backtests are omitted. All validated results are
persisted in one transaction against the pinned import revision.
