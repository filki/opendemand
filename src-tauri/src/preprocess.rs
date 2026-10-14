//! Raw-source preprocessing: detection and preview for non-canonical exports.
//!
//! Ticket 1 of the preprocessing plan: a read-only preview. It decodes a head
//! window of a raw CSV export, detects the encoding, delimiter, and header row,
//! and returns sampled parsed cells with blocking and warning findings. It never
//! writes, never opens the database, and never changes the frozen `canonical/1`
//! parser in `workspace.rs`.

use crate::{
    error::{AppError, AppResult},
    workspace::{cancelled_import, classify, ParsedImport, ParsedIssue},
};
use calamine::{open_workbook_auto_from_rs, Data, Range, Reader, Sheets};
use chrono::{Datelike, Duration as ChronoDuration, NaiveDate};
use encoding_rs::{Encoding, UTF_16BE, UTF_16LE, UTF_8, WINDOWS_1250};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    io::{Cursor, Read},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

/// Bytes read from the head of the file for detection and preview.
const HEAD_BYTES: usize = 64 * 1024;
/// Physical rows inspected while locating the header row.
const DETECTION_ROWS: usize = 20;
/// Raw rows returned in a preview.
const PREVIEW_ROWS: usize = 50;
const DEFAULT_KEY_SEPARATOR: &str = "|";
const DEFAULT_DATE_FORMATS: [&str; 4] = ["%Y-%m-%d", "%d.%m.%Y", "%d/%m/%Y", "%Y%m%d"];
const DELIMITER_CANDIDATES: [u8; 4] = *b";,\t|";
/// Per code cap on stored findings; overflow is reported by one summary row.
const MAX_ISSUES_PER_CODE: usize = 50;

/// A versioned description of how the bytes of one source file become
/// `canonical/1` rows. Fields are `Option` because detection fills them in
/// step by step on the way through the wizard.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct ParsingDescriptor {
    pub descriptor_version: u32,
    pub format: Option<String>,
    pub encoding: Option<String>,
    pub delimiter: Option<String>,
    pub header_row: Option<u32>,
    pub sheet: Option<String>,
    pub key_columns: Option<Vec<String>>,
    pub key_separator: Option<String>,
    pub date_column: Option<String>,
    pub quantity_column: Option<String>,
    pub unit_column: Option<String>,
    pub date_formats: Option<Vec<String>>,
    pub decimal_separator: Option<String>,
    pub thousands_separator: Option<String>,
    pub aggregation: Option<String>,
}

impl Default for ParsingDescriptor {
    fn default() -> Self {
        Self {
            descriptor_version: 1,
            format: None,
            encoding: None,
            delimiter: None,
            header_row: None,
            sheet: None,
            key_columns: None,
            key_separator: None,
            date_column: None,
            quantity_column: None,
            unit_column: None,
            date_formats: None,
            decimal_separator: None,
            thousands_separator: None,
            aggregation: None,
        }
    }
}

/// One sampled cell: a composed key, the Monday of its week, the parsed
/// quantity, and the observation status derived from that quantity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct PreviewCell {
    pub series_key: String,
    pub week_start: String,
    pub quantity: f64,
    pub status: String,
}

/// A validation finding. `severity` is `blocking` (commit must not proceed) or
/// `warning` (commit may proceed, the report lists it).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct PreviewFinding {
    pub severity: String,
    pub code: String,
    pub detail: String,
    pub row_ref: Option<String>,
}

/// The result of a read-only preview pass over the head of a source file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct ImportPreview {
    pub detected: ParsingDescriptor,
    pub headers: Vec<String>,
    pub sample_rows: Vec<Vec<String>>,
    pub parsed_rows: Vec<PreviewCell>,
    pub findings: Vec<PreviewFinding>,
    pub can_parse: bool,
    pub truncated: bool,
    /// Sheet names for workbooks; empty for text exports.
    pub sheets: Vec<String>,
}

/// Deterministic serialization of a descriptor. Field order follows the struct
/// declaration, so equal descriptors always produce equal JSON.
pub fn descriptor_json(descriptor: &ParsingDescriptor) -> serde_json::Result<String> {
    serde_json::to_string(descriptor)
}

/// The retained-file extension for a supported source path, or a blocking
/// `unsupported_format` error.
pub fn retained_extension(path: &Path) -> AppResult<String> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "csv" | "txt" | "xlsx" => Ok(extension),
        _ => Err(AppError::new(
            "unsupported_format",
            format!(
                "Unsupported file type '{extension}'. The preprocessing layer reads CSV, TXT, and XLSX exports."
            ),
        )),
    }
}

/// Parse a whole raw export into the canonical observation shape, dispatching
/// on the validated file extension.
pub(crate) fn parse_raw(
    bytes: &[u8],
    extension: &str,
    descriptor: &ParsingDescriptor,
    cancel: &AtomicBool,
) -> AppResult<ParsedImport> {
    if extension == "xlsx" {
        parse_workbook(bytes, descriptor, cancel)
    } else {
        parse_text(bytes, descriptor, cancel)
    }
}

