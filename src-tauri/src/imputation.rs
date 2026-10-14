//! Model-input imputation of missing weeks (ADR-0023).
//!
//! Imputation is run-scoped and input-only: it fills the series copy that the
//! forecast planner hands to the engine. Stored observations, `H` in
//! `series_qualification`, exports, and the series chart stay as imported.
//! Weeks filled here are excluded from backtest scoring by the planner.

use crate::types::InputSeries;

/// The imputation methods a run may request (ADR-0023).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImputationMethod {
    /// Last known value carried forward (LOCF).
    Last,
    /// The value of the same week one year earlier, stepping back by whole
    /// years through originally known weeks only; falls back to `Last`.
    LastSeasonal52,
    /// Mean of up to the four most recent originally known values.
    Mean4,
}

impl ImputationMethod {
    /// Stable method ids shared with the run record and the IPC surface.
    pub const ALL: [(&'static str, ImputationMethod); 3] = [
        ("last", ImputationMethod::Last),
        ("last_seasonal_52", ImputationMethod::LastSeasonal52),
        ("mean_4", ImputationMethod::Mean4),
    ];

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|(candidate, _)| *candidate == id)
            .map(|(_, method)| method)
    }

    pub fn id(self) -> &'static str {
        Self::ALL
            .into_iter()
            .find(|(_, candidate)| *candidate == self)
            .map(|(id, _)| id)
            .expect("every method has an id")
    }
}

/// Fill the internal and trailing gaps of one series' effective calendar.
///
/// The effective calendar starts at the first originally known week and ends at
/// the cutoff; leading weeks without a value are dropped, not filled. The
/// series' `h_length` becomes the filled calendar length, and a series that
/// reaches the Naive floor becomes plannable in this run. Returns the number of
/// filled weeks.
pub fn impute_series(series: &mut InputSeries, method: ImputationMethod) -> u32 {
    let Some(anchor) = series
        .observations
        .iter()
        .position(|observation| observation.quantity.is_some())
    else {
        return 0;
    };
    if anchor > 0 {
        series.observations.drain(..anchor);
    }
    let known: Vec<bool> = series
        .observations
        .iter()
        .map(|observation| observation.quantity.is_some())
        .collect();
    let mut filled = Vec::new();
    for index in 0..series.observations.len() {
        if known[index] {
            continue;
        }
        let value = match method {
            ImputationMethod::Last => previous_value(&series.observations, index),
            ImputationMethod::LastSeasonal52 => seasonal_value(&series.observations, &known, index)
                .or_else(|| previous_value(&series.observations, index)),
            ImputationMethod::Mean4 => mean_of_known(&series.observations, &known, index),
        };
        let value = value.expect("weeks after the anchor always have a previous value");
        series.observations[index].quantity = Some(value);
        filled.push(index as u32);
    }
    let effective = series.observations.len() as u32;
    if effective != series.h_length {
        series.h_length = effective;
        if effective >= 12 && series.qualification_status != "eligible" {
            series.qualification_status = "eligible".to_string();
            series.reason = None;
        }
    }
    series.imputed_indices = filled.clone();
    filled.len() as u32
}

/// The value of the nearest preceding week, known or already filled.
fn previous_value(observations: &[crate::types::ObservationRecord], index: usize) -> Option<f64> {
    observations[..index]
        .iter()
        .rev()
        .find_map(|observation| observation.quantity)
}

/// The nearest originally known value 52, 104, 156, … weeks back.
fn seasonal_value(
    observations: &[crate::types::ObservationRecord],
    known: &[bool],
    index: usize,
) -> Option<f64> {
    let mut back = index as i64 - 52;
    while back >= 0 {
        let position = back as usize;
        if known[position] {
            return observations[position].quantity;
        }
        back -= 52;
    }
    None
}

/// The mean of up to the four most recent originally known values.
fn mean_of_known(
    observations: &[crate::types::ObservationRecord],
    known: &[bool],
    index: usize,
) -> Option<f64> {
    let values: Vec<f64> = (0..index)
        .rev()
        .filter(|position| known[*position])
        .take(4)
        .filter_map(|position| observations[position].quantity)
        .collect();
    if values.is_empty() {
        return None;
    }
    Some(values.iter().sum::<f64>() / values.len() as f64)
}

#[cfg(test)]
mod tests {
    use super::{impute_series, ImputationMethod};
    use crate::types::{InputSeries, ObservationRecord};
    use chrono::NaiveDate;
    use uuid::Uuid;

    fn series(values: &[Option<f64>], h_length: u32, status: &str) -> InputSeries {
        let start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let observations = values
            .iter()
            .enumerate()
            .map(|(index, quantity)| ObservationRecord {
                week_start: start + chrono::Duration::weeks(index as i64),
                quantity: *quantity,
                status: if quantity.is_some() {
                    "known".to_string()
                } else {
                    "missing".to_string()
                },
            })
            .collect();
        InputSeries {
            product_id: Uuid::now_v7(),
            location_id: Uuid::now_v7(),
            series_key: "TEST-1".to_string(),
            h_length,
            qualification_status: status.to_string(),
            reason: None,
            observations,
            imputed_indices: Vec::new(),
        }
    }

