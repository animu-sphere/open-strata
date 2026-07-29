#!/usr/bin/env bash
set -euo pipefail

version="${1:?OpenUSD version is required (26.05 or 26.08)}"
jobs="${2:?parallel job count is required}"
output_name="${3:?output directory name is required}"
openstrata_revision="${4:?OpenStrata revision is required}"

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
      "vulkan_sdk": "ubuntu-24.04-libvulkan-dev"
    }
  }
}
EOF

build_args=(
  --build-arg --vulkan
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

ost runtime validate cy2026 --profile usd
python /src/open-strata/support/validate-openusd-vulkan-runtime.py \
  "${runtime_root}" --version "${version}" --platform linux \
  | tee "${output_root}/feature-validation.json"

ost runtime export cy2026 \
  --profile usd \
  --dist "${dist_dir}" \
  --build-metadata "${metadata_file}" \
  --jobs "${jobs}" \
  --json \
  | tee "${output_root}/export.json"
