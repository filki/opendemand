/**
 * Pure geometry for the series chart: history drawn with gaps (no zero fill),
 * a persisted weekly forecast drawn dashed after a split zone, and scales that
 * stay readable for negative and constant series. No DOM, no framework imports,
 * so this module is unit-tested directly with the Node test runner.
 */

export interface HistoryPointInput {
	week_start: string;
	quantity: number | null;
	status: string;
}

export interface ForecastPointInput {
	week_start: string;
	value: number | null;
}

export interface ChartSize {
	width: number;
	height: number;
}

export interface WeekTick {
	index: number;
	x: number;
	label: string;
}

export interface ValueTick {
	value: number;
	y: number;
	label: string;
}

export interface PointMark {
	x: number;
	y: number;
}

export interface ChartModel {
	width: number;
	height: number;
	plot: { x: number; y: number; width: number; height: number };
	isEmpty: boolean;
	weekCount: number;
	historyWeeks: number;
	gapCount: number;
	forecastWeeks: number;
	weekTicks: WeekTick[];
	valueTicks: ValueTick[];
	historyPaths: string[];
	isolatedHistoryPoints: PointMark[];
	forecastPath: string | null;
	isolatedForecastPoints: PointMark[];
	connector: string | null;
	splitX: number | null;
	forecastZone: { x: number; width: number } | null;
	zeroY: number | null;
	domain: { x0: number; x1: number; y0: number; y1: number };
}

export interface TableRow {
	week_start: string;
	kind: 'history' | 'forecast';
	value: number | null;
	status: string | null;
}

const DAY_MS = 86_400_000;
const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];

export function parseIsoDate(iso: string): number | null {
	const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(iso.trim());
	if (!match) return null;
	const year = Number(match[1]);
	const month = Number(match[2]);
	const day = Number(match[3]);
	if (month < 1 || month > 12 || day < 1 || day > 31) return null;
	const ms = Date.UTC(year, month - 1, day);
	const check = new Date(ms);
	if (check.getUTCFullYear() !== year || check.getUTCMonth() !== month - 1 || check.getUTCDate() !== day) {
		return null;
	}
	return ms;
}

/** Weekly index anchored so that 1970-01-05 (a Monday) is week 0. */
export function weekIndex(iso: string): number | null {
	const ms = parseIsoDate(iso);
	if (ms === null) return null;
	return Math.round((ms / DAY_MS - 4) / 7);
}

export function weekIndexToIso(index: number): string {
	const date = new Date((index * 7 + 4) * DAY_MS);
	const year = date.getUTCFullYear();
	const month = String(date.getUTCMonth() + 1).padStart(2, '0');
	const day = String(date.getUTCDate()).padStart(2, '0');
	return `${year}-${month}-${day}`;
}

export function formatWeekLabel(iso: string, includeYear = false): string {
	const ms = parseIsoDate(iso);
	if (ms === null) return iso;
	const date = new Date(ms);
	const label = `${MONTHS[date.getUTCMonth()]} ${date.getUTCDate()}`;
	return includeYear ? `${label}, ${date.getUTCFullYear()}` : label;
}

/** Nice 1/2/5-based tick values covering [min, max]; safe for constant spans. */
export function niceTicks(min: number, max: number, count = 5): number[] {
	if (!Number.isFinite(min) || !Number.isFinite(max)) return [];
	let lo = Math.min(min, max);
	let hi = Math.max(min, max);
	if (lo === hi) {
		const pad = Math.max(Math.abs(lo) * 0.1, 1);
		lo -= pad;
		hi += pad;
	}
	const span = hi - lo;
	const rawStep = span / Math.max(1, count - 1);
	const magnitude = 10 ** Math.floor(Math.log10(rawStep));
	const normalized = rawStep / magnitude;
	const step = (normalized <= 1.5 ? 1 : normalized <= 3 ? 2 : normalized <= 7 ? 5 : 10) * magnitude;
	const ticks: number[] = [];
	const start = Math.ceil(lo / step - 1e-9) * step;
	for (let value = start; value <= hi + step * 1e-9 && ticks.length < 24; value += step) {
		ticks.push(Number(value.toPrecision(12)));
	}
	return ticks;
}

export function formatAxisNumber(value: number): string {
	if (!Number.isFinite(value)) return '—';
	if (Math.abs(value) >= 10_000) {
		return new Intl.NumberFormat('en-US', { notation: 'compact', maximumFractionDigits: 1 }).format(value);
	}
	return new Intl.NumberFormat('en-US', { maximumFractionDigits: 2 }).format(value);
}

