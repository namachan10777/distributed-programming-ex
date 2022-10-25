VERSION 0.6


mochi-base:
    FROM ubuntu:22.04

    ARG USERNAME=ex
    ARG UID=1000
    ARG GID=1000

    RUN apt-get update && \
        apt-get -y upgrade  && \
        apt-get install -y \
            gcc autoconf automake cmake libtool pkgconf \
            git python3 bison fuse libfuse-dev libssl-dev curl meson

    RUN groupadd -g $GID $USERNAME && \
        useradd -m -s /bin/bash -u $UID -g $GID $USERNAME

    WORKDIR /home/$USERNAME

    USER $USERNAME
    RUN git clone --depth=1 https://github.com/spack/spack.git /home/$USERNAME/spack
    RUN . $HOME/spack/share/spack/setup-env.sh && \
        spack external find automake autoconf libtool cmake m4 pkgconf bison && \
        spack install mochi-margo ^mercury~boostsys ^libfabric fabrics=rxm,sockets,tcp,udp && \
        spack install mochi-thallium  && \
        printf '%s\n' \
            '. $HOME/spack/share/spack/setup-env.sh' \
            'export PATH=$HOME/workspace/bin:$PATH' \
            'export LD_LIBRARY_PATH=$HOME/workspace/lib:$LD_LIBRARY_PATH' \
            >> .bashrc

intro:
    FROM +mochi-base
    WORKDIR $HOME/build
    COPY intro .
    RUN . $HOME/spack/share/spack/setup-env.sh && \
        spack load mochi-margo && \
        meson setup build
    RUN meson compile -C build

rust-base:
    FROM rust:latest
    RUN cargo install cargo-chef
    RUN curl -fsSL https://github.com/protocolbuffers/protobuf/releases/download/v21.8/protoc-21.8-linux-$(uname -m | sed -e 's/aarch64/aarch_64/').zip -o protoc.zip && \
        unzip protoc.zip -d /usr/local/

ring-rs-recipe:
    FROM +rust-base
    WORKDIR /work
    COPY ring-rs .
    RUN cargo chef prepare --recipe-path recipe.json
    SAVE ARTIFACT recipe.json /recipe.json

ring-rs-bin:
    FROM +rust-base
    WORKDIR /work
    COPY +ring-rs-recipe/recipe.json recipe.json
    RUN cargo chef cook --release --recipe-path recipe.json
    COPY ring-rs /work
    RUN cargo build --release
    SAVE ARTIFACT target/release/ring-rs /bin

ring-rs:
    FROM debian:buster-slim
    COPY +ring-rs-bin/bin /usr/local/bin/ring-rs
    ENTRYPOINT [ "/usr/local/bin/ring-rs" ]
	SAVE IMAGE ring-rs:latest
