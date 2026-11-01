# SPDX-License-Identifier: Apache-2.0
# Embedded by python.rs and passed directly to the interpreter with -c.
import os
import sys
import sysconfig

version = '%d.%d' % sys.version_info[:2]
print(sys.executable)
print(version)
print(sysconfig.get_path('include') or '')

libdir = sysconfig.get_config_var('LIBDIR') or ''
name = sysconfig.get_config_var('LDLIBRARY') or ''
multiarch = sysconfig.get_config_var('MULTIARCH') or ''
framework = sysconfig.get_config_var('PYTHONFRAMEWORK') or ''
framework_prefix = sysconfig.get_config_var('PYTHONFRAMEWORKPREFIX') or ''

if os.name == 'nt':
    candidates = [os.path.join(sys.base_prefix, 'libs', 'python' + version.replace('.', '') + '.lib')]
else:
    candidates = [os.path.join(libdir, name)] if libdir and name else []
    if libdir and name and multiarch:
        candidates.append(os.path.join(libdir, multiarch, name))
    if framework and framework_prefix:
        # Framework LDLIBRARY may be relative to the framework's parent,
        # rather than LIBDIR (which is inside Versions/X.Y/lib).
        if name:
            candidates.append(os.path.join(framework_prefix, name))
        candidates.append(os.path.join(
            framework_prefix, framework + '.framework', 'Versions', version, framework
        ))

# Do not turn a directory or an absent library into a Development hint.
print(next((path for path in candidates if os.path.isfile(path)), ''))