/// Parse a text export: decode, locate the header, bind columns, sum document
/// rows into Monday weeks, and classify the net quantity. Blocking problems
/// return an error; row-level problems become capped issues and are excluded.
fn parse_text(
    bytes: &[u8],
    descriptor: &ParsingDescriptor,
    cancel: &AtomicBool,
) -> AppResult<ParsedImport> {
    let encoding = match descriptor.encoding.as_deref() {
        Some(label) => Encoding::for_label(label.as_bytes()).ok_or_else(|| {
            AppError::new("invalid_config", format!("Unknown encoding '{label}'."))
        })?,
        None => detect_encoding(bytes).0,
    };
    let text = decode_text(bytes, false, encoding).ok_or_else(|| {
        AppError::new(
            "invalid_encoding",
            "The file could not be decoded with the selected encoding.",
        )
    })?;
    let delimiter = match descriptor.delimiter.as_deref() {
        Some(value) => single_byte_delimiter(value)?,
        None => detect_delimiter(&text),
    };
    let lines: Vec<&str> = text.lines().collect();
    let (header_line, headers) = locate_header(&lines, delimiter, descriptor.header_row)
        .ok_or_else(|| {
            AppError::new(
                "header_row_not_found",
                "The header row could not be located in the file.",
            )
        })?;
    let bindings = row_bindings(descriptor, &headers)?;

    let mut issues: Vec<ParsedIssue> = Vec::new();
    let mut daily: BTreeMap<(String, NaiveDate), Vec<f64>> = BTreeMap::new();
    let mut row_count = 0usize;
    for (offset, line) in lines[header_line..].iter().enumerate() {
        if cancel.load(Ordering::SeqCst) {
            return Err(cancelled_import());
        }
        if line.trim().is_empty() {
            continue;
        }
        let number = header_line + 1 + offset;
        row_count += 1;
        let Some(fields) = parse_line(line, delimiter) else {
            issues.push(row_issue(
                "parse_error",
                format!("Row {number} could not be parsed as CSV."),
                number,
            ));
            continue;
        };
        let cells: Vec<CellValue> = fields.into_iter().map(CellValue::from_text).collect();
        aggregate_row(number, &cells, &bindings, &mut daily, &mut issues);
    }

    finish_import(row_count, daily, issues)
}

/// Parse a workbook sheet into the same canonical shape. Native date and number
/// cells keep their type, so they never go through separator or format guessing.
fn parse_workbook(
    bytes: &[u8],
    descriptor: &ParsingDescriptor,
    cancel: &AtomicBool,
) -> AppResult<ParsedImport> {
    let mut workbook = open_workbook(bytes)?;
    let sheet_names = workbook.sheet_names();
    let chosen = choose_sheet(&sheet_names, descriptor.sheet.as_deref())?;
    let range = workbook.worksheet_range(&chosen).map_err(|_| {
        AppError::new(
            "unreadable_xlsx",
            format!("Sheet '{chosen}' could not be read."),
        )
    })?;
    let rows = range_rows(&range);
    let (header_line, headers) =
        locate_header_rows(&rows, descriptor.header_row).ok_or_else(|| {
            AppError::new(
                "header_row_not_found",
                "The header row could not be located in the sheet.",
            )
        })?;
    let bindings = row_bindings(descriptor, &headers)?;

    let mut issues: Vec<ParsedIssue> = Vec::new();
    let mut daily: BTreeMap<(String, NaiveDate), Vec<f64>> = BTreeMap::new();
    let mut row_count = 0usize;
    for (number, cells) in &rows {
        if *number <= header_line {
            continue;
        }
        if cancel.load(Ordering::SeqCst) {
            return Err(cancelled_import());
        }
        row_count += 1;
        aggregate_row(*number, cells, &bindings, &mut daily, &mut issues);
    }

    finish_import(row_count, daily, issues)
}

/// Resolved column bindings for one import, or the blocking error the preview
/// reports with the same code.
struct RowBindings {
    key_indexes: Vec<usize>,
    date_index: usize,
    quantity_index: usize,
    key_separator: String,
    decimal: String,
    thousands: Option<String>,
    date_formats: Vec<String>,
}

fn row_bindings(descriptor: &ParsingDescriptor, headers: &[String]) -> AppResult<RowBindings> {
    let key_columns = descriptor
        .key_columns
        .clone()
        .filter(|columns| !columns.is_empty())
        .ok_or_else(|| {
            AppError::new(
                "invalid_config",
                "Bind at least one key column before importing.",
            )
        })?;
    let date_column = descriptor
        .date_column
        .clone()
        .ok_or_else(|| AppError::new("invalid_config", "Bind a date column before importing."))?;
    let quantity_column = descriptor.quantity_column.clone().ok_or_else(|| {
        AppError::new("invalid_config", "Bind a quantity column before importing.")
    })?;
    let key_separator = descriptor
        .key_separator
        .clone()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_KEY_SEPARATOR.to_string());
    let decimal = descriptor
        .decimal_separator
        .clone()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| ".".to_string());
    let thousands = descriptor
        .thousands_separator
        .clone()
        .filter(|value| !value.is_empty());
    if thousands.as_deref() == Some(decimal.as_str()) {
        return Err(AppError::new(
            "invalid_config",
            "The decimal and thousands separators must differ.",
        ));
    }
    let date_formats = descriptor
        .date_formats
        .clone()
        .filter(|formats| !formats.is_empty())
        .unwrap_or_else(|| {
            DEFAULT_DATE_FORMATS
                .iter()
                .map(|format| (*format).to_string())
                .collect()
        });
    let key_indexes: Vec<usize> = key_columns
        .iter()
        .map(|name| {
            column_index(headers, name).ok_or_else(|| {
                AppError::new(
                    "missing_key_column",
                    format!("Key column '{name}' is not in the header row."),
                )
            })
        })
        .collect::<AppResult<Vec<usize>>>()?;
    let date_index = column_index(headers, &date_column).ok_or_else(|| {
        AppError::new(
            "missing_date_column",
            format!("Date column '{date_column}' is not in the header row."),
        )
    })?;
    let quantity_index = column_index(headers, &quantity_column).ok_or_else(|| {
        AppError::new(
            "missing_quantity_column",
            format!("Quantity column '{quantity_column}' is not in the header row."),
        )
    })?;
    Ok(RowBindings {
        key_indexes,
        date_index,
        quantity_index,
        key_separator,
        decimal,
        thousands,
        date_formats,
    })
}

