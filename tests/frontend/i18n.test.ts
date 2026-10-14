import assert from 'node:assert/strict';
import { test } from 'node:test';
import { LOCALES, messages, resolveLocale, t } from '../../src/lib/frontend/i18n.ts';

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
