fn main() {
    println!("cargo:rerun-if-changed=witness.c");
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
    cc::Build::new().file("witness.c").opt_level(2).compile("m3_witness");
}
