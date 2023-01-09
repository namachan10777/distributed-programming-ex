fn main() {
    tonic_build::compile_protos("./protos/fs.proto").unwrap();
}