    fn values(series: &InputSeries) -> Vec<Option<f64>> {
        series
            .observations
            .iter()
            .map(|observation| observation.quantity)
            .collect()
    }

    #[test]
    fn last_fills_internal_and_trailing_gaps() {
        let mut series = series(&[Some(1.0), None, Some(3.0), None, None], 1, "eligible");
        let filled = impute_series(&mut series, ImputationMethod::Last);
        assert_eq!(filled, 3);
        assert_eq!(series.imputed_indices, vec![1, 3, 4]);
        assert_eq!(
            values(&series),
            vec![Some(1.0), Some(1.0), Some(3.0), Some(3.0), Some(3.0)]
        );
    }

    #[test]
    fn last_seasonal_52_prefers_the_same_week_one_year_back() {
        let mut observations = vec![Some(1.0); 56];
        observations[3] = Some(9.0);
        observations[55] = None;
        let mut series = series(&observations, 54, "eligible");
        let filled = impute_series(&mut series, ImputationMethod::LastSeasonal52);
        assert_eq!(filled, 1);
        // 55 - 52 = 3, originally known with 9.0.
        assert_eq!(series.observations[55].quantity, Some(9.0));
    }

    #[test]
    fn last_seasonal_52_ignores_filled_weeks_and_falls_back_to_last() {
        let mut observations = vec![Some(1.0); 110];
        observations[4] = None;
        observations[56] = None;
        observations[107] = Some(9.0);
        observations[108] = None;
        let mut series = series(&observations, 105, "eligible");
        let filled = impute_series(&mut series, ImputationMethod::LastSeasonal52);
        assert_eq!(filled, 3);
        // Weeks 4 and 56 have no originally known year-back step: LOCF.
        assert_eq!(series.observations[4].quantity, Some(1.0));
        assert_eq!(series.observations[56].quantity, Some(1.0));
        // Week 108 steps back over the filled week 56 to unknown week 4 and
        // falls back to the last value (9.0), not to the filled 1.0.
        assert_eq!(series.observations[108].quantity, Some(9.0));
    }

    #[test]
    fn last_seasonal_52_uses_an_older_year_when_the_last_one_is_unknown() {
        let mut observations = vec![Some(1.0); 110];
        observations[4] = Some(40.0);
        observations[56] = None;
        observations[108] = None;
        let mut series = series(&observations, 105, "eligible");
        let _ = impute_series(&mut series, ImputationMethod::LastSeasonal52);
        // Week 56 fills from week 4; week 108 skips the filled week 56 and
        // takes the same value from the originally known week 4.
        assert_eq!(series.observations[56].quantity, Some(40.0));
        assert_eq!(series.observations[108].quantity, Some(40.0));
    }

    #[test]
    fn last_seasonal_52_falls_back_to_last_without_a_year() {
        let mut series = series(&[Some(2.0), Some(4.0), None], 2, "eligible");
        let _ = impute_series(&mut series, ImputationMethod::LastSeasonal52);
        assert_eq!(values(&series), vec![Some(2.0), Some(4.0), Some(4.0)]);
    }

    #[test]
    fn mean_4_averages_the_known_values_before_the_gap() {
        let mut series = series(
            &[
                Some(10.0),
                None,
                Some(20.0),
                None,
                Some(30.0),
                Some(40.0),
                None,
            ],
            1,
            "eligible",
        );
        let filled = impute_series(&mut series, ImputationMethod::Mean4);
        assert_eq!(filled, 3);
        assert_eq!(series.observations[1].quantity, Some(10.0));
        assert_eq!(series.observations[3].quantity, Some(15.0));
        assert_eq!(series.observations[6].quantity, Some(25.0));
    }

    #[test]
    fn leading_gap_is_dropped_not_filled() {
        let mut series = series(&[None, None, Some(7.0), None], 1, "ineligible");
        let filled = impute_series(&mut series, ImputationMethod::Last);
        assert_eq!(filled, 1);
        assert_eq!(series.observations.len(), 2);
        assert_eq!(values(&series), vec![Some(7.0), Some(7.0)]);
        assert_eq!(series.imputed_indices, vec![1]);
    }

    #[test]
    fn filling_to_the_naive_floor_unlocks_planning_eligibility() {
        let mut observations = vec![Some(1.0); 13];
        observations[5] = None;
        let mut series = series(&observations, 7, "ineligible");
        let filled = impute_series(&mut series, ImputationMethod::Last);
        assert_eq!(filled, 1);
        assert_eq!(series.h_length, 13);
        assert_eq!(series.qualification_status, "eligible");
    }

    #[test]
    fn series_without_any_known_week_stays_untouched() {
        let mut series = series(&[None, None], 0, "ineligible");
        let filled = impute_series(&mut series, ImputationMethod::Last);
        assert_eq!(filled, 0);
        assert_eq!(series.observations.len(), 2);
        assert!(series.imputed_indices.is_empty());
    }
}
