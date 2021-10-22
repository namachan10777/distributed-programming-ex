#!/bin/sh

set -eux

cd `dirname $0`

eval `spack load --sh mochi-margo`
meson build
ninja -C ./build
ninja -C ./build test