export function buildChartModel(
	history: HistoryPointInput[],
	forecast: ForecastPointInput[],
	size: ChartSize = { width: 760, height: 300 }
): ChartModel {
	const margin = { top: 18, right: 18, bottom: 32, left: 54 };
	const plot = {
		x: margin.left,
		y: margin.top,
		width: Math.max(10, size.width - margin.left - margin.right),
		height: Math.max(10, size.height - margin.top - margin.bottom)
	};

	const preparedHistory = history
		.map((point) => ({ idx: weekIndex(point.week_start), quantity: point.quantity }))
		.filter((point): point is { idx: number; quantity: number | null } => point.idx !== null)
		.sort((a, b) => a.idx - b.idx);
	const preparedForecast = forecast
		.map((point) => ({ idx: weekIndex(point.week_start), value: point.value }))
		.filter((point): point is { idx: number; value: number } => point.idx !== null && point.value !== null && Number.isFinite(point.value))
		.sort((a, b) => a.idx - b.idx);

	const base: ChartModel = {
		width: size.width,
		height: size.height,
		plot,
		isEmpty: true,
		weekCount: 0,
		historyWeeks: preparedHistory.length,
		gapCount: 0,
		forecastWeeks: preparedForecast.length,
		weekTicks: [],
		valueTicks: [],
		historyPaths: [],
		isolatedHistoryPoints: [],
		forecastPath: null,
		isolatedForecastPoints: [],
		connector: null,
		splitX: null,
		forecastZone: null,
		zeroY: null,
		domain: { x0: 0, x1: 1, y0: 0, y1: 1 }
	};

	const allIndices = [...preparedHistory.map((point) => point.idx), ...preparedForecast.map((point) => point.idx)];
	if (allIndices.length === 0) return base;

	const x0 = Math.min(...allIndices);
	let x1 = Math.max(...allIndices);
	if (x0 === x1) x1 = x0 + 1;

	const values: number[] = [];
	for (const point of preparedHistory) {
		if (point.quantity !== null && Number.isFinite(point.quantity)) values.push(point.quantity);
	}
	for (const point of preparedForecast) {
		if (Number.isFinite(point.value)) values.push(point.value);
	}
	if (values.length === 0) {
		return { ...base, weekCount: x1 - x0 + 1, domain: { x0, x1, y0: 0, y1: 1 } };
	}

	let lo = Math.min(...values);
	let hi = Math.max(...values);
	if (lo === hi) {
		const pad = Math.max(Math.abs(lo) * 0.1, 1);
		lo -= pad;
		hi += pad;
	} else {
		const pad = (hi - lo) * 0.08;
		lo -= pad;
		hi += pad;
	}

	const spanX = x1 - x0;
	const weekWidth = plot.width / spanX;
	const xScale = (idx: number) => plot.x + ((idx - x0) / spanX) * plot.width;
	const yScale = (value: number) => plot.y + plot.height - ((value - lo) / (hi - lo)) * plot.height;

	const historyPaths: string[] = [];
	const isolatedHistoryPoints: PointMark[] = [];
	let segment: PointMark[] = [];
	let gapCount = 0;
	const flush = () => {
		if (segment.length >= 2) historyPaths.push(toPath(segment));
		else if (segment.length === 1) isolatedHistoryPoints.push(segment[0]);
		segment = [];
	};
	for (const point of preparedHistory) {
		if (point.quantity === null || !Number.isFinite(point.quantity)) {
			gapCount += 1;
			flush();
			continue;
		}
		segment.push({ x: xScale(point.idx), y: yScale(point.quantity) });
	}
	flush();

	const forecastMarks: PointMark[] = preparedForecast
		.filter((point) => Number.isFinite(point.value))
		.map((point) => ({ x: xScale(point.idx), y: yScale(point.value) }));
	const forecastPath = forecastMarks.length >= 2 ? toPath(forecastMarks) : null;
	const isolatedForecastPoints = forecastMarks.length === 1 ? forecastMarks : [];

	const firstForecast = preparedForecast.find((point) => Number.isFinite(point.value)) ?? null;
	const lastHistoryEntry = preparedHistory.length > 0 ? preparedHistory[preparedHistory.length - 1] : null;
	let connector: string | null = null;
	if (
		firstForecast !== null &&
		lastHistoryEntry !== null &&
		lastHistoryEntry.quantity !== null &&
		Number.isFinite(lastHistoryEntry.quantity) &&
		firstForecast.idx === lastHistoryEntry.idx + 1
	) {
		connector = toPath([
			{ x: xScale(lastHistoryEntry.idx), y: yScale(lastHistoryEntry.quantity) },
			{ x: xScale(firstForecast.idx), y: yScale(firstForecast.value) }
		]);
	}

	let splitX: number | null = null;
	let forecastZone: { x: number; width: number } | null = null;
	if (firstForecast !== null) {
		splitX = Math.max(plot.x, Math.min(plot.x + plot.width, xScale(firstForecast.idx) - weekWidth / 2));
		forecastZone = { x: splitX, width: plot.x + plot.width - splitX };
	}

	const valueTicks = niceTicks(lo, hi, 5).map((value) => ({
		value,
		y: yScale(value),
		label: formatAxisNumber(value)
	}));
	const weekTicks = buildWeekTicks(x0, x1, plot, xScale, firstForecast?.idx ?? null);
	const zeroY = lo <= 0 && 0 <= hi ? yScale(0) : null;

	return {
		...base,
		isEmpty: false,
		weekCount: spanX + 1,
		gapCount,
		historyPaths,
		isolatedHistoryPoints,
		forecastPath,
		isolatedForecastPoints,
		connector,
		splitX,
		forecastZone,
		zeroY,
		valueTicks,
		weekTicks,
		domain: { x0, x1, y0: lo, y1: hi }
	};
}