/// Validate one document row and record its quantity in the daily cell key.
fn aggregate_row(
    number: usize,
    fields: &[CellValue],
    bindings: &RowBindings,
    daily: &mut BTreeMap<(String, NaiveDate), Vec<f64>>,
    issues: &mut Vec<ParsedIssue>,
) {
    let parts: Vec<String> = bindings
        .key_indexes
        .iter()
        .map(|index| cell_at(fields, *index).text())
        .collect();
    if parts.iter().any(|part| part.is_empty()) {
        issues.push(row_issue(
            "empty_key_row",
            format!("Row {number} has an empty key column; the row was excluded."),
            number,
        ));
        return;
    }
    if parts
        .iter()
        .any(|part| part.contains(&bindings.key_separator))
    {
        issues.push(row_issue(
            "key_separator_in_part",
            format!(
                "Row {number} has a key part containing the separator '{}'; the row was excluded.",
                bindings.key_separator
            ),
            number,
        ));
        return;
    }
    let Some(date) = cell_at(fields, bindings.date_index).value_date(&bindings.date_formats) else {
        issues.push(row_issue(
            "invalid_date_row",
            format!("Row {number} has a date that matches no selected format."),
            number,
        ));
        return;
    };
    let Some(quantity) = cell_at(fields, bindings.quantity_index)
        .value_quantity(&bindings.decimal, bindings.thousands.as_deref())
    else {
        issues.push(row_issue(
            "invalid_number_row",
            format!("Row {number} has a non-numeric quantity."),
            number,
        ));
        return;
    };
    daily
        .entry((parts.join(&bindings.key_separator), date))
        .or_default()
        .push(quantity);
}

