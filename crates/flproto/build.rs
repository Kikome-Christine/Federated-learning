fn main() {
    let protoc = protoc_bin_vendored::protoc_bin_path().expect("vendored protoc not found");
    std::env::set_var("PROTOC", protoc);
    tonic_build::configure()
        .compile_protos(&["proto/fl.proto"], &["proto"])
        .expect("failed to compile proto/fl.proto");
    println!("cargo:rerun-if-changed=proto/fl.proto");
}
