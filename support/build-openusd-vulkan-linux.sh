#!/usr/bin/env bash
set -euo pipefail

version="${1:?OpenUSD version is required (26.05 or 26.08)}"
jobs="${2:?parallel job count is required}"
output_name="${3:?output directory name is required}"
openstrata_revision="${4:?OpenStrata revision is required}"
expected_python_version="${5:?expected Python patch version is required}"

case "${version}" in
  26.05|26.08) ;;
  *) echo "unsupported OpenUSD version: ${version}" >&2; exit 2 ;;
esac

source_dir=/work/OpenUSD
export OST_HOME=/work/.ost
output_root="/out/${output_name}"
dist_dir="${output_root}/dist"
metadata_file="${output_root}/build-metadata.json"
runtime_root="${OST_HOME}/runtimes/openstrata-cy2026-linux-x86_64-py313-usd"
tag="v${version}"

mkdir -p "${output_root}"
if [[ -e "${dist_dir}" ]]; then
  echo "refusing to overwrite existing dist directory: ${dist_dir}" >&2
  exit 2
fi

if [[ ! -d "${source_dir}/.git" ]]; then
  git clone --branch "${tag}" --depth 1 \
    https://github.com/PixarAnimationStudios/OpenUSD.git "${source_dir}"
else
  if [[ -n "$(git -C "${source_dir}" status --porcelain)" ]]; then
    echo "OpenUSD source checkout is dirty: ${source_dir}" >&2
    exit 2
  fi
  git -C "${source_dir}" fetch --depth 1 origin "refs/tags/${tag}:refs/tags/${tag}"
  git -C "${source_dir}" checkout --detach "${tag}"
fi

source_revision="$(git -C "${source_dir}" rev-parse HEAD)"
python_version="$(python -c 'import sys; print(".".join(map(str, sys.version_info[:3])))')"
if [[ "${python_version}" != "${expected_python_version}" ]]; then
  echo "Python ${expected_python_version} is required; found ${python_version}" >&2
  exit 2
fi
cat >"${metadata_file}" <<EOF
{
  "source": {
    "repository": "https://github.com/PixarAnimationStudios/OpenUSD",
    "revision": "${source_revision}"
  },
  "builder": {
    "id": "https://github.com/animu-sphere/open-strata/blob/${openstrata_revision}/support/build-openusd-vulkan-linux.sh",
    "identity": {
      "host": "wsl2-docker",
      "pipeline": "openusd-vulkan-runtime",
      "git_ref": "${tag}",
      "platform": "linux-x86_64",
      "python": "${python_version}",
      "vulkan_sdk": "headers+utility-1.4.350+vma-3.4.0+ubuntu-24.04-loader+shaderc"
    }
  }
}
EOF

build_args=(
  --openusd-variant vulkan
  --build-arg --examples
)
if [[ "${version}" == "26.08" ]]; then
  build_args+=(--build-arg --python-install-dir=lib/python)
fi

ost runtime pull cy2026 \
  --profile usd \
  --build "${source_dir}" \
  --jobs "${jobs}" \
  --force \
  "${build_args[@]}"

xvfb_log="${output_root}/xvfb.log"
Xvfb :99 -screen 0 1280x1024x24 -nolisten tcp >"${xvfb_log}" 2>&1 &
xvfb_pid=$!
cleanup_xvfb() {
  kill "${xvfb_pid}" 2>/dev/null || true
  wait "${xvfb_pid}" 2>/dev/null || true
}
trap cleanup_xvfb EXIT
export DISPLAY=:99
sleep 1
if ! kill -0 "${xvfb_pid}" 2>/dev/null; then
  echo "Xvfb failed to start; see ${xvfb_log}" >&2
  exit 2
fi
ost runtime validate cy2026 --profile usd
python /src/open-strata/support/validate-openusd-vulkan-runtime.py \
  "${runtime_root}" --version "${version}" --platform linux \
  | tee "${output_root}/feature-validation.json"

ost runtime export cy2026 \
  --profile usd \
  --dist "${dist_dir}" \
  --build-metadata "${metadata_file}" \
  --slim \
  --jobs "${jobs}" \
  --json \
  | tee "${output_root}/export.json"

cleanup_xvfb
trap - EXIT
unset DISPLAY
