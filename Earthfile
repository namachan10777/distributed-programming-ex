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

    USER $USERNAME
    RUN cd && \
        git clone -c feature.manyFiles=true --depth 1 https://github.com/spack/spack.git  && \
        . spack/share/spack/setup-env.sh && \
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
    COPY mochi-intro .
    RUN ls
    RUN meson setup build
    RUN meson compile -C build
    COPY build/server /usr/local/bin/server
    COPY build/client /usr/local/bin/client
