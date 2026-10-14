-- OpenDemand workspace database: model-input imputation recorded per run
-- (migration 003, ADR-0023).
--
-- Additive only. Migration 001 is frozen and must never be edited; this file
-- changes the schema exclusively through two nullable/defaulted columns.
-- Imputation is model input only: observation rows and series_qualification
-- stay exactly as imported.
--
-- Gate: the Rust core creates and verifies a pre-migration backup before it
-- applies this migration to an existing workspace (ADR-0008).

-- The imputation method a run planned with: 'last', 'last_seasonal_52',
-- 'mean_4', or NULL when the run used no imputation.
ALTER TABLE forecast_run ADD COLUMN imputation_method TEXT;

-- How many missing weeks inside the effective calendar this run filled for the
-- series (0 when none). Stored evidence is unaffected.
ALTER TABLE forecast_series_status ADD COLUMN imputed_weeks INTEGER NOT NULL DEFAULT 0;

-- End of migration 003.
