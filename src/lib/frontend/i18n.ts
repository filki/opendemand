/**
 * Minimal message catalog for the guided import. Only the wizard copy is
 * localized (en, pl, es); the rest of the interface is English, and Rust error
 * messages stay English with stable codes.
 */
export type Locale = 'en' | 'pl' | 'es';

export const LOCALES: Locale[] = ['en', 'pl', 'es'];

const en = {
	'wizard.title': 'Guided import',
	'wizard.intro':
		'Convert a raw planner export into the canonical weekly history. The wizard reads a preview only; nothing is imported until you press Import.',
	'wizard.chooseFile': 'Choose a file…',
	'wizard.changeFile': 'Choose another file…',
	'wizard.pickHint': 'CSV, TXT, and XLSX exports are supported.',
	'wizard.structure': 'File structure',
	'wizard.encoding': 'Encoding',
	'wizard.delimiter': 'Delimiter',
	'wizard.headerRow': 'Header row',
	'wizard.sheet': 'Sheet',
	'wizard.columns': 'Column mapping',
	'wizard.keyColumns': 'Key columns',
	'wizard.keyColumnsHint':
		'Checked columns build the series key in header order; the separator must not appear inside a value.',
	'wizard.keySeparator': 'Key separator',
	'wizard.dateColumn': 'Date column',
	'wizard.dateFormats': 'Accepted date formats',
	'wizard.quantityColumn': 'Quantity column',
	'wizard.numbers': 'Number format',
	'wizard.decimalSeparator': 'Decimal separator',
	'wizard.thousandsSeparator': 'Thousands separator',
	'wizard.none': 'none',
	'wizard.sample': 'Preview',
	'wizard.seriesKey': 'series_key',
	'wizard.weekStart': 'week_start',
	'wizard.quantity': 'quantity',
	'wizard.status': 'status',
	'wizard.findings': 'Validation',
	'wizard.blocking': 'Blocking',
	'wizard.warning': 'Warning',
	'wizard.noFindings': 'No problems found in the sample.',
	'wizard.mappingIncomplete': 'Select at least one key column, a date column, and a quantity column.',
	'wizard.import': 'Import',
	'wizard.cancel': 'Close',
	'wizard.refreshing': 'Reading the file…',
	'wizard.savedConfigs': 'Saved configurations',
	'wizard.configName': 'Configuration name',
	'wizard.saveConfig': 'Save configuration',
	'wizard.deleteConfig': 'Delete',
	'wizard.noConfigs': 'No saved configurations yet.'
};

export type MessageKey = keyof typeof en;

const pl: Record<MessageKey, string> = {
	'wizard.title': 'Import z przewodnikiem',
	'wizard.intro':
		'Konwersja surowego eksportu planisty do kanonicznej historii tygodniowej. Kreator tylko czyta podgląd; nic nie zostanie zaimportowane, dopóki nie naciśniesz Importuj.',
	'wizard.chooseFile': 'Wybierz plik…',
	'wizard.changeFile': 'Wybierz inny plik…',
	'wizard.pickHint': 'Obsługiwane są eksporty CSV, TXT i XLSX.',
	'wizard.structure': 'Struktura pliku',
	'wizard.encoding': 'Kodowanie',
	'wizard.delimiter': 'Separator kolumn',
	'wizard.headerRow': 'Wiersz nagłówka',
	'wizard.sheet': 'Arkusz',
	'wizard.columns': 'Mapowanie kolumn',
	'wizard.keyColumns': 'Kolumny klucza',
	'wizard.keyColumnsHint':
		'Zaznaczone kolumny tworzą klucz serii w kolejności nagłówka; separator nie może wystąpić wewnątrz wartości.',
	'wizard.keySeparator': 'Separator klucza',
	'wizard.dateColumn': 'Kolumna daty',
	'wizard.dateFormats': 'Akceptowane formaty dat',
	'wizard.quantityColumn': 'Kolumna ilości',
	'wizard.numbers': 'Format liczb',
	'wizard.decimalSeparator': 'Separator dziesiętny',
	'wizard.thousandsSeparator': 'Separator tysięcy',
	'wizard.none': 'brak',
	'wizard.sample': 'Podgląd',
	'wizard.seriesKey': 'series_key',
	'wizard.weekStart': 'week_start',
	'wizard.quantity': 'quantity',
	'wizard.status': 'status',
	'wizard.findings': 'Walidacja',
	'wizard.blocking': 'Blokujące',
	'wizard.warning': 'Ostrzeżenie',
	'wizard.noFindings': 'Brak problemów w próbce.',
	'wizard.mappingIncomplete': 'Wybierz co najmniej jedną kolumnę klucza, kolumnę daty i kolumnę ilości.',
	'wizard.import': 'Importuj',
	'wizard.cancel': 'Zamknij',
	'wizard.refreshing': 'Wczytywanie pliku…',
	'wizard.savedConfigs': 'Zapisane konfiguracje',
	'wizard.configName': 'Nazwa konfiguracji',
	'wizard.saveConfig': 'Zapisz konfigurację',
	'wizard.deleteConfig': 'Usuń',
	'wizard.noConfigs': 'Brak zapisanych konfiguracji.'
};

