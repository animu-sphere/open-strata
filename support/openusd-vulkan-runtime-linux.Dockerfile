# syntax=docker/dockerfile:1
# The published Linux runtime intentionally matches GitHub's ubuntu-24.04
# consumer lane. ost measures and records the actual glibc floor at export.
FROM ubuntu:24.04

ARG DEBIAN_FRONTEND=noninteractive
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
      build-essential ca-certificates cmake curl git ninja-build pkg-config \
      software-properties-common unzip \
      libgl1-mesa-dev libglu1-mesa-dev libvulkan-dev \
      libx11-dev libxcursor-dev libxext-dev libxi-dev libxinerama-dev \
      libxrandr-dev libxt-dev libxkbcommon-x11-0 \
    && add-apt-repository -y ppa:deadsnakes/ppa \
    && apt-get update \
    && apt-get install -y --no-install-recommends \
      python3.13 python3.13-dev python3.13-venv \
    && rm -rf /var/lib/apt/lists/*

ENV VIRTUAL_ENV=/opt/py313
ENV PATH=/opt/py313/bin:/root/.cargo/bin:${PATH}
ENV VULKAN_SDK=/usr
RUN python3.13 -m venv /opt/py313 \
    && python -m pip install --no-cache-dir --upgrade pip \
    && python -m pip install --no-cache-dir Jinja2 PyOpenGL PySide6

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
      | sh -s -- -y --profile minimal --default-toolchain 1.96.0

WORKDIR /src/open-strata
COPY . .
RUN cargo build --locked --release -p ost-cli \
    && install -m 0755 target/release/ost /usr/local/bin/ost

ENV OST_HOME=/work/.ost
WORKDIR /work
CMD ["bash"]
