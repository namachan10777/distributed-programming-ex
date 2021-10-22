FROM ubuntu:20.04

ARG UID=1000
ARG GID=1000

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y \
    git python3 gcc g++ \
    cmake=3.16.3-1ubuntu1 \
    pkgconf=1.6.3-5 \
    autoconf=2.69-11.1 \
    libtool=2.4.6-14 \
    m4=1.4.18-4

COPY ./packages.yaml /home/spack/.spack/packages.yaml

RUN groupadd -g ${GID} spack && \
    useradd -m -s /bin/bash -u ${UID} -g ${GID} spack
RUN chown -R spack:spack /home/spack
USER spack

RUN git clone --depth=1 https://github.com/spack/spack.git /home/spack/spack
ENV PATH $PATH:/home/spack/spack/bin

RUN spack install mochi-margo

ENTRYPOINT [ "/bin/bash" ]