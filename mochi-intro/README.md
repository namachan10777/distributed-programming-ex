# Mochi-intro

* **required**
  * [bear](https://github.com/rizsotto/Bear)
  * [spack](https://spack.io/)
  * `mochi-margo` by spack

## Generete `compile_commands.json`

```
spack load mochi-margo
meson build
bear -- ninja -C ./build
```

## Build

```
spack load mochi-margo
meson build
ninja -C ./build
```