use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=app.rc");
    println!("cargo:rerun-if-changed=assets/bks-term.ico");
    println!("cargo:rerun-if-env-changed=RC");

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"));
    let resource = out_dir.join("bks-term.res");
    let resource_arg = format!("/fo{}", resource.display());
    let configured = env::var_os("RC").map(PathBuf::from);
    let compilers = configured
        .into_iter()
        .chain([PathBuf::from("rc.exe")])
        .chain(sdk_resource_compilers());

    for compiler in compilers {
        let Ok(status) = Command::new(&compiler)
            .arg("/nologo")
            .arg(&resource_arg)
            .arg("app.rc")
            .status()
        else {
            continue;
        };
        if status.success() {
            println!("cargo:rustc-link-arg-bins={}", resource.display());
            return;
        }
    }
    panic!("Windows resource compiler (rc.exe) was not found or failed");
}

fn sdk_resource_compilers() -> Vec<PathBuf> {
    let root = PathBuf::from(r"C:\Program Files (x86)\Windows Kits\10\bin");
    let Ok(entries) = root.read_dir() else {
        return Vec::new();
    };
    let mut compilers: Vec<_> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path().join("x64").join("rc.exe"))
        .filter(|path| path.is_file())
        .collect();
    compilers.sort();
    compilers.reverse();
    compilers
}
