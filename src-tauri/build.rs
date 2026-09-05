fn main() {
    println!("cargo:rerun-if-changed=.pake/pake.json");
    println!("cargo:rerun-if-changed=.pake/tauri.conf.json");
    println!("cargo:rerun-if-changed=../dist/gbf-sidebar.html");
    println!("cargo:rerun-if-changed=../dist/gbf-options.html");
    tauri_build::build()
}