/// Fold daily cells into Monday weeks, classify the net, and cap the issues.
fn finish_import(
    row_count: usize,
    daily: BTreeMap<(String, NaiveDate), Vec<f64>>,
    mut issues: Vec<ParsedIssue>,
) -> AppResult<ParsedImport> {
    // Sum each calendar day first so identical duplicate rows can be reported
    // at their true granularity, then fold the days into Monday weeks.
    let mut weekly: BTreeMap<(String, NaiveDate), f64> = BTreeMap::new();
    let mut duplicates: BTreeMap<(String, NaiveDate), usize> = BTreeMap::new();
    for ((key, day), values) in daily {
        let week = monday_of(day);
        let distinct: HashSet<u64> = values.iter().map(|value| value.to_bits()).collect();
        if distinct.len() < values.len() {
            *duplicates.entry((key.clone(), week)).or_default() += values.len() - distinct.len();
        }
        *weekly.entry((key, week)).or_default() += values.iter().sum::<f64>();
    }

    let mut series: BTreeMap<String, BTreeMap<NaiveDate, (f64, &'static str)>> = BTreeMap::new();
    let mut series_keys: BTreeSet<String> = BTreeSet::new();
    let mut cutoff: Option<NaiveDate> = None;
    for ((key, week), total) in weekly {
        if !total.is_finite() {
            issues.push(cell_issue(
                "warning",
                "invalid_number_row",
                format!("Rows summing to a non-finite quantity for {key} at {week} were excluded."),
            ));
            continue;
        }
        cutoff = Some(cutoff.map_or(week, |current| current.max(week)));
        series_keys.insert(key.clone());
        series
            .entry(key)
            .or_default()
            .insert(week, (total, classify(total)));
    }
    for ((key, week), count) in duplicates {
        issues.push(cell_issue(
            "info",
            "duplicate_rows_summed",
            format!("{count} identical duplicate rows were summed for {key} at {week}."),
        ));
    }
    let cutoff = cutoff.ok_or_else(|| {
        AppError::new(
            "empty_import",
            "No usable rows remained after validation; nothing was imported.",
        )
    })?;
    Ok(ParsedImport {
        row_count,
        cutoff,
        series_keys,
        series,
        issues: cap_issues(issues),
    })
}

/// One normalized cell. Text exports are all `Text`; workbook cells keep their
/// native type so typed values are never re-parsed from formatted text.
#[derive(Debug, Clone)]
enum CellValue {
    Empty,
    Text(String),
    Number(f64),
    Date(NaiveDate),
}

impl CellValue {
    fn from_text(text: String) -> Self {
        if text.is_empty() {
            CellValue::Empty
        } else {
            CellValue::Text(text)
        }
    }

    fn text(&self) -> String {
        match self {
            CellValue::Empty => String::new(),
            CellValue::Text(text) => text.clone(),
            CellValue::Number(value) => number_text(*value),
            CellValue::Date(date) => date.format("%Y-%m-%d").to_string(),
        }
    }

    fn value_date(&self, formats: &[String]) -> Option<NaiveDate> {
        match self {
            CellValue::Date(date) => Some(*date),
            other => parse_date(&other.text(), formats),
        }
    }

    fn value_quantity(&self, decimal: &str, thousands: Option<&str>) -> Option<f64> {
        match self {
            CellValue::Number(value) => value.is_finite().then_some(*value),
            other => parse_quantity(&other.text(), decimal, thousands),
        }
    }
}

fn cell_at(fields: &[CellValue], index: usize) -> &CellValue {
    fields.get(index).unwrap_or(&CellValue::Empty)
}

fn number_text(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

/// Open a workbook from the already-read source bytes.
fn open_workbook(bytes: &[u8]) -> AppResult<Sheets<Cursor<&[u8]>>> {
    open_workbook_auto_from_rs(Cursor::new(bytes))
        .map_err(|_| AppError::new("unreadable_xlsx", "The workbook could not be read."))
}

fn choose_sheet(sheet_names: &[String], requested: Option<&str>) -> AppResult<String> {
    match requested {
        Some(name) => {
            if sheet_names.iter().any(|sheet| sheet == name) {
                Ok(name.to_string())
            } else {
                Err(AppError::new(
                    "sheet_not_found",
                    format!("The workbook has no sheet named '{name}'."),
                ))
            }
        }
        None => sheet_names
            .first()
            .cloned()
            .ok_or_else(|| AppError::new("unreadable_xlsx", "The workbook contains no sheets.")),
    }
}

/// Normalize every non-empty sheet row, numbered by its physical sheet row.
fn range_rows(range: &Range<Data>) -> Vec<(usize, Vec<CellValue>)> {
    let start_row = range.start().map(|(row, _)| row as usize).unwrap_or(0);
    let mut rows = Vec::new();
    for (index, row) in range.rows().enumerate() {
        let cells: Vec<CellValue> = row.iter().map(cell_value).collect();
        if cells.iter().all(|cell| matches!(cell, CellValue::Empty)) {
            continue;
        }
        rows.push((start_row + index + 1, cells));
    }
    rows
}

fn cell_value(value: &Data) -> CellValue {
    match value {
        Data::Empty => CellValue::Empty,
        Data::String(text) => CellValue::from_text(text.trim().to_string()),
        Data::Float(number) => CellValue::Number(*number),
        Data::Int(number) => CellValue::Number(*number as f64),
        Data::Bool(flag) => CellValue::from_text(flag.to_string()),
        Data::DateTime(excel) => excel
            .as_datetime()
            .map(|datetime| CellValue::Date(datetime.date()))
            .unwrap_or_else(|| CellValue::from_text(excel.as_f64().to_string())),
        other => CellValue::from_text(other.to_string().trim().to_string()),
    }
}

fn cell_texts(cells: &[CellValue]) -> Vec<String> {
    cells.iter().map(CellValue::text).collect()
}

fn locate_header_rows(
    rows: &[(usize, Vec<CellValue>)],
    requested: Option<u32>,
) -> Option<(usize, Vec<String>)> {
    match requested {
        Some(line) => rows
            .iter()
            .find(|(number, _)| *number as u32 == line)
            .map(|(number, cells)| (*number, cell_texts(cells))),
        None => rows
            .iter()
            .take(DETECTION_ROWS)
            .map(|(number, cells)| (*number, cell_texts(cells)))
            .find(|(_, headers)| headers.iter().filter(|header| !header.is_empty()).count() >= 2),
    }
}

fn single_byte_delimiter(value: &str) -> AppResult<u8> {
    let mut bytes = value.bytes();
    match (bytes.next(), bytes.next()) {
        (Some(byte), None) => Ok(byte),
        _ => Err(AppError::new(
            "invalid_config",
            format!("The delimiter must be exactly one character; got '{value}'."),
        )),
    }
}

fn row_issue(code: &'static str, detail: String, line: usize) -> ParsedIssue {
    ParsedIssue {
        severity: "warning",
        code,
        detail,
        row_ref: Some(line.to_string()),
    }
}

fn cell_issue(severity: &'static str, code: &'static str, detail: String) -> ParsedIssue {
    ParsedIssue {
        severity,
        code,
        detail,
        row_ref: None,
    }
}

/// Keep at most `MAX_ISSUES_PER_CODE` findings per code and append one summary
/// per truncated code, so a damaged file cannot create unbounded issue rows.
fn cap_issues(issues: Vec<ParsedIssue>) -> Vec<ParsedIssue> {
    let mut counts: BTreeMap<&'static str, (usize, usize)> = BTreeMap::new();
    let mut kept: Vec<ParsedIssue> = Vec::new();
    for issue in issues {
        let entry = counts.entry(issue.code).or_default();
        entry.1 += 1;
        if entry.0 < MAX_ISSUES_PER_CODE {
            entry.0 += 1;
            kept.push(issue);
        }
    }
    let summaries = counts
        .into_iter()
        .filter(|(_, (kept_count, total))| total > kept_count)
        .map(|(code, (kept_count, total))| ParsedIssue {
            severity: "warning",
            code,
            detail: format!(
                "{} additional findings with code '{code}' were omitted; {total} in total.",
                total - kept_count
            ),
            row_ref: None,
        });
    kept.extend(summaries);
    kept
}

/// Read a head window of `path` and return a preview. `descriptor` may be a
/// partial descriptor from an earlier wizard step; `None` runs detection only.
/// Read a source file and return a preview. `descriptor` may be a partial
/// descriptor from an earlier wizard step; `None` runs detection only.
pub fn preview(path: &Path, descriptor: Option<&ParsingDescriptor>) -> AppResult<ImportPreview> {
    if file_extension(path) == "xlsx" {
        preview_workbook(path, descriptor)
    } else {
        preview_text(path, descriptor)
    }
}

fn file_extension(path: &Path) -> String {
    path.extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_default()
}

fn preview_text(path: &Path, descriptor: Option<&ParsingDescriptor>) -> AppResult<ImportPreview> {
    let mut findings: Vec<PreviewFinding> = Vec::new();
    let mut detected = descriptor.cloned().unwrap_or_default();
    let extension = file_extension(path);
    detected.format = Some(if extension.is_empty() {
        "unknown".to_string()
    } else {
        extension.clone()
    });
    if !matches!(extension.as_str(), "csv" | "txt") {
        findings.push(blocking(
            "unsupported_format",
            format!(
                "Unsupported file type '{extension}'. This preview reads CSV, TXT, and XLSX exports."
            ),
            None,
        ));
        return Ok(empty_preview(detected, findings, false, Vec::new()));
    }

    let (head, file_truncated) = read_head(path)?;
    if head.is_empty() {
        findings.push(blocking(
            "empty_file",
            "The file is empty; expected a header row and data rows.".to_string(),
            None,
        ));
        return Ok(empty_preview(detected, findings, false, Vec::new()));
    }

    let (bytes, truncated) = head_slice(&head, file_truncated);
    let encoding = match descriptor.and_then(|value| value.encoding.as_deref()) {
        Some(label) => match Encoding::for_label(label.as_bytes()) {
            Some(encoding) => encoding,
            None => {
                findings.push(blocking(
                    "invalid_config",
                    format!("Unknown encoding '{label}'."),
                    None,
                ));
                return Ok(empty_preview(detected, findings, truncated, Vec::new()));
            }
        },
        None => {
            let (encoding, assumed) = detect_encoding(&bytes);
            if assumed {
                findings.push(warning(
                    "encoding_assumed",
                    "The file is not valid UTF-8; Windows-1250 was assumed. Confirm the encoding."
                        .to_string(),
                    None,
                ));
            }
            encoding
        }
    };
    detected.encoding = Some(encoding.name().to_ascii_lowercase());

    let text = match decode_text(&bytes, truncated, encoding) {
        Some(text) => text,
        None => {
            findings.push(blocking(
                "invalid_encoding",
                "The file could not be decoded with the selected encoding.".to_string(),
                None,
            ));
            return Ok(empty_preview(detected, findings, truncated, Vec::new()));
        }
    };

    let delimiter = detect_delimiter(&text);
    detected.delimiter = Some((delimiter as char).to_string());

    let lines: Vec<&str> = text.lines().collect();
    let requested_header = descriptor.and_then(|value| value.header_row);
    let Some((header_line, headers)) = locate_header(&lines, delimiter, requested_header) else {
        findings.push(blocking(
            "header_row_not_found",
            match requested_header {
                Some(line) => format!("Row {line} is not present in the file head."),
                None => "No header row with at least two named columns was found.".to_string(),
            },
            None,
        ));
        return Ok(empty_preview(detected, findings, truncated, Vec::new()));
    };
    detected.header_row = Some(header_line as u32);

    let mut sample_rows: Vec<Vec<String>> = Vec::new();
    let mut sample_records: Vec<(usize, Vec<CellValue>)> = Vec::new();
    for (offset, line) in lines[header_line..].iter().enumerate() {
        if sample_rows.len() >= PREVIEW_ROWS {
            break;
        }
        if line.trim().is_empty() {
            continue;
        }
        let number = header_line + 1 + offset;
        match parse_line(line, delimiter) {
            Some(fields) => {
                let cells: Vec<CellValue> = fields.into_iter().map(CellValue::from_text).collect();
                sample_rows.push(cells.iter().map(CellValue::text).collect());
                sample_records.push((number, cells));
            }
            None => findings.push(warning(
                "parse_error",
                format!("Row {number} could not be parsed as CSV."),
                Some(number.to_string()),
            )),
        }
    }

    let parsed_rows = parse_sample(descriptor, &headers, &sample_records, &mut findings);

    let can_parse = !findings
        .iter()
        .any(|finding| finding.severity == "blocking")
        && !parsed_rows.is_empty();
    Ok(ImportPreview {
        detected,
        headers,
        sample_rows,
        parsed_rows,
        findings,
        can_parse,
        truncated,
        sheets: Vec::new(),
    })
}

/// Preview a workbook sheet: list sheets, locate the header row, and sample
/// typed cells. Native dates and numbers keep their type.
fn preview_workbook(
    path: &Path,
    descriptor: Option<&ParsingDescriptor>,
) -> AppResult<ImportPreview> {
    let mut findings: Vec<PreviewFinding> = Vec::new();
    let mut detected = descriptor.cloned().unwrap_or_default();
    detected.format = Some("xlsx".to_string());
    let bytes = std::fs::read(path)?;
    let mut workbook = match open_workbook(&bytes) {
        Ok(workbook) => workbook,
        Err(error) => {
            findings.push(blocking(error.code.as_str(), error.message, None));
            return Ok(empty_preview(detected, findings, false, Vec::new()));
        }
    };
    let sheet_names = workbook.sheet_names();
    let chosen = match choose_sheet(
        &sheet_names,
        descriptor.and_then(|value| value.sheet.as_deref()),
    ) {
        Ok(chosen) => chosen,
        Err(error) => {
            findings.push(blocking(error.code.as_str(), error.message, None));
            return Ok(empty_preview(detected, findings, false, sheet_names));
        }
    };
    detected.sheet = Some(chosen.clone());
    let range = match workbook.worksheet_range(&chosen) {
        Ok(range) => range,
        Err(_) => {
            findings.push(blocking(
                "unreadable_xlsx",
                format!("Sheet '{chosen}' could not be read."),
                None,
            ));
            return Ok(empty_preview(detected, findings, false, sheet_names));
        }
    };
    let rows = range_rows(&range);
    let requested_header = descriptor.and_then(|value| value.header_row);
    let Some((header_line, headers)) = locate_header_rows(&rows, requested_header) else {
        findings.push(blocking(
            "header_row_not_found",
            match requested_header {
                Some(line) => format!("Row {line} is not present in the sheet."),
                None => "No header row with at least two named columns was found.".to_string(),
            },
            None,
        ));
        return Ok(empty_preview(detected, findings, false, sheet_names));
    };
    detected.header_row = Some(header_line as u32);

    let mut sample_rows: Vec<Vec<String>> = Vec::new();
    let mut sample_records: Vec<(usize, Vec<CellValue>)> = Vec::new();
    for (number, cells) in &rows {
        if *number <= header_line {
            continue;
        }
        if sample_rows.len() >= PREVIEW_ROWS {
            break;
        }
        sample_rows.push(cell_texts(cells));
        sample_records.push((*number, cells.clone()));
    }

    let parsed_rows = parse_sample(descriptor, &headers, &sample_records, &mut findings);

    let can_parse = !findings
        .iter()
        .any(|finding| finding.severity == "blocking")
        && !parsed_rows.is_empty();
    Ok(ImportPreview {
        detected,
        headers,
        sample_rows,
        parsed_rows,
        findings,
        can_parse,
        truncated: false,
        sheets: sheet_names,
    })
}

fn empty_preview(
    detected: ParsingDescriptor,
    findings: Vec<PreviewFinding>,
    truncated: bool,
    sheets: Vec<String>,
) -> ImportPreview {
    ImportPreview {
        detected,
        headers: Vec::new(),
        sample_rows: Vec::new(),
        parsed_rows: Vec::new(),
        findings,
        can_parse: false,
        truncated,
        sheets,
    }
}

/// Compose the bound cells of every sampled record into key, week, quantity,
/// and status, recording warnings for rows that cannot be interpreted.
fn parse_sample(
    descriptor: Option<&ParsingDescriptor>,
    headers: &[String],
    rows: &[(usize, Vec<CellValue>)],
    findings: &mut Vec<PreviewFinding>,
) -> Vec<PreviewCell> {
    let Some(descriptor) = descriptor else {
        return Vec::new();
    };
    let key_columns = descriptor.key_columns.clone().unwrap_or_default();
    let date_column = descriptor.date_column.clone();
    let quantity_column = descriptor.quantity_column.clone();
    let key_separator = descriptor
        .key_separator
        .clone()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_KEY_SEPARATOR.to_string());
    let decimal = descriptor
        .decimal_separator
        .clone()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| ".".to_string());
    let thousands = descriptor
        .thousands_separator
        .clone()
        .filter(|value| !value.is_empty());
    let date_formats = descriptor
        .date_formats
        .clone()
        .filter(|formats| !formats.is_empty())
        .unwrap_or_else(|| {
            DEFAULT_DATE_FORMATS
                .iter()
                .map(|format| (*format).to_string())
                .collect()
        });

    if thousands.as_deref() == Some(decimal.as_str()) {
        findings.push(blocking(
            "invalid_config",
            "The decimal and thousands separators must differ.".to_string(),
            None,
        ));
        return Vec::new();
    }
    if key_columns.is_empty() {
        return Vec::new();
    }
    let mut bound = true;
    for name in &key_columns {
        if column_index(headers, name).is_none() {
            findings.push(blocking(
                "missing_key_column",
                format!("Key column '{name}' is not in the header row."),
                None,
            ));
            bound = false;
        }
    }
    let date_index = match &date_column {
        Some(name) => match column_index(headers, name) {
            Some(index) => Some(index),
            None => {
                findings.push(blocking(
                    "missing_date_column",
                    format!("Date column '{name}' is not in the header row."),
                    None,
                ));
                bound = false;
                None
            }
        },
        None => None,
    };
    let quantity_index = match &quantity_column {
        Some(name) => match column_index(headers, name) {
            Some(index) => Some(index),
            None => {
                findings.push(blocking(
                    "missing_quantity_column",
                    format!("Quantity column '{name}' is not in the header row."),
                    None,
                ));
                bound = false;
                None
            }
        },
        None => None,
    };
    let (Some(date_index), Some(quantity_index)) = (date_index, quantity_index) else {
        return Vec::new();
    };
    if !bound {
        return Vec::new();
    }

    let key_indexes: Vec<usize> = key_columns
        .iter()
        .filter_map(|name| column_index(headers, name))
        .collect();
    let mut parsed = Vec::new();
    for (line, fields) in rows {
        if parsed.len() >= PREVIEW_ROWS {
            break;
        }
        let parts: Vec<String> = key_indexes
            .iter()
            .map(|index| cell_at(fields, *index).text())
            .collect();
        if parts.iter().any(|part| part.is_empty()) {
            findings.push(warning(
                "empty_key_row",
                format!("Row {line} has an empty key column; the row is skipped."),
                Some(line.to_string()),
            ));
            continue;
        }
        let Some(date) = cell_at(fields, date_index).value_date(&date_formats) else {
            findings.push(warning(
                "invalid_date_row",
                format!("Row {line} has a date that matches no selected format."),
                Some(line.to_string()),
            ));
            continue;
        };
        let Some(quantity) =
            cell_at(fields, quantity_index).value_quantity(&decimal, thousands.as_deref())
        else {
            findings.push(warning(
                "invalid_number_row",
                format!("Row {line} has a non-numeric quantity."),
                Some(line.to_string()),
            ));
            continue;
        };
        parsed.push(PreviewCell {
            series_key: parts.join(&key_separator),
            week_start: monday_of(date).format("%Y-%m-%d").to_string(),
            quantity,
            status: classify(quantity).to_string(),
        });
    }
    parsed
}

