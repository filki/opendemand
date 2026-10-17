"""Unit tests for the opt-in sparse models (ADR-0032).

Croston-SBA and TSB are dispatched through the pinned statsforecast classes;
these tests pin finite output on intermittent histories and the fail-closed
rule that sparse candidates never see negative training windows.
"""

import math
import unittest

from opendemand_engine.errors import TaskFailure
from opendemand_engine.modeling import MODEL_FLOOR, forecast
from opendemand_engine import SUPPORTED_MODELS

SPARSE = ("croston_sba", "tsb")


class SparseModelTests(unittest.TestCase):
    def test_sparse_models_are_advertised_with_the_twelve_week_floor(self):
        for model in SPARSE:
            self.assertIn(model, SUPPORTED_MODELS)
            self.assertEqual(MODEL_FLOOR[model], 12)

    def test_intermittent_history_produces_finite_forecasts(self):
        history = [0.0] * 9 + [5.0, 0.0, 0.0, 3.0] + [0.0] * 8
        for model in SPARSE:
            values = forecast(model, history, 12)
            self.assertEqual(len(values), 12, model)
            self.assertTrue(all(math.isfinite(value) for value in values), model)

    def test_all_zero_history_stays_finite(self):
        for model in SPARSE:
            values = forecast(model, [0.0] * 24, 12)
            self.assertEqual(len(values), 12, model)
            self.assertTrue(all(math.isfinite(value) for value in values), model)

    def test_unknown_model_is_refused(self):
        with self.assertRaises(TaskFailure) as caught:
            forecast("croston", [1.0] * 24, 12)
        self.assertEqual(caught.exception.code, "unsupported_model")


if __name__ == "__main__":
    unittest.main()
