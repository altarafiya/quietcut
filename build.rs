use std::{env, path::PathBuf, process::Command};
fn main() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    println!("cargo:rerun-if-changed=assets/quietcut.rc");
    println!("cargo:rerun-if-changed=assets/quietcut.ico");
    println!("cargo:rerun-if-changed=assets/quietcut.manifest");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("quietcut.res");
    let sdk = PathBuf::from("C:/Program Files (x86)/Windows Kits/10/bin");
    let rc = env::var_os("QUIETCUT_RC").map(PathBuf::from).or_else(|| {
        let mut versions = std::fs::read_dir(sdk)
            .ok()?
            .flatten()
            .map(|e| e.path().join("x64/rc.exe"))
            .filter(|p| p.is_file())
            .collect::<Vec<_>>();
        versions.sort();
        versions.pop()
    });
    if let Some(rc) = rc {
        let result = Command::new(rc)
            .arg("/nologo")
            .arg(format!("/fo{}", out.display()))
            .arg("assets/quietcut.rc")
            .status()
            .expect("Run resource compiler");
        assert!(result.success(), "Resource compilation failed");
        println!("cargo:rustc-link-arg-bins={}", out.display());
    }
}