fn parse_date(raw: &str, formats: &[String]) -> Option<NaiveDate> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    formats
        .iter()
        .find_map(|format| NaiveDate::parse_from_str(raw, format).ok())
}

fn parse_quantity(raw: &str, decimal: &str, thousands: Option<&str>) -> Option<f64> {
    let mut value = raw.trim().to_string();
    if value.is_empty() {
        return None;
    }
    if let Some(separator) = thousands {
        value = value.replace(separator, "");
    }
    if decimal != "." {
        value = value.replace(decimal, ".");
    }
    let parsed: f64 = value.parse().ok()?;
    parsed.is_finite().then_some(parsed)
}

fn monday_of(date: NaiveDate) -> NaiveDate {
    date - ChronoDuration::days(date.weekday().num_days_from_monday() as i64)
}

fn column_index(headers: &[String], name: &str) -> Option<usize> {
    headers.iter().position(|header| header == name)
}

/// Read the head window and report whether the file continues past it.
fn read_head(path: &Path) -> AppResult<(Vec<u8>, bool)> {
    let file = std::fs::File::open(path)?;
    let length = file.metadata()?.len();
    let mut bytes = Vec::new();
    file.take(HEAD_BYTES as u64).read_to_end(&mut bytes)?;
    let truncated = length > bytes.len() as u64;
    Ok((bytes, truncated))
}

