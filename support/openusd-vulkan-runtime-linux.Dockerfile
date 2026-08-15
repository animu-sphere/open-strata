# syntax=docker/dockerfile:1
# The published Linux runtime intentionally matches GitHub's ubuntu-24.04
# consumer lane. ost measures and records the actual glibc floor at export.
FROM ubuntu:24.04

ARG DEBIAN_FRONTEND=noninteractive
ARG VULKAN_HEADERS_VERSION=v1.4.350
ARG VULKAN_UTILITY_LIBRARIES_VERSION=v1.4.350
ARG VMA_VERSION=v3.4.0
ARG PYTHON_VERSION=3.13.15
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
      build-essential gcc-14 g++-14 ca-certificates cmake curl git ninja-build pkg-config \
      software-properties-common unzip \
      libgl1-mesa-dev libglu1-mesa-dev libshaderc-dev libvulkan-dev \
      mesa-vulkan-drivers \
      libx11-dev libxcursor-dev libxext-dev libxi-dev libxinerama-dev \
      libxrandr-dev libxt-dev libxkbcommon-x11-0 \
    && add-apt-repository -y ppa:deadsnakes/ppa \
    && apt-get update \
    && apt-get install -y --no-install-recommends \
      python3.13 python3.13-dev python3.13-venv \
    && ln -s libshaderc.so \
      /usr/lib/x86_64-linux-gnu/libshaderc_combined.so \
    && rm -rf /var/lib/apt/lists/*

RUN git clone --branch "${VULKAN_HEADERS_VERSION}" --depth 1 \
      https://github.com/KhronosGroup/Vulkan-Headers.git /tmp/Vulkan-Headers \
    && cmake -S /tmp/Vulkan-Headers -B /tmp/Vulkan-Headers/build \
      -DCMAKE_INSTALL_PREFIX=/opt/vulkan-sdk \
    && cmake --install /tmp/Vulkan-Headers/build \
    && git clone --branch "${VULKAN_UTILITY_LIBRARIES_VERSION}" --depth 1 \
      https://github.com/KhronosGroup/Vulkan-Utility-Libraries.git /tmp/Vulkan-Utility-Libraries \
    && install -m 0644 \
      /tmp/Vulkan-Utility-Libraries/include/vulkan/vk_enum_string_helper.h \
      /opt/vulkan-sdk/include/vulkan/vk_enum_string_helper.h \
    && git clone --branch "${VMA_VERSION}" --depth 1 \
      https://github.com/GPUOpen-LibrariesAndSDKs/VulkanMemoryAllocator.git /tmp/VMA \
    && install -d /opt/vulkan-sdk/include/vma \
    && install -m 0644 /tmp/VMA/include/vk_mem_alloc.h \
      /opt/vulkan-sdk/include/vma/vk_mem_alloc.h \
    && rm -rf /tmp/Vulkan-Headers /tmp/Vulkan-Utility-Libraries /tmp/VMA

ENV VIRTUAL_ENV=/opt/py313
ENV PATH=/opt/py313/bin:/root/.cargo/bin:${PATH}
ENV VULKAN_SDK=/opt/vulkan-sdk
RUN python3.13 -m venv /opt/py313 \
    && python -m pip install --no-cache-dir --upgrade pip \
    && python -m pip install --no-cache-dir Jinja2 PyOpenGL PySide6 \
    && test "$(python -c 'import sys; print(".".join(map(str, sys.version_info[:3])))')" \
      = "${PYTHON_VERSION}"

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
      | sh -s -- -y --profile minimal --default-toolchain 1.96.0

WORKDIR /src/open-strata
COPY . .
RUN cargo build --locked --release -p ost-cli \
    && install -m 0755 target/release/ost /usr/local/bin/ost

ENV OST_HOME=/work/.ost
WORKDIR /work
CMD ["bash"]