const es: Record<MessageKey, string> = {
	'wizard.title': 'Importación guiada',
	'wizard.intro':
		'Convierte una exportación sin procesar del planificador en el historial semanal canónico. El asistente solo lee una vista previa; nada se importa hasta que pulses Importar.',
	'wizard.chooseFile': 'Elegir un archivo…',
	'wizard.changeFile': 'Elegir otro archivo…',
	'wizard.pickHint': 'Se admiten exportaciones CSV, TXT y XLSX.',
	'wizard.structure': 'Estructura del archivo',
	'wizard.encoding': 'Codificación',
	'wizard.delimiter': 'Delimitador',
	'wizard.headerRow': 'Fila de encabezado',
	'wizard.sheet': 'Hoja',
	'wizard.columns': 'Asignación de columnas',
	'wizard.keyColumns': 'Columnas de clave',
	'wizard.keyColumnsHint':
		'Las columnas marcadas forman la clave de serie en el orden del encabezado; el separador no debe aparecer dentro de un valor.',
	'wizard.keySeparator': 'Separador de clave',
	'wizard.dateColumn': 'Columna de fecha',
	'wizard.dateFormats': 'Formatos de fecha aceptados',
	'wizard.quantityColumn': 'Columna de cantidad',
	'wizard.numbers': 'Formato numérico',
	'wizard.decimalSeparator': 'Separador decimal',
	'wizard.thousandsSeparator': 'Separador de miles',
	'wizard.none': 'ninguno',
	'wizard.sample': 'Vista previa',
	'wizard.seriesKey': 'series_key',
	'wizard.weekStart': 'week_start',
	'wizard.quantity': 'quantity',
	'wizard.status': 'status',
	'wizard.findings': 'Validación',
	'wizard.blocking': 'Bloqueante',
	'wizard.warning': 'Aviso',
	'wizard.noFindings': 'No se encontraron problemas en la muestra.',
	'wizard.mappingIncomplete':
		'Selecciona al menos una columna de clave, una columna de fecha y una columna de cantidad.',
	'wizard.import': 'Importar',
	'wizard.cancel': 'Cerrar',
	'wizard.refreshing': 'Leyendo el archivo…',
	'wizard.savedConfigs': 'Configuraciones guardadas',
	'wizard.configName': 'Nombre de la configuración',
	'wizard.saveConfig': 'Guardar configuración',
	'wizard.deleteConfig': 'Eliminar',
	'wizard.noConfigs': 'Todavía no hay configuraciones guardadas.'
};

export const messages: Record<Locale, Record<MessageKey, string>> = { en, pl, es };

/** Map an arbitrary locale value to a supported one; unknown values use en. */
export function resolveLocale(value: string | null | undefined): Locale {
	if (typeof value !== 'string') return 'en';
	const normalized = value.trim().toLowerCase();
	if (normalized.startsWith('pl')) return 'pl';
	if (normalized.startsWith('es')) return 'es';
	return 'en';
}

/** Look up a message, falling back to English and then to the key itself. */
export function t(locale: Locale, key: MessageKey): string {
	return messages[locale][key] ?? messages.en[key] ?? key;
}