/// Cut a truncated head at the last line boundary so decoding never fails on a
/// character split by the window edge.
fn head_slice(head: &[u8], file_truncated: bool) -> (Vec<u8>, bool) {
    if !file_truncated {
        return (head.to_vec(), false);
    }
    match head.iter().rposition(|byte| *byte == b'\n') {
        Some(index) => (head[..=index].to_vec(), true),
        None => (head.to_vec(), true),
    }
}

/// Guess the encoding from a BOM, then UTF-8 validity; fall back to
/// Windows-1250 and report that assumption.
fn detect_encoding(bytes: &[u8]) -> (&'static Encoding, bool) {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        (UTF_8, false)
    } else if bytes.starts_with(&[0xFF, 0xFE]) {
        (UTF_16LE, false)
    } else if bytes.starts_with(&[0xFE, 0xFF]) {
        (UTF_16BE, false)
    } else if std::str::from_utf8(bytes).is_ok() {
        (UTF_8, false)
    } else {
        (WINDOWS_1250, true)
    }
}

fn decode_text(bytes: &[u8], truncated: bool, encoding: &'static Encoding) -> Option<String> {
    let text = match encoding.decode_without_bom_handling_and_without_replacement(bytes) {
        Some(text) => text.into_owned(),
        None if truncated => encoding.decode_without_bom_handling(bytes).0.into_owned(),
        None => return None,
    };
    Some(
        text.strip_prefix('\u{feff}')
            .map(str::to_owned)
            .unwrap_or(text),
    )
}

