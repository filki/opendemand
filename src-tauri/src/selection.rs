//! Champion selection policy for one forecast run x series (ADR-0006, ADR-0012,
//! ADR-0017).
//!
//! Rust is the sole owner of production eligibility, champion ranking, and the
//! production fallback chain. Models are ranked per series on the origin dates
//! every champion-eligible candidate completed, so a model with four origins is
//! never scored against another model as if the extra evidence were shared. The
//! comparable score pools held-out errors over that common evaluation set:
//! pooled WAPE when the comparable actual magnitude is positive, pooled MAE
//! otherwise. Pooling keeps one unit per comparison and never averages WAPE
//! ratios with MAE values.

use chrono::NaiveDate;

/// Complexity order, used as the deterministic tie-break (ADR-0006).
pub const MODELS: [&str; 3] = ["naive", "seasonal_naive_52", "auto_ets_52"];

/// Exact held-out sums over one completed backtest origin.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct OriginSums {
    pub points: u32,
    /// Sum of |actual - forecast| over the origin horizon.
    pub absolute_error: f64,
    /// Sum of |actual| over the origin horizon.
    pub actual_magnitude: f64,
    /// Sum of (forecast - actual); positive means overforecasting.
    pub signed_error: f64,
}

#[derive(Debug, Clone)]
pub struct ModelEvidence {
    pub model_id: String,
    pub production_eligible: bool,
    pub production_completed: bool,
    /// Completed origins as (origin date, held-out sums); order is irrelevant.
    pub completed: Vec<(NaiveDate, OriginSums)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelScore {
    pub model_id: String,
    pub score: f64,
    /// `wape` when the comparable actual magnitude is positive, else `mae`.
    pub metric_kind: &'static str,
}

#[derive(Debug, Clone, Default)]
pub struct Selection {
    /// Best ranked champion-eligible model, if any comparison was defensible.
    pub champion: Option<String>,
    /// Model whose production forecast is persisted, if any succeeded.
    pub used: Option<String>,
    /// Ranked candidates, best first.
    pub ranked: Vec<ModelScore>,
}

/// Pooled score over one comparable evaluation set.
pub fn pooled(origins: &[(NaiveDate, OriginSums)]) -> Option<(&'static str, f64)> {
    let mut error = 0.0;
    let mut magnitude = 0.0;
    let mut points = 0u32;
    for (_, sums) in origins {
        error += sums.absolute_error;
        magnitude += sums.actual_magnitude;
        points += sums.points;
    }
    if points == 0 {
        return None;
    }
    Some(if magnitude > 0.0 {
        ("wape", error / magnitude)
    } else {
        ("mae", error / points as f64)
    })
}

/// Diagnostic metrics over all completed origins of one model. Bias carries
/// the sign convention "+ = overforecasting" and is unavailable (None) when
/// the actual magnitude is zero, because the ratio is then undefined.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Diagnostics {
    pub wape: Option<f64>,
    pub mae: Option<f64>,
    pub bias: Option<f64>,
}

pub fn diagnostics(origins: &[(NaiveDate, OriginSums)]) -> Diagnostics {
    let mut error = 0.0;
    let mut magnitude = 0.0;
    let mut signed = 0.0;
    let mut points = 0u32;
    for (_, sums) in origins {
        error += sums.absolute_error;
        magnitude += sums.actual_magnitude;
        signed += sums.signed_error;
        points += sums.points;
    }
    Diagnostics {
        wape: (magnitude > 0.0).then(|| error / magnitude),
        mae: (points > 0).then(|| error / points as f64),
        bias: (magnitude > 0.0).then(|| signed / magnitude),
    }
}

fn complexity(model_id: &str) -> usize {
    MODELS
        .iter()
        .position(|candidate| *candidate == model_id)
        .unwrap_or(MODELS.len())
}

