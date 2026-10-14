/**
 * Pure logic for the guided import wizard: descriptor building, stage
 * transitions, and commit gating. The Svelte component renders these results;
 * keeping them here makes them testable without a DOM.
 */
import type { ImportPreview, ParsingDescriptor, PreviewFinding } from '#lib/bindings.ts';

export type WizardStage = 'pick' | 'configure' | 'review';

export const IMPORT_ENCODINGS = [
	'utf-8',
	'windows-1250',
	'windows-1252',
	'iso-8859-2',
	'utf-16le',
	'utf-16be'
] as const;

export const IMPORT_DELIMITERS = [';', ',', '\t', '|'] as const;

export const DATE_FORMAT_CHOICES = [
	'%Y-%m-%d',
	'%d.%m.%Y',
	'%d/%m/%Y',
	'%Y%m%d',
	'%m/%d/%Y'
] as const;

/** Formats pre-selected for a new file: unambiguous formats only. */
export const DEFAULT_DATE_FORMATS = ['%Y-%m-%d', '%d.%m.%Y', '%d/%m/%Y', '%Y%m%d'];

export const DEFAULT_KEY_SEPARATOR = '|';

const THOUSANDS_SEPARATORS = ['.', ',', ' '] as const;

export interface WizardForm {
	encoding: string;
	delimiter: string;
	headerRow: number;
	sheet: string;
	keyColumns: string[];
	keySeparator: string;
	dateColumn: string;
	dateFormats: string[];
	quantityColumn: string;
	decimalSeparator: string;
	thousandsSeparator: string;
}

export function emptyForm(): WizardForm {
	return {
		encoding: 'utf-8',
		delimiter: ',',
		headerRow: 1,
		sheet: '',
		keyColumns: [],
		keySeparator: DEFAULT_KEY_SEPARATOR,
		dateColumn: '',
		dateFormats: [...DEFAULT_DATE_FORMATS],
		quantityColumn: '',
		decimalSeparator: '.',
		thousandsSeparator: ''
	};
}

/** The wizard asks for a file, then for the mapping, then for confirmation. */
export function wizardStage(path: string | null, preview: ImportPreview | null): WizardStage {
	if (path === null) return 'pick';
	if (preview === null) return 'configure';
	return 'review';
}

export function blockingFindings(findings: PreviewFinding[]): PreviewFinding[] {
	return findings.filter((finding) => finding.severity === 'blocking');
}

export function warningFindings(findings: PreviewFinding[]): PreviewFinding[] {
	return findings.filter((finding) => finding.severity === 'warning');
}

/** Mapping is complete when every required binding has a value. */
export function mappingComplete(form: WizardForm): boolean {
	return (
		form.keyColumns.length > 0 &&
		form.dateColumn !== '' &&
		form.quantityColumn !== '' &&
		form.dateFormats.length > 0
	);
}

/** Import is possible only from a clean sample preview and a complete mapping. */
export function canImport(
	preview: ImportPreview | null,
	form: WizardForm,
	busy: boolean
): boolean {
	if (busy || preview === null) return false;
	if (!mappingComplete(form)) return false;
	return blockingFindings(preview.findings).length === 0;
}

export function composeKey(parts: string[], separator: string): string {
	return parts.join(separator);
}

/** Workbook previews carry sheet names; text exports never do. */
export function sourceFormat(preview: ImportPreview): 'csv' | 'xlsx' {
	return preview.sheets.length > 0 ? 'xlsx' : 'csv';
}

/** Build the wire descriptor; empty optional values become null. */
export function descriptorFromForm(
	form: WizardForm,
	format: 'csv' | 'xlsx',
	sheets: string[]
): ParsingDescriptor {
	return {
		descriptor_version: 1,
		format,
		encoding: format === 'xlsx' ? null : form.encoding,
		delimiter: format === 'xlsx' ? null : form.delimiter,
		header_row: form.headerRow,
		sheet: format === 'xlsx' ? form.sheet || sheets[0] || null : null,
		key_columns: form.keyColumns,
		key_separator: form.keySeparator || DEFAULT_KEY_SEPARATOR,
		date_column: form.dateColumn,
		quantity_column: form.quantityColumn,
		unit_column: null,
		date_formats: form.dateFormats,
		decimal_separator: form.decimalSeparator,
		thousands_separator:
			THOUSANDS_SEPARATORS.find((separator) => separator === form.thousandsSeparator) ?? null,
		aggregation: 'weekly_sum'
	};
}

/**
 * Overlay detection results onto the form. Apply this once per chosen file
 * (see `adoptDetectionOnce`), right after the first detection-only preview, so
 * later user edits are never overwritten.
 */
export function applyDetected(form: WizardForm, preview: ImportPreview): WizardForm {
	const detected = preview.detected;
	return {
		...form,
		encoding: detected.encoding ?? form.encoding,
		delimiter: detected.delimiter ?? form.delimiter,
		headerRow: detected.header_row ?? form.headerRow,
		sheet: detected.sheet ?? form.sheet
	};
}

export interface DetectionOutcome {
	form: WizardForm;
	applied: boolean;
}

/** Apply detection only when the wizard is still waiting for it. */
export function adoptDetectionOnce(
	form: WizardForm,
	preview: ImportPreview,
	adopt: boolean
): DetectionOutcome {
	if (!adopt) return { form, applied: false };
	return { form: applyDetected(form, preview), applied: true };
}

/**
 * Build a form from a saved configuration. The wizard still re-previews the
 * chosen file and shows the findings before anything can be imported, so a
 * saved configuration never skips confirmation.
 */
export function applyConfig(descriptor: ParsingDescriptor): WizardForm {
	const form = emptyForm();
	form.encoding = descriptor.encoding ?? form.encoding;
	form.delimiter = descriptor.delimiter ?? form.delimiter;
	form.headerRow = descriptor.header_row ?? form.headerRow;
	form.sheet = descriptor.sheet ?? '';
	form.keyColumns = descriptor.key_columns ?? [];
	form.keySeparator = descriptor.key_separator ?? DEFAULT_KEY_SEPARATOR;
	form.dateColumn = descriptor.date_column ?? '';
	form.quantityColumn = descriptor.quantity_column ?? '';
	form.dateFormats = descriptor.date_formats ?? [...DEFAULT_DATE_FORMATS];
	form.decimalSeparator = descriptor.decimal_separator ?? form.decimalSeparator;
	form.thousandsSeparator = descriptor.thousands_separator ?? '';
	return form;
}