/// Pick the delimiter whose field count is most consistent across the head.
fn detect_delimiter(text: &str) -> u8 {
    let mut best = b',';
    let mut best_score = 0;
    for candidate in DELIMITER_CANDIDATES {
        let score = delimiter_score(text, candidate);
        if score > best_score {
            best = candidate;
            best_score = score;
        }
    }
    best
}

fn delimiter_score(text: &str, delimiter: u8) -> usize {
    let mut frequencies: BTreeMap<usize, usize> = BTreeMap::new();
    for line in text.lines().take(DETECTION_ROWS) {
        if let Some(fields) = parse_line(line, delimiter) {
            *frequencies.entry(fields.len()).or_default() += 1;
        }
    }
    frequencies
        .into_iter()
        .filter(|(width, _)| *width >= 2)
        .map(|(_, count)| count)
        .max()
        .unwrap_or(0)
}

/// Parse one physical row, returning its trimmed fields, or `None` when the row
/// is not valid CSV (an unterminated quote, for example).
fn parse_line(line: &str, delimiter: u8) -> Option<Vec<String>> {
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(false)
        .flexible(true)
        .from_reader(line.as_bytes());
    let record = reader.records().next()?.ok()?;
    Some(
        record
            .iter()
            .map(|field| field.trim().to_string())
            .collect(),
    )
}

/// Find the header row: an explicit physical line when the descriptor provides
/// one, otherwise the first physical line with at least two non-empty fields.
fn locate_header(
    lines: &[&str],
    delimiter: u8,
    requested: Option<u32>,
) -> Option<(usize, Vec<String>)> {
    match requested {
        Some(line) => {
            let index = (line as usize).checked_sub(1)?;
            let fields = parse_line(lines.get(index)?, delimiter)?;
            Some((line as usize, fields))
        }
        None => lines
            .iter()
            .take(DETECTION_ROWS)
            .enumerate()
            .find_map(|(index, text)| {
                let fields = parse_line(text, delimiter)?;
                let named = fields.iter().filter(|field| !field.is_empty()).count();
                (named >= 2).then_some((index + 1, fields))
            }),
    }
}

fn blocking(code: &str, detail: String, row_ref: Option<String>) -> PreviewFinding {
    PreviewFinding {
        severity: "blocking".to_string(),
        code: code.to_string(),
        detail,
        row_ref,
    }
}

