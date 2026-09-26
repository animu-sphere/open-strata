import importlib.util
import subprocess
import unittest
from pathlib import Path
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "validate_openusd_runtime", Path(__file__).resolve().parents[1] / "validate-openusd-runtime.py"
)
assert SPEC is not None and SPEC.loader is not None
VALIDATOR = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(VALIDATOR)


class RelocationTests(unittest.TestCase):
    def test_universal_binary_headers_are_not_dependencies(self):
        binary = Path("/runtime/lib/libqt.dylib")
        output = (
            f"{binary} (architecture x86_64):\n"
            "\t@rpath/libqt.dylib (compatibility version 1.0.0, current version 1.0.0)\n"
            f"{binary} (architecture arm64):\n"
            "\t/usr/lib/libSystem.B.dylib (compatibility version 1.0.0, current version 1.0.0)\n"
        )
        with patch.object(VALIDATOR, "mach_o_files", return_value=[binary]), patch.object(
            VALIDATOR, "contractual_absolute_dependencies", return_value=()
        ), patch.object(
            VALIDATOR.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, output, "")
        ):
            VALIDATOR.validate_relocation(Path("/runtime"), "macos")

            bad_output = output + "\t/usr/local/lib/libiodbc.dylib (compatibility version 1.0.0)\n"
            with patch.object(
                VALIDATOR.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, bad_output, "")
            ), self.assertRaisesRegex(RuntimeError, "libiodbc.dylib"):
                VALIDATOR.validate_relocation(Path("/runtime"), "macos")


if __name__ == "__main__":
    unittest.main()