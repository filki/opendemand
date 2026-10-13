/**
 * Display formatting for real application values: labels, quantities, scores,
 * timestamps, and identifiers. No fabricated numbers live here.
 */

export function humanizeToken(token: string): string {
	return token
		.split(/[_-]+/)
		.filter(Boolean)
		.map((word) => word.charAt(0).toUpperCase() + word.slice(1))
		.join(' ');
}

export function qualificationLabel(status: string): string {
	switch (status) {
		case 'eligible':
			return 'Eligible';
		case 'insufficient_history':
			return 'Insufficient history';
		case 'invalid_data':
			return 'Invalid data';
		default:
			return humanizeToken(status);
	}
}

export function outcomeLabel(outcome: string | null | undefined): string {
	if (!outcome) return 'Not forecast';
	switch (outcome) {
		case 'ok':
			return 'Forecasted';
		case 'fallback_naive':
			return 'Naive fallback';
		case 'insufficient_history':
			return 'Insufficient history';
		case 'model_failure':
			return 'Model failure';
		case 'invalid_data':
			return 'Invalid data';
		default:
			return humanizeToken(outcome);
	}
}

export function observationStatusLabel(status: string | null | undefined): string {
	if (!status) return '—';
	switch (status) {
		case 'known':
			return 'Known';
		case 'zero':
			return 'Zero';
		case 'missing':
			return 'Missing';
		case 'return':
			return 'Return';
		case 'correction':
			return 'Correction';
		case 'stockout':
			return 'Stockout';
		case 'partial':
			return 'Partial';
		default:
			return humanizeToken(status);
	}
}

export function formatQuantity(value: number | null | undefined, maxDigits = 2): string {
	if (value === null || value === undefined || !Number.isFinite(value)) return '—';
	return new Intl.NumberFormat('en-US', { maximumFractionDigits: maxDigits }).format(value);
}

export function formatScore(value: number | null | undefined): string {
	if (value === null || value === undefined || !Number.isFinite(value)) return '—';
	return value
		.toFixed(3)
		.replace(/0+$/, '')
		.replace(/\.$/, '');
}

export function formatPercent(fraction: number | null | undefined): string {
	if (fraction === null || fraction === undefined || !Number.isFinite(fraction)) return '—';
	const clamped = Math.max(0, Math.min(1, fraction));
	return `${Math.round(clamped * 100)}%`;
}

export function shortId(id: string | null | undefined, length = 8): string {
	if (!id) return '—';
	return id.length <= length + 1 ? id : `${id.slice(0, length)}…`;
}

export function formatTimestamp(value: string | null | undefined): string {
	if (!value) return '—';
	const ms = Date.parse(value);
	if (Number.isNaN(ms)) return value;
	return new Intl.DateTimeFormat('en-US', { dateStyle: 'medium', timeStyle: 'short' }).format(new Date(ms));
}
