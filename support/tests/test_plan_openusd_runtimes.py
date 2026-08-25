import copy
import importlib.util
import json
import unittest
from pathlib import Path


SUPPORT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "plan_openusd_runtimes", SUPPORT / "plan-openusd-runtimes.py"
)
assert SPEC is not None and SPEC.loader is not None
PLANNER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PLANNER)


def declaration():
    return json.loads((SUPPORT / "openusd-runtime-matrix.json").read_text(encoding="utf-8"))


class CanonicalRuntimePlannerTests(unittest.TestCase):
    def test_primary_matrix_expands_to_16_unique_ordered_leaves(self):
        jobs = PLANNER.expand(declaration())

        self.assertEqual(len(jobs), 16)
        self.assertEqual(len({job["tag"] for job in jobs}), 16)
        self.assertEqual(jobs[0]["tag"], "26.05-core-linux-x86_64")
        self.assertEqual(jobs[-1]["tag"], "26.08-metal-macos-arm64")
        self.assertTrue(all(job["examples_required"] == (job["variant"] != "core") for job in jobs))

    def test_macos_declares_no_gl_lane(self):
        jobs = PLANNER.expand(declaration())
        macos = {job["variant"] for job in jobs if job["os"] == "macos"}

        self.assertEqual(macos, {"core", "metal"})
        self.assertEqual([job for job in jobs if job["tag"].endswith("gl-macos-arm64")], [])

    def test_host_and_leaf_filters_preserve_declared_jobs(self):
        jobs = PLANNER.select_jobs(
            PLANNER.expand(declaration()),
            host="macos",
            arch="arm64",
            versions={"26.08"},
            variants={"metal"},
        )

        self.assertEqual([job["tag"] for job in jobs], ["26.08-metal-macos-arm64"])
        self.assertEqual(jobs[0]["sdk"], "15.5")
        self.assertEqual(jobs[0]["deployment_target"], "13.0")

    def test_host_and_arch_are_an_atomic_filter(self):
        with self.assertRaisesRegex(ValueError, "provided together"):
            PLANNER.select_jobs(PLANNER.expand(declaration()), host="linux")

    def test_reordered_variants_are_rejected(self):
        document = copy.deepcopy(declaration())
        document["cells"][0]["variants"] = ["gl", "core", "vulkan"]

        with self.assertRaisesRegex(ValueError, "not canonical"):
            PLANNER.expand(document)

    def test_duplicate_platform_cells_are_rejected(self):
        document = copy.deepcopy(declaration())
        document["cells"][1] = copy.deepcopy(document["cells"][0])

        with self.assertRaisesRegex(ValueError, "more than one linux"):
            PLANNER.expand(document)

    def test_publication_policy_drift_is_rejected(self):
        document = copy.deepcopy(declaration())
        document["publication"]["multi_platform_aliases"] = True

        with self.assertRaisesRegex(ValueError, "publication policy"):
            PLANNER.expand(document)


if __name__ == "__main__":
    unittest.main()
