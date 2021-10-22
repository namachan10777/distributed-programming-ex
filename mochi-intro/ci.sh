#!/bin/sh

set -eux

eval `spack load --sh mochi-margo`
meson build
ninja -C ./build
ninja -C ./build test