/// Rank champion-eligible models (at least two completed origins) on the origin
/// dates every such candidate completed. Returns an empty ranking when no
/// comparison is defensible (no candidate, or no common completed origin date).
pub fn rank(models: &[ModelEvidence]) -> Vec<ModelScore> {
    let candidates: Vec<&ModelEvidence> = models
        .iter()
        .filter(|model| model.completed.len() >= 2)
        .collect();
    let Some(first) = candidates.first() else {
        return Vec::new();
    };
    let mut common: Vec<NaiveDate> = first.completed.iter().map(|(date, _)| *date).collect();
    for candidate in &candidates[1..] {
        common.retain(|date| candidate.completed.iter().any(|(other, _)| other == date));
    }
    if common.is_empty() {
        return Vec::new();
    }
    let mut scores = Vec::new();
    for candidate in candidates {
        let set: Vec<(NaiveDate, OriginSums)> = common
            .iter()
            .filter_map(|date| {
                candidate
                    .completed
                    .iter()
                    .find(|(other, _)| other == date)
                    .copied()
            })
            .collect();
        if let Some((metric_kind, score)) = pooled(&set) {
            scores.push(ModelScore {
                model_id: candidate.model_id.clone(),
                score,
                metric_kind,
            });
        }
    }
    scores.sort_by(|left, right| {
        left.score
            .total_cmp(&right.score)
            .then_with(|| complexity(&left.model_id).cmp(&complexity(&right.model_id)))
    });
    scores
}

/// Production attempt order: ranked candidates best first, then Naive when it
/// is production eligible. A model that was never ranked is never used for
/// production because it has no comparable evidence (ADR-0017).
fn production_order(models: &[ModelEvidence], ranked: &[ModelScore]) -> Vec<String> {
    let mut order: Vec<String> = ranked.iter().map(|score| score.model_id.clone()).collect();
    let naive_eligible = models
        .iter()
        .any(|model| model.model_id == "naive" && model.production_eligible);
    if naive_eligible && !order.contains(&"naive".to_string()) {
        order.push("naive".into());
    }
    order
}

/// Select the champion and the model actually used for production. The
/// champion is the statistically preferred model; `used` is the first model in
/// the fallback chain whose production forecast completed, and the two differ
/// when the champion production task failed.
pub fn select(models: &[ModelEvidence]) -> Selection {
    let ranked = rank(models);
    let champion = ranked.first().map(|score| score.model_id.clone());
    let used = production_order(models, &ranked)
        .into_iter()
        .find(|model_id| {
            models
                .iter()
                .any(|model| model.model_id == *model_id && model.production_completed)
        });
    Selection {
        champion,
        used,
        ranked,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(week: &str) -> NaiveDate {
        NaiveDate::parse_from_str(week, "%Y-%m-%d").unwrap()
    }

    fn sums(error: f64, magnitude: f64, signed: f64) -> OriginSums {
        OriginSums {
            points: 12,
            absolute_error: error,
            actual_magnitude: magnitude,
            signed_error: signed,
        }
    }

    fn model(
        model_id: &str,
        completed: Vec<(&str, OriginSums)>,
        production_completed: bool,
    ) -> ModelEvidence {
        ModelEvidence {
            model_id: model_id.into(),
            production_eligible: true,
            production_completed,
            completed: completed
                .into_iter()
                .map(|(week, sums)| (date(week), sums))
                .collect(),
        }
    }

    #[test]
    fn fewer_than_two_completed_origins_is_not_champion_eligible() {
        let evidence = [model(
            "naive",
            vec![("2026-01-05", sums(1.0, 100.0, 0.0))],
            true,
        )];
        let selection = select(&evidence);
        assert!(selection.champion.is_none());
        assert!(selection.ranked.is_empty());
        assert_eq!(selection.used.as_deref(), Some("naive"));
    }

    #[test]
    fn single_candidate_pools_its_own_completed_origins() {
        let evidence = [model(
            "naive",
            vec![
                ("2026-01-05", sums(10.0, 120.0, 0.0)),
                ("2026-01-12", sums(20.0, 240.0, 0.0)),
            ],
            true,
        )];
        let selection = select(&evidence);
        assert_eq!(selection.champion.as_deref(), Some("naive"));
        assert_eq!(selection.ranked.len(), 1);
        assert_eq!(selection.ranked[0].score, 30.0 / 360.0);
        assert_eq!(selection.ranked[0].metric_kind, "wape");
    }

    #[test]
    fn ranking_uses_only_origin_dates_every_candidate_completed() {
        // Naive has four origins; SeasonalNaive has the two most recent. The
        // comparison must use the two shared origins, not Naive's full set.
        let evidence = [
            model(
                "naive",
                vec![
                    ("2025-01-06", sums(1000.0, 1000.0, 0.0)),
                    ("2025-01-13", sums(1000.0, 1000.0, 0.0)),
                    ("2026-01-05", sums(12.0, 120.0, 0.0)),
                    ("2026-01-12", sums(24.0, 240.0, 0.0)),
                ],
                true,
            ),
            model(
                "seasonal_naive_52",
                vec![
                    ("2026-01-05", sums(6.0, 120.0, 0.0)),
                    ("2026-01-12", sums(12.0, 240.0, 0.0)),
                ],
                true,
            ),
        ];
        let selection = select(&evidence);
        assert_eq!(selection.ranked.len(), 2);
        assert_eq!(selection.ranked[0].model_id, "seasonal_naive_52");
        assert!((selection.ranked[0].score - 18.0 / 360.0).abs() < 1e-12);
        assert_eq!(selection.ranked[1].model_id, "naive");
        assert!((selection.ranked[1].score - 36.0 / 360.0).abs() < 1e-12);
        assert_eq!(selection.champion.as_deref(), Some("seasonal_naive_52"));
        assert_eq!(selection.used.as_deref(), Some("seasonal_naive_52"));
    }

    #[test]
    fn zero_actual_magnitude_uses_pooled_mae_for_every_candidate() {
        let evidence = [
            model(
                "naive",
                vec![
                    ("2026-01-05", sums(12.0, 0.0, 12.0)),
                    ("2026-01-12", sums(24.0, 0.0, 24.0)),
                ],
                true,
            ),
            model(
                "seasonal_naive_52",
                vec![
                    ("2026-01-05", sums(6.0, 0.0, -6.0)),
                    ("2026-01-12", sums(12.0, 0.0, -12.0)),
                ],
                true,
            ),
        ];
        let selection = select(&evidence);
        assert_eq!(selection.ranked[0].model_id, "seasonal_naive_52");
        assert_eq!(selection.ranked[0].metric_kind, "mae");
        assert!((selection.ranked[0].score - 18.0 / 24.0).abs() < 1e-12);
        assert_eq!(selection.ranked[1].metric_kind, "mae");
        assert!((selection.ranked[1].score - 36.0 / 24.0).abs() < 1e-12);
    }

    #[test]
    fn equal_scores_break_by_documented_complexity_order() {
        let evidence = [
            model(
                "auto_ets_52",
                vec![
                    ("2026-01-05", sums(12.0, 120.0, 0.0)),
                    ("2026-01-12", sums(24.0, 240.0, 0.0)),
                ],
                true,
            ),
            model(
                "seasonal_naive_52",
                vec![
                    ("2026-01-05", sums(12.0, 120.0, 0.0)),
                    ("2026-01-12", sums(24.0, 240.0, 0.0)),
                ],
                true,
            ),
            model(
                "naive",
                vec![
                    ("2026-01-05", sums(12.0, 120.0, 0.0)),
                    ("2026-01-12", sums(24.0, 240.0, 0.0)),
                ],
                true,
            ),
        ];
        let selection = select(&evidence);
        assert_eq!(
            selection
                .ranked
                .iter()
                .map(|score| score.model_id.as_str())
                .collect::<Vec<_>>(),
            vec!["naive", "seasonal_naive_52", "auto_ets_52"]
        );
        assert_eq!(selection.champion.as_deref(), Some("naive"));
    }

    #[test]
    fn disjoint_completed_origins_skip_unranked_models_in_fallback() {
        let evidence = [
            model(
                "naive",
                vec![
                    ("2026-01-05", sums(12.0, 120.0, 0.0)),
                    ("2026-01-12", sums(24.0, 240.0, 0.0)),
                ],
                false,
            ),
            model(
                "seasonal_naive_52",
                vec![
                    ("2026-01-19", sums(6.0, 120.0, 0.0)),
                    ("2026-01-26", sums(12.0, 240.0, 0.0)),
                ],
                true,
            ),
        ];
        let selection = select(&evidence);
        // No common origin date means no ranking and no champion. SeasonalNaive
        // is champion-eligible but unranked, so it must not serve production;
        // Naive is the only fallback candidate and its production failed.
        assert!(selection.champion.is_none());
        assert!(selection.ranked.is_empty());
        assert!(selection.used.is_none());
    }

    #[test]
    fn disjoint_completed_origins_still_fall_back_to_naive() {
        let evidence = [
            model(
                "naive",
                vec![
                    ("2026-01-05", sums(12.0, 120.0, 0.0)),
                    ("2026-01-12", sums(24.0, 240.0, 0.0)),
                ],
                true,
            ),
            model(
                "seasonal_naive_52",
                vec![
                    ("2026-01-19", sums(6.0, 120.0, 0.0)),
                    ("2026-01-26", sums(12.0, 240.0, 0.0)),
                ],
                true,
            ),
        ];
        let selection = select(&evidence);
        assert!(selection.champion.is_none());
        assert_eq!(selection.used.as_deref(), Some("naive"));
    }

    #[test]
    fn failed_champion_production_falls_back_to_next_ranked_model() {
        let evidence = [
            model(
                "naive",
                vec![
                    ("2026-01-05", sums(12.0, 120.0, 0.0)),
                    ("2026-01-12", sums(24.0, 240.0, 0.0)),
                ],
                true,
            ),
            model(
                "seasonal_naive_52",
                vec![
                    ("2026-01-05", sums(6.0, 120.0, 0.0)),
                    ("2026-01-12", sums(12.0, 240.0, 0.0)),
                ],
                false,
            ),
            model("auto_ets_52", vec![], false),
        ];
        let selection = select(&evidence);
        // SeasonalNaive wins the comparison but its production task failed;
        // Naive is the next ranked model and produces the persisted forecast.
        assert_eq!(selection.champion.as_deref(), Some("seasonal_naive_52"));
        assert_eq!(selection.used.as_deref(), Some("naive"));
    }

    #[test]
    fn no_successful_production_leaves_no_used_model() {
        let evidence = [
            model(
                "naive",
                vec![
                    ("2026-01-05", sums(12.0, 120.0, 0.0)),
                    ("2026-01-12", sums(24.0, 240.0, 0.0)),
                ],
                false,
            ),
            model(
                "seasonal_naive_52",
                vec![
                    ("2026-01-05", sums(6.0, 120.0, 0.0)),
                    ("2026-01-12", sums(12.0, 240.0, 0.0)),
                ],
                false,
            ),
        ];
        let selection = select(&evidence);
        assert_eq!(selection.champion.as_deref(), Some("seasonal_naive_52"));
        assert!(selection.used.is_none());
    }

    #[test]
    fn diagnostics_report_pooled_metrics_and_signed_bias() {
        let origins = [
            ("2026-01-05", sums(12.0, 120.0, 6.0)),
            ("2026-01-12", sums(24.0, 240.0, -18.0)),
        ]
        .map(|(week, sums)| (date(week), sums));
        let report = diagnostics(&origins);
        assert_eq!(report.wape, Some(36.0 / 360.0));
        assert_eq!(report.mae, Some(36.0 / 24.0));
        assert_eq!(report.bias, Some(-12.0 / 360.0));
        let zero = [(date("2026-01-05"), sums(0.0, 0.0, 0.0))];
        let report = diagnostics(&zero);
        assert_eq!(report.wape, None);
        assert_eq!(report.bias, None);
        assert_eq!(report.mae, Some(0.0));
    }
}
