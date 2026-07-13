# syntax=docker/dockerfile:1
# Reference builder for a Linux x86_64 runtime with a glibc 2.28 ceiling.
FROM quay.io/pypa/manylinux_2_28_x86_64

RUN dnf install -y \
      cmake git ninja-build unzip which \
      libX11-devel libXext-devel libXi-devel libXrandr-devel libXt-devel \
      mesa-libGL-devel mesa-libGLU-devel \
    && dnf clean all

# manylinux publishes shared-library CPython builds at stable ABI paths.
ENV VIRTUAL_ENV=/opt/ost-venv
ENV PATH=/opt/ost-venv/bin:/opt/python/cp313-cp313/bin:/root/.cargo/bin:${PATH}
ENV LD_LIBRARY_PATH=/opt/python/cp313-cp313/lib
RUN /opt/python/cp313-cp313/bin/python -m venv /opt/ost-venv \
    && python -m pip install --no-cache-dir Jinja2 PyOpenGL PySide6

# Build ost inside the same glibc floor instead of copying a host-built binary.
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
      | sh -s -- -y --profile minimal --default-toolchain 1.96.0
WORKDIR /src/open-strata
COPY . .
RUN cargo build --locked --release -p ost-cli \
    && install -m 0755 target/release/ost /usr/local/bin/ost

ENV OST_HOME=/work/.ost
WORKDIR /work
CMD ["bash"]