fn warning(code: &str, detail: String, row_ref: Option<String>) -> PreviewFinding {
    PreviewFinding {
        severity: "warning".to_string(),
        code: code.to_string(),
        detail,
        row_ref,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn example(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../docs/architecture/contracts/examples/import")
            .join(name)
    }

    fn write_temp(directory: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let path = directory.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn subiekt_descriptor() -> ParsingDescriptor {
        ParsingDescriptor {
            format: Some("csv".into()),
            key_columns: Some(vec!["Kod towaru".into()]),
            date_column: Some("Data sprzedaży".into()),
            date_formats: Some(vec!["%d.%m.%Y".into()]),
            quantity_column: Some("Ilość".into()),
            ..ParsingDescriptor::default()
        }
    }

    #[test]
    fn detection_prefers_semicolon_and_1250_for_subiekt_fixture() {
        let preview = preview(&example("subiekt-sprzedaz.csv"), None).unwrap();
        assert_eq!(preview.detected.encoding.as_deref(), Some("windows-1250"));
        assert_eq!(preview.detected.delimiter.as_deref(), Some(";"));
        assert_eq!(preview.detected.header_row, Some(1));
        assert_eq!(preview.sample_rows.len(), PREVIEW_ROWS);
        assert!(!preview.truncated);
        assert!(preview.findings.iter().any(|finding| {
            finding.severity == "warning" && finding.code == "encoding_assumed"
        }));
    }

    #[test]
    fn detection_decodes_polish_headers_from_windows_1250() {
        let preview = preview(&example("subiekt-sprzedaz.csv"), None).unwrap();
        assert_eq!(
            preview.headers,
            vec![
                "Lp.",
                "Kod towaru",
                "Nazwa",
                "Data sprzedaży",
                "Ilość",
                "Wartość netto"
            ]
        );
    }

    #[test]
    fn detection_finds_header_row_after_title_and_blank_lines() {
        let directory = tempfile::tempdir().unwrap();
        let path = write_temp(
            directory.path(),
            "report.csv",
            "Raport sprzedaży\n\nKod;Data;Ilosc\n15039;21.11.2011;10\n".as_bytes(),
        );
        let preview = preview(&path, None).unwrap();
        assert_eq!(preview.detected.header_row, Some(3));
        assert_eq!(preview.headers, vec!["Kod", "Data", "Ilosc"]);
    }

    #[test]
    fn preview_returns_first_rows_only() {
        let directory = tempfile::tempdir().unwrap();
        let mut csv = String::from("Lp.;Kod towaru;Data sprzedaży;Ilość\n");
        for index in 0..120 {
            csv.push_str(&format!("{index};15039;21.11.2011;{}\n", index + 1));
        }
        let path = write_temp(directory.path(), "long.csv", csv.as_bytes());
        let descriptor = subiekt_descriptor();
        let preview = preview(&path, Some(&descriptor)).unwrap();
        assert_eq!(preview.sample_rows.len(), PREVIEW_ROWS);
        assert_eq!(preview.parsed_rows.len(), PREVIEW_ROWS);
    }

    #[test]
    fn preview_without_descriptor_returns_detection_only() {
        let preview = preview(&example("subiekt-sprzedaz.csv"), None).unwrap();
        assert!(preview.parsed_rows.is_empty());
        assert!(!preview.can_parse);
    }

    #[test]
    fn preview_composes_key_and_monday_week_on_sample() {
        let descriptor = subiekt_descriptor();
        let preview = preview(&example("subiekt-sprzedaz.csv"), Some(&descriptor)).unwrap();
        assert!(preview.can_parse);
        let first = &preview.parsed_rows[0];
        assert_eq!(first.series_key, "15039");
        assert_eq!(first.week_start, "2011-11-21");
        assert_eq!(first.quantity, 10.0);
        assert_eq!(first.status, "known");
    }

    #[test]
    fn preview_missing_bound_column_is_blocking() {
        let descriptor = ParsingDescriptor {
            key_columns: Some(vec!["Nie ma".into()]),
            date_column: Some("Data sprzedaży".into()),
            quantity_column: Some("Ilość".into()),
            ..ParsingDescriptor::default()
        };
        let preview = preview(&example("subiekt-sprzedaz.csv"), Some(&descriptor)).unwrap();
        assert!(!preview.can_parse);
        assert!(preview.parsed_rows.is_empty());
        assert!(preview.findings.iter().any(|finding| {
            finding.severity == "blocking" && finding.code == "missing_key_column"
        }));
    }

    #[test]
    fn preview_undecodable_bytes_is_blocking() {
        let directory = tempfile::tempdir().unwrap();
        let path = write_temp(
            directory.path(),
            "broken.csv",
            &[0xFF, 0xFE, 0x41, 0x00, 0x42],
        );
        let preview = preview(&path, None).unwrap();
        assert!(!preview.can_parse);
        assert!(preview.findings.iter().any(|finding| {
            finding.severity == "blocking" && finding.code == "invalid_encoding"
        }));
    }

    #[test]
    fn descriptor_serialization_is_deterministic() {
        let descriptor = ParsingDescriptor {
            format: Some("csv".into()),
            encoding: Some("windows-1250".into()),
            delimiter: Some(";".into()),
            header_row: Some(1),
            key_columns: Some(vec!["Kod towaru".into()]),
            key_separator: Some("|".into()),
            date_column: Some("Data sprzedaży".into()),
            quantity_column: Some("Ilość".into()),
            date_formats: Some(vec!["%d.%m.%Y".into()]),
            decimal_separator: Some(",".into()),
            aggregation: Some("weekly_sum".into()),
            ..ParsingDescriptor::default()
        };
        let expected = "{\"descriptor_version\":1,\"format\":\"csv\",\"encoding\":\"windows-1250\",\"delimiter\":\";\",\"header_row\":1,\"sheet\":null,\"key_columns\":[\"Kod towaru\"],\"key_separator\":\"|\",\"date_column\":\"Data sprzedaży\",\"quantity_column\":\"Ilość\",\"unit_column\":null,\"date_formats\":[\"%d.%m.%Y\"],\"decimal_separator\":\",\",\"thousands_separator\":null,\"aggregation\":\"weekly_sum\"}";
        assert_eq!(descriptor_json(&descriptor).unwrap(), expected);
        assert_eq!(
            descriptor_json(&descriptor).unwrap(),
            descriptor_json(&descriptor).unwrap()
        );
    }
}
