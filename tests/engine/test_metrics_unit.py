"""Unit tests for WAPE/MAE metric computation and guards."""

from __future__ import annotations

import unittest

from opendemand_engine import errors, metrics


class MetricsUnitTests(unittest.TestCase):
    def test_wape_matches_formula(self):
        actuals = [10.0] * 11 + [20.0]
        predictions = [10.0] * 12
        wape, mae, kind, value = metrics.compute_metrics(actuals, predictions)
        self.assertAlmostEqual(wape, 10.0 / 130.0, places=15)
        self.assertAlmostEqual(mae, 10.0 / 12.0, places=15)
        self.assertEqual(kind, "wape")
        self.assertEqual(value, wape)

    def test_mae_fallback_when_actuals_all_zero(self):
        actuals = [0.0] * 12
        predictions = [5.0] * 12
        wape, mae, kind, value = metrics.compute_metrics(actuals, predictions)
        self.assertIsNone(wape)
        self.assertEqual(mae, 5.0)
        self.assertEqual(kind, "mae_fallback")
        self.assertEqual(value, mae)

    def test_negative_actuals_are_valid(self):
        actuals = [-12.0] * 11 + [-6.0]
        predictions = [-12.0] * 12
        wape, mae, kind, value = metrics.compute_metrics(actuals, predictions)
        self.assertAlmostEqual(wape, 6.0 / 138.0, places=15)
        self.assertAlmostEqual(mae, 0.5, places=15)
        self.assertEqual(kind, "wape")
        self.assertGreaterEqual(mae, 0.0)
        self.assertGreaterEqual(value, 0.0)

    def test_non_finite_inputs_raise_metric_error(self):
        with self.assertRaises(errors.TaskFailure) as caught:
            metrics.compute_metrics([1.0, float("nan")], [1.0, 1.0])
        self.assertEqual(caught.exception.code, "metric_error")

    def test_length_mismatch_raises_metric_error(self):
        with self.assertRaises(errors.TaskFailure) as caught:
            metrics.compute_metrics([1.0, 2.0], [1.0])
        self.assertEqual(caught.exception.code, "metric_error")


if __name__ == "__main__":
    unittest.main()
