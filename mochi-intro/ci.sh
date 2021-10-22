#!/bin/sh

set -eux

cd `dirname $0`

clang-format --Werror --dry-run *.c

eval `spack load --sh mochi-margo`
meson build
ninja -C ./build
ninja -C ./build test
