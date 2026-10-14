<script lang="ts">
	import { buildChartModel, describeChart, mergeTableRows, planValueSourceLabel } from '$lib/frontend/chart';
	import type { ForecastPointInput, HistoryPointInput, PlanningPointInput } from '$lib/frontend/chart';
	import { formatQuantity, observationStatusLabel } from '$lib/frontend/format';

	let {
		seriesKey,
		history,
		forecast,
		planning = []
	}: {
		seriesKey: string;
		history: HistoryPointInput[];
		forecast: ForecastPointInput[];
		planning?: PlanningPointInput[];
	} = $props();

	const model = $derived(buildChartModel(history, forecast, undefined, planning));
	const ariaLabel = $derived(describeChart(seriesKey, history, forecast, model, planning));
	const rows = $derived(mergeTableRows(history, forecast, planning));
</script>

<div class="chart-block">
	{#if model.isEmpty}
		<p class="banner banner-info">No numeric observations to plot for this series.</p>
	{:else}
		<svg
			class="chart"
			viewBox="0 0 {model.width} {model.height}"
			role="img"
			aria-label={ariaLabel}
		>
			{#if model.forecastZone}
				<rect
					class="forecast-zone"
					x={model.forecastZone.x}
					y={model.plot.y}
					width={model.forecastZone.width}
					height={model.plot.height}
				/>
			{/if}

			{#each model.valueTicks as tick (tick.value)}
				<line
					class="grid"
					x1={model.plot.x}
					x2={model.plot.x + model.plot.width}
					y1={tick.y}
					y2={tick.y}
				/>
				<text class="axis-label" x={model.plot.x - 8} y={tick.y + 3.5} text-anchor="end">
					{tick.label}
				</text>
			{/each}

			{#if model.zeroY !== null}
				<line
					class="zero"
					x1={model.plot.x}
					x2={model.plot.x + model.plot.width}
					y1={model.zeroY}
					y2={model.zeroY}
				/>
			{/if}

			{#each model.weekTicks as tick (tick.index)}
				<line
					class="tick"
					x1={tick.x}
					x2={tick.x}
					y1={model.plot.y + model.plot.height}
					y2={model.plot.y + model.plot.height + 4}
				/>
				<text
					class="axis-label"
					x={tick.x}
					y={model.plot.y + model.plot.height + 18}
					text-anchor="middle"
				>
					{tick.label}
				</text>
			{/each}

			{#if model.splitX !== null}
				<line
					class="split"
					x1={model.splitX}
					x2={model.splitX}
					y1={model.plot.y}
					y2={model.plot.y + model.plot.height}
				/>
			{/if}

			{#each model.historyPaths as path}
				<path class="history" d={path} />
			{/each}
			{#each model.isolatedHistoryPoints as point}
				<circle class="history-point" cx={point.x} cy={point.y} r="2.6" />
			{/each}

			{#if model.connector}
				<path class="forecast-connector" d={model.connector} />
			{/if}
			{#if planning.length === 0}
				{#if model.forecastPath}
					<path class="forecast" d={model.forecastPath} />
				{/if}
				{#each model.isolatedForecastPoints as point}
					<circle class="forecast-point" cx={point.x} cy={point.y} r="2.6" />
				{/each}
			{/if}

			{#each model.planningBaselinePaths as path}
				<path class="forecast" d={path} />
			{/each}
			{#each model.isolatedPlanningBaselinePoints as point}
				<circle class="forecast-point" cx={point.x} cy={point.y} r="2.6" />
			{/each}

			{#each model.effectivePaths as path}
				<path class="plan" d={path} />
			{/each}
			{#each model.isolatedEffectivePoints as point}
				<circle class="plan-point" cx={point.x} cy={point.y} r="2.6" />
			{/each}
			{#each model.overridePoints as point}
				<circle class="override-point" cx={point.x} cy={point.y} r="3.4" />
			{/each}

			{#if model.forecastZone && model.forecastZone.width > 90}
				<text class="zone-label" x={model.forecastZone.x + 8} y={model.plot.y + 14}>
					Forecast
				</text>
			{/if}
		</svg>

		<ul class="legend">
			<li><span class="swatch swatch-history"></span> History</li>
			<li><span class="swatch swatch-forecast"></span> {planning.length > 0 ? 'Baseline (model, dashed)' : 'Forecast (persisted, dashed)'}</li>
			{#if model.planningWeeks > 0}
				<li><span class="swatch swatch-plan"></span> Plan (effective, solid)</li>
				{#if model.overridePoints.length > 0}
					<li><span class="swatch swatch-override"></span> Manual override</li>
				{/if}
			{/if}
			{#if model.gapCount > 0}
				<li class="muted">
					{model.gapCount} week{model.gapCount === 1 ? '' : 's'} without a numeric value — the history
					line breaks there instead of filling zeros.
				</li>
			{/if}
		</ul>
	{/if}

	<details class="data-table">
		<summary>Data table ({rows.length} rows)</summary>
		<div class="table-wrap">
			<table>
				<thead>
					<tr>
						<th scope="col">Week</th>
						<th scope="col">Series</th>
						<th scope="col">Value</th>
						<th scope="col">Observation</th>
					</tr>
				</thead>
				<tbody>
					{#each rows as row (row.kind + row.week_start)}
						<tr>
							<th scope="row" class="mono">{row.week_start}</th>
							<td>{row.kind === 'history' ? 'History' : row.kind === 'forecast' ? 'Forecast' : 'Plan'}</td>
							<td class="num">{formatQuantity(row.value)}</td>
							<td>
								{#if row.kind === 'history'}
									{observationStatusLabel(row.status)}
								{:else if row.kind === 'plan'}
									{planValueSourceLabel(row.status ?? '')}
								{:else}
									—
								{/if}
							</td>
						</tr>
					{/each}
				</tbody>
			</table>
		</div>
	</details>
</div>

<style>
	.chart-block {
		display: flex;
		flex-direction: column;
		gap: 0.7rem;
	}

	.chart {
		display: block;
		width: 100%;
		height: auto;
		background: var(--surface);
		border: 1px solid var(--border);
		border-radius: var(--radius);
	}

	.forecast-zone {
		fill: var(--violet-soft);
		opacity: 0.55;
	}

	.grid {
		stroke: #e6eaf2;
		stroke-width: 1;
	}

	.tick {
		stroke: var(--border-strong);
		stroke-width: 1;
	}

	.zero {
		stroke: var(--border-strong);
		stroke-width: 1.2;
		stroke-dasharray: 2 3;
	}

	.split {
		stroke: var(--violet);
		stroke-width: 1.2;
		stroke-dasharray: 4 3;
	}

	.history {
		fill: none;
		stroke: var(--cobalt);
		stroke-width: 2;
		stroke-linejoin: round;
		stroke-linecap: round;
	}

	.history-point {
		fill: var(--cobalt);
	}

	.forecast-connector {
		fill: none;
		stroke: var(--violet);
		stroke-width: 2;
		stroke-dasharray: 5 4;
	}

	.forecast {
		fill: none;
		stroke: var(--violet);
		stroke-width: 2;
		stroke-dasharray: 6 4;
		stroke-linejoin: round;
		stroke-linecap: round;
	}

	.forecast-point {
		fill: var(--violet);
	}

	.plan {
		fill: none;
		stroke: var(--success);
		stroke-width: 2;
		stroke-linejoin: round;
		stroke-linecap: round;
	}

	.plan-point {
		fill: var(--success);
	}

	.override-point {
		fill: var(--warning);
		stroke: var(--surface);
		stroke-width: 1.5;
	}

	.axis-label {
		fill: var(--ink-muted);
		font-size: 11px;
		font-family: var(--font-sans);
	}

	.zone-label {
		fill: var(--violet);
		font-size: 11px;
		font-weight: 600;
		font-family: var(--font-sans);
	}

	.legend {
		margin: 0;
		padding: 0;
		list-style: none;
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 0.35rem 1rem;
		font-size: 0.82rem;
	}

	.legend li {
		display: inline-flex;
		align-items: center;
		gap: 0.4rem;
	}

	.swatch {
		display: inline-block;
		width: 1.4rem;
		height: 0;
		border-top: 2px solid var(--cobalt);
	}

	.swatch-forecast {
		border-top-style: dashed;
		border-top-color: var(--violet);
	}

	.swatch-plan {
		border-top-color: var(--success);
	}

	.swatch-override {
		width: 0.6rem;
		height: 0.6rem;
		border-top: 0;
		border-radius: 50%;
		background: var(--warning);
	}

	.data-table summary {
		cursor: pointer;
		font-size: 0.85rem;
		font-weight: 600;
		color: var(--cobalt-strong);
	}

	.table-wrap {
		max-height: 18rem;
		overflow: auto;
		margin-top: 0.5rem;
		border: 1px solid var(--border);
		border-radius: var(--radius-sm);
	}

	table {
		width: 100%;
		border-collapse: collapse;
		font-size: 0.85rem;
	}

	th,
	td {
		padding: 0.3rem 0.55rem;
		text-align: left;
		border-bottom: 1px solid var(--border);
		white-space: nowrap;
	}

	thead th {
		position: sticky;
		top: 0;
		background: var(--surface-subtle);
		color: var(--ink-soft);
		font-size: 0.78rem;
		text-transform: uppercase;
		letter-spacing: 0.04em;
	}

	td.num {
		font-variant-numeric: tabular-nums;
	}
</style>
