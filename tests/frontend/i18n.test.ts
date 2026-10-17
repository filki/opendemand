import assert from 'node:assert/strict';
import { test } from 'node:test';
import { LOCALES, LOCALE_NAMES, messages, resolveLocale, t } from '../../src/lib/frontend/i18n.ts';

test('en, pl, es catalogs expose identical key sets', () => {
	const reference = Object.keys(messages.en).sort();
	for (const locale of LOCALES) {
		assert.deepEqual(Object.keys(messages[locale]).sort(), reference);
	}
});

test('every message is non-empty', () => {
	for (const locale of LOCALES) {
		for (const [key, value] of Object.entries(messages[locale])) {
			assert.ok(value.trim().length > 0, `${locale}.${key} is empty`);
		}
	}
});

test('locale resolution falls back to en for unknown values', () => {
	assert.equal(resolveLocale(undefined), 'en');
	assert.equal(resolveLocale(null), 'en');
	assert.equal(resolveLocale(''), 'en');
	assert.equal(resolveLocale('de-DE'), 'en');
	assert.equal(resolveLocale('en'), 'en');
	assert.equal(resolveLocale('pl'), 'pl');
	assert.equal(resolveLocale('pl-PL'), 'pl');
	assert.equal(resolveLocale('PL'), 'pl');
	assert.equal(resolveLocale('es-419'), 'es');
});

test('translation looks up the locale and falls back to the key for unknown messages', () => {
	assert.equal(t('en', 'wizard.import'), 'Import');
	assert.equal(t('pl', 'wizard.import'), 'Importuj');
	assert.equal(t('es', 'wizard.import'), 'Importar');
	assert.equal(t('pl', 'wizard.unknown' as never), 'wizard.unknown');
});

test('language names stay recognizable in their own language', () => {
	assert.deepEqual(Object.keys(LOCALE_NAMES).sort(), [...LOCALES].sort());
	assert.equal(LOCALE_NAMES.en, 'English');
	assert.equal(LOCALE_NAMES.pl, 'Polski');
	assert.equal(LOCALE_NAMES.es, 'Español');
});

test('interpolation replaces named placeholders per locale and keeps missing ones visible', () => {
	assert.equal(t('en', 'weekly.demandCount', { count: 3 }), 'Demand 3');
	assert.equal(t('pl', 'weekly.demandCount', { count: 3 }), 'Popyt 3');
	assert.equal(t('es', 'weekly.forecastCount', { count: 12 }), 'Pronóstico 12');
	assert.equal(
		t('en', 'overview.coverage', { covered: 2, total: 3, percent: '67%' }),
		'Coverage 2/3 (67%)'
	);
	assert.equal(
		t('en', 'weekly.pair', {
			previousRevision: 1,
			currentRevision: 2
		}),
		'Comparing the latest import (#2) with the previous import (#1).'
	);
	assert.equal(
		t('en', 'overview.range', { from: 1, count: 40 }),
		'Showing 1–{to} of 40',
		'only provided parameters are replaced'
	);
	assert.equal(
		t('pl', 'export.xlsxSaved', { rows: 36, path: 'C:\\plans\\run.xlsx' }),
		'Wyeksportowano 36 wierszy tygodniowych wszystkich serii do C:\\plans\\run.xlsx (skoroszyt Excel).'
	);
});

test('every weekly-review message is present in all three locales', () => {
	for (const key of [
		'workspace.language',
		'workspace.languageHint',
		'weekly.filterLabel',
		'weekly.filter.demand',
		'weekly.noFilterMatch',
		'detail.deltaNote',
		'overview.coverageNote',
		'overview.exceptionsHeading',
		'export.xlsx',
		'export.xlsxBusy'
	] as const) {
		for (const locale of LOCALES) {
			assert.ok(t(locale, key).trim().length > 0, `${locale}.${key} is empty`);
		}
	}
});