function buildWeekTicks(
	x0: number,
	x1: number,
	plot: { x: number; width: number },
	xScale: (idx: number) => number,
	forecastStartIdx: number | null
): WeekTick[] {
	const spanX = x1 - x0;
	const maxTicks = Math.max(2, Math.min(8, Math.floor(plot.width / 120) + 1));
	const indices = new Set<number>([x0, x1]);
	for (let i = 1; i < maxTicks - 1; i += 1) {
		indices.add(x0 + Math.round((spanX * i) / (maxTicks - 1)));
	}
	if (forecastStartIdx !== null && forecastStartIdx > x0 + 1 && forecastStartIdx < x1 - 1) {
		indices.add(forecastStartIdx);
	}
	const sorted = [...indices].sort((a, b) => a - b);
	let previousYear: number | null = null;
	return sorted.map((index, position) => {
		const iso = weekIndexToIso(index);
		const year = new Date((index * 7 + 4) * DAY_MS).getUTCFullYear();
		const includeYear = position === 0 || year !== previousYear;
		previousYear = year;
		return { index, x: xScale(index), label: formatWeekLabel(iso, includeYear) };
	});
}

function toPath(points: PointMark[]): string {
	return points
		.map((point, index) => `${index === 0 ? 'M' : 'L'} ${round2(point.x)} ${round2(point.y)}`)
		.join(' ');
}

function round2(value: number): number {
	return Math.round(value * 100) / 100;
}

/** Accessible one-sentence summary for the chart's aria-label. */
export function describeChart(
	seriesKey: string,
	history: HistoryPointInput[],
	forecast: ForecastPointInput[],
	model: ChartModel
): string {
	const numericHistory = history.filter((point) => point.quantity !== null && Number.isFinite(point.quantity));
	const parts: string[] = [];
	if (history.length > 0) {
		const weeks = history.map((point) => point.week_start).sort();
		parts.push(
			`History for ${seriesKey}: ${history.length} weeks from ${weeks[0]} to ${weeks[weeks.length - 1]}, ` +
				`${numericHistory.length} plotted points and ${model.gapCount} week${model.gapCount === 1 ? '' : 's'} without a numeric value.`
		);
	} else {
		parts.push(`No history recorded for ${seriesKey}.`);
	}
	if (forecast.length > 0) {
		const weeks = forecast.map((point) => point.week_start).sort();
		parts.push(
			`Forecast: ${forecast.length} weekly points from ${weeks[0]} to ${weeks[weeks.length - 1]}, drawn dashed after the split.`
		);
	} else {
		parts.push('No persisted forecast in the selected run.');
	}
	return parts.join(' ');
}

/** Chronological rows for the accessible data table under the chart. */
export function mergeTableRows(history: HistoryPointInput[], forecast: ForecastPointInput[]): TableRow[] {
	const rows: TableRow[] = [
		...history.map((point) => ({
			week_start: point.week_start,
			kind: 'history' as const,
			value: point.quantity,
			status: point.status
		})),
		...forecast.map((point) => ({
			week_start: point.week_start,
			kind: 'forecast' as const,
			value: point.value,
			status: null
		}))
	];
	return rows.sort((a, b) => {
		const ia = weekIndex(a.week_start) ?? 0;
		const ib = weekIndex(b.week_start) ?? 0;
		if (ia !== ib) return ia - ib;
		if (a.kind === b.kind) return 0;
		return a.kind === 'history' ? -1 : 1;
	});
}
