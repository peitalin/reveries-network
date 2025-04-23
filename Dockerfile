FROM ubuntu:22.04 AS base
# bypass prompts from apt
ENV DEBIAN_FRONTEND=noninteractive
# Update and install build dependencies
RUN apt-get update && \
    apt-get install -y \
    pkg-config \
    libssl-dev \
    libtss2-dev \
    build-essential \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Install Rust using rustup
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
# Add Cargo's bin directory to PATH
ENV PATH="/root/.cargo/bin:${PATH}"
# install Just for JustFile
RUN cargo install just

############## BEGIN ##############

FROM base AS build
# Verify installation
RUN rustc --version && cargo --version

# Create a working directory
WORKDIR /app

# Copy src into the container
COPY ./agents  ./agents
COPY ./cmd  ./cmd
COPY ./llm-proxy  ./llm-proxy
COPY ./node ./node
COPY ./telemetry ./telemetry
COPY ./tests ./tests

COPY ./Cargo.toml ./Cargo.toml
COPY ./Cargo.lock ./Cargo.lock

COPY ./JustFile ./JustFile
COPY ./README.md ./README.md

# Build the project
RUN cargo build
# Run the following if building on a TDX enabled Linux VM
# RUN cargo build --features "tdx"

# Default command when container starts
CMD ["ls"]