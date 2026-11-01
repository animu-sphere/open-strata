# SPDX-License-Identifier: Apache-2.0
import contextlib
import io
import os
from pathlib import Path
import subprocess
import sys
import sysconfig
import tempfile
import unittest
from unittest.mock import patch


PROBE = (Path(__file__).resolve().parents[2] / 'crates/ost-build/src/python_probe.py').read_text()


class PythonProbeTests(unittest.TestCase):
    def probe(self, config, platform='posix', base_prefix=''):
        output = io.StringIO()
        with patch.object(sysconfig, 'get_config_var', side_effect=config.get), patch.object(
            sysconfig, 'get_path', return_value='/selected/include'
        ), patch.object(os, 'name', platform), patch.object(
            sys, 'base_prefix', base_prefix
        ), contextlib.redirect_stdout(output):
            exec(compile(PROBE, 'python_probe.py', 'exec'), {})
        lines = output.getvalue().splitlines()
        self.assertEqual(lines[:3], [sys.executable, '%d.%d' % sys.version_info[:2], '/selected/include'])
        return lines[3]

    def test_framework_relative_library(self):
        with tempfile.TemporaryDirectory() as root:
            version = '%d.%d' % sys.version_info[:2]
            relative = f'Python.framework/Versions/{version}/Python'
            library = Path(root) / relative
            library.parent.mkdir(parents=True)
            library.touch()
            config = {
                'LIBDIR': str(library.parent / 'lib'),
                'LDLIBRARY': relative,
                'PYTHONFRAMEWORK': 'Python',
                'PYTHONFRAMEWORKPREFIX': root,
            }
            self.assertEqual(self.probe(config), os.path.join(root, relative))
            alias = library.parent / 'lib' / f'libpython{version}.dylib'
            alias.parent.mkdir()
            alias.touch()
            self.assertEqual(Path(self.probe(config)), alias)
            alias.unlink()
            # Some framework builds report just the binary's basename.
            config['LDLIBRARY'] = 'Python'
            self.assertEqual(Path(self.probe(config)), library)
            library.unlink()
            self.assertEqual(self.probe(config), '')
            library.mkdir()
            self.assertEqual(self.probe(config), '')

    def test_posix_and_multiarch_preserve_precedence(self):
        with tempfile.TemporaryDirectory() as root:
            primary = Path(root) / 'libpython3.13.so'
            alternate = Path(root) / 'arch' / primary.name
            alternate.parent.mkdir()
            alternate.touch()
            config = {'LIBDIR': root, 'LDLIBRARY': primary.name, 'MULTIARCH': 'arch'}
            self.assertEqual(Path(self.probe(config)), alternate)
            primary.touch()
            self.assertEqual(Path(self.probe(config)), primary)
            primary.unlink()
            primary.mkdir()
            self.assertEqual(Path(self.probe(config)), alternate)

    def test_windows_uses_base_prefix_import_library(self):
        with tempfile.TemporaryDirectory() as root:
            library = Path(root) / 'libs' / ('python%d%d.lib' % sys.version_info[:2])
            library.parent.mkdir()
            library.touch()
            self.assertEqual(Path(self.probe({}, 'nt', root)), library)
            library.unlink()
            self.assertEqual(self.probe({}, 'nt', root), '')

    def test_missing_metadata_does_not_search_current_directory(self):
        self.assertEqual(self.probe({}), '')
        self.assertEqual(self.probe({'LDLIBRARY': sys.executable}), '')

    def test_framework_candidates_require_framework_declaration(self):
        with tempfile.TemporaryDirectory() as root:
            library = Path(root) / 'Python'
            library.touch()
            self.assertEqual(self.probe({
                'LDLIBRARY': library.name, 'PYTHONFRAMEWORKPREFIX': root
            }), '')

    def test_selected_interpreter_has_real_development_artifacts(self):
        result = subprocess.run([sys.executable, '-c', PROBE], check=True, text=True, capture_output=True)
        executable, version, include, library = result.stdout.splitlines()
        self.assertEqual(executable, sys.executable)
        self.assertEqual(version, '%d.%d' % sys.version_info[:2])
        self.assertTrue((Path(include) / 'Python.h').is_file(), include)
        self.assertTrue(Path(library).is_file(), library)
        if sys.platform == 'darwin':
            self.assertTrue(sysconfig.get_config_var('PYTHONFRAMEWORK'), 'CI must exercise framework Python')


if __name__ == '__main__':
    unittest.main()
