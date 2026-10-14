import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
	adoptDetectionOnce,
	applyDetected,
	blockingFindings,
	canImport,
	composeKey,
	descriptorFromForm,
	emptyForm,
	mappingComplete,
	sourceFormat,
	warningFindings,
	wizardStage
} from '../../src/lib/frontend/import-wizard.ts';
import type { ImportPreview, ParsingDescriptor, PreviewFinding } from '../../src/lib/bindings.ts';

function previewFixture(overrides: Partial<ImportPreview> = {}): ImportPreview {
	return {
		detected: {
			descriptor_version: 1,
			format: 'csv',
			encoding: 'windows-1250',
			delimiter: ';',
			header_row: 3,
			sheet: null,
			key_columns: null,
			key_separator: null,
			date_column: null,
			quantity_column: null,
			unit_column: null,
			date_formats: null,
			decimal_separator: null,
			thousands_separator: null,
			aggregation: null
		},
		headers: ['Lp.', 'Kod towaru', 'Data sprzedaży', 'Ilość'],
		sample_rows: [['1', '15039', '21.11.2011', '10']],
		parsed_rows: [{ series_key: '15039', week_start: '2011-11-21', quantity: 10, status: 'known' }],
		findings: [],
		can_parse: true,
		truncated: false,
		sheets: [],
		...overrides
	};
}

function completeForm() {
	const form = emptyForm();
	form.keyColumns = ['Kod towaru'];
	form.dateColumn = 'Data sprzedaży';
	form.quantityColumn = 'Ilość';
	return form;
}

const blocking: PreviewFinding = {
	severity: 'blocking',
	code: 'missing_quantity_column',
	detail: "Quantity column 'Ilosc' is not in the header row.",
	row_ref: null
};
const warning: PreviewFinding = {
	severity: 'warning',
	code: 'empty_key_row',
	detail: 'Row 12 has an empty key column; the row is excluded.',
	row_ref: '12'
};

test('wizard stages: pick before a file, configure without a preview, review afterwards', () => {
	assert.equal(wizardStage(null, null), 'pick');
	assert.equal(wizardStage('/tmp/source.csv', null), 'configure');
	assert.equal(wizardStage('/tmp/source.csv', previewFixture()), 'review');
});

test('descriptor preview: blocking issues disable commit', () => {
	assert.equal(canImport(previewFixture({ findings: [blocking] }), completeForm(), false), false);
	assert.equal(canImport(previewFixture({ findings: [warning] }), completeForm(), false), true);
	assert.equal(canImport(previewFixture(), completeForm(), false), true);
});

test('descriptor preview: commit stays disabled while busy or unmapped', () => {
	assert.equal(canImport(previewFixture(), completeForm(), true), false);
	assert.equal(canImport(previewFixture(), emptyForm(), false), false);
	assert.equal(canImport(null, completeForm(), false), false);
});

test('column mapping requires one key, one date, and one quantity column', () => {
	const form = emptyForm();
	assert.equal(mappingComplete(form), false);
	form.keyColumns = ['Kod'];
	assert.equal(mappingComplete(form), false);
	form.dateColumn = 'Data';
	assert.equal(mappingComplete(form), false);
	form.quantityColumn = 'Ilosc';
	assert.equal(mappingComplete(form), true);
	form.dateFormats = [];
	assert.equal(mappingComplete(form), false);
});

test('composed key joins parts in declared order with the separator', () => {
	assert.equal(composeKey(['15039', '1000'], '|'), '15039|1000');
	assert.equal(composeKey(['15039'], '|'), '15039');
	assert.equal(composeKey(['A', 'B', 'C'], '::'), 'A::B::C');
});

test('descriptor from form: text exports keep encoding and delimiter, workbooks keep the sheet', () => {
	const form = completeForm();
	form.thousandsSeparator = ' ';

	const text = descriptorFromForm(form, 'csv', []);
	assert.equal(text.format, 'csv');
	assert.equal(text.encoding, 'utf-8');
	assert.equal(text.delimiter, ',');
	assert.equal(text.sheet, null);
	assert.equal(text.thousands_separator, ' ');
	assert.equal(text.aggregation, 'weekly_sum');
	assert.equal(text.key_separator, '|');
	assert.deepEqual(text.key_columns, ['Kod towaru']);

	const workbook = descriptorFromForm(form, 'xlsx', ['Sprzedaz']);
	assert.equal(workbook.format, 'xlsx');
	assert.equal(workbook.encoding, null);
	assert.equal(workbook.delimiter, null);
	assert.equal(workbook.sheet, 'Sprzedaz');

	form.thousandsSeparator = '';
	assert.equal(descriptorFromForm(form, 'csv', []).thousands_separator, null);

	form.keySeparator = '';
	assert.equal(descriptorFromForm(form, 'csv', []).key_separator, '|');
});

test('descriptor from form: an empty sheet selection falls back to the first sheet', () => {
	const form = completeForm();
	const descriptor = descriptorFromForm(form, 'xlsx', ['Sprzedaz', 'Inne']);
	assert.equal(descriptor.sheet, 'Sprzedaz');
});

test('applyDetected adopts the detected file structure', () => {
	const adopted = applyDetected(emptyForm(), previewFixture());
	assert.equal(adopted.encoding, 'windows-1250');
	assert.equal(adopted.delimiter, ';');
	assert.equal(adopted.headerRow, 3);
});

test('detection is adopted once per file and never overwrites later edits', () => {
	const first = adoptDetectionOnce(emptyForm(), previewFixture(), true);
	assert.equal(first.applied, true);
	assert.equal(first.form.delimiter, ';');

	const edited = { ...first.form, delimiter: ',' };
	const second = adoptDetectionOnce(edited, previewFixture(), false);
	assert.equal(second.applied, false);
	assert.equal(second.form.delimiter, ',');
	assert.equal(second.form, edited);
});

test('sourceFormat: workbook previews carry sheet names, text previews do not', () => {
	assert.equal(sourceFormat(previewFixture()), 'csv');
	assert.equal(sourceFormat(previewFixture({ sheets: ['Sprzedaz'] })), 'xlsx');
});

test('findings split into blocking and warning lists', () => {
	const findings = [blocking, warning, { ...warning, row_ref: '13' }];
	assert.deepEqual(
		blockingFindings(findings).map((finding) => finding.code),
		['missing_quantity_column']
	);
	assert.equal(warningFindings(findings).length, 2);
});

test('descriptor type keeps optional units unset', () => {
	const descriptor: ParsingDescriptor = descriptorFromForm(completeForm(), 'csv', []);
	assert.equal(descriptor.unit_column, null);
	assert.equal(descriptor.descriptor_version, 1);
});
