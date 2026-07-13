# Build a portable Linux OpenUSD runtime

Build redistributable Linux runtimes in a container whose glibc is no newer
than the oldest runner you support. A binary linked on a newer distro cannot be
made compatible with an older glibc after packaging; the build environment owns
that floor.

The reference recipe uses the `manylinux_2_28_x86_64` image, its shared CPython
3.13, and a virtual environment with the Python packages required by OpenUSD
schema and usdview tooling. It also installs the X11 and Mesa development files
needed by imaging, GL, and MaterialX paths.

## 1. Choose the floor

Run `ldd --version` on the oldest target runner and select a builder with an
equal or older glibc. The reference image sets a 2.28 floor. Use an older base
if any supported runner is older; never substitute a newer Ubuntu/Fedora image
merely because it is convenient.

This is a compatibility floor, not a claim that every dependency is portable.
Keep CPU architecture, C++ ABI, driver requirements, and Python ABI consistent
with the target as well.

## 2. Build the reference image

From the OpenStrata repository root:

```bash
docker build \
  -f support/portable-linux-runtime.Dockerfile \
  -t openstrata-runtime-builder:glibc228 .
```

The image builds `ost` inside the same container. Copying an `ost` binary built
on a newer host would reintroduce the compatibility problem before the OpenUSD
build even starts.

The recipe downloads Rust, Python packages, and later OpenUSD dependencies.
Mirror or pin those inputs according to your production supply-chain policy;
the file is a build-environment reference, not a hermetic dependency lock.

## 3. Build and validate OpenUSD

Clone the intended OpenUSD revision on the host, then mount it read-only. Keep
`/work` in a named volume so the runtime store and upstream dependency builds
survive container restarts:

```bash
docker volume create openstrata-runtime-work
docker run --rm -it \
  -v openstrata-runtime-work:/work \
  -v "$PWD/../OpenUSD:/src/OpenUSD:ro" \
  openstrata-runtime-builder:glibc228 \
  ost runtime pull cy2026 --profile usd --build /src/OpenUSD --jobs 8

docker run --rm -it \
  -v openstrata-runtime-work:/work \
  openstrata-runtime-builder:glibc228 \
  ost runtime validate cy2026 --profile usd
```

The virtual environment is already active through `PATH`, and
`LD_LIBRARY_PATH` exposes the shared CPython 3.13 library. Do not replace it
with a system Python lacking `--enable-shared`: OpenUSD's Python and usdview
components link against `libpython`.

## 4. Export and check the measured target

After validation, export the runtime into the local artifact registry:

```bash
docker run --rm -it \
  -v openstrata-runtime-work:/work \
  openstrata-runtime-builder:glibc228 \
  ost runtime export cy2026 --profile usd
```

`ost` scans the packaged ELF files and records the measured glibc requirement in
the artifact target. Inspect the printed digest with `ost artifact show`; a
floor higher than intended means a dependency or toolchain escaped the chosen
base and the artifact must not be promoted to older runners.

Finally, pull the exported artifact on the oldest real runner and run
`ost runtime validate`. A container build proves the build floor; the oldest
consumer is the acceptance test for loader, driver, filesystem, and display
dependencies.

For OpenUSD downloader failure modes and the direct-CMake dependency-prefix
alternative, see the [runtime examples](examples.md#linux---build-prerequisites).
