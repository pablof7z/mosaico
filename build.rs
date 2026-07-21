use std::env;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::process::Command;

const CROISSANT_REV: &str = "c55533d0e7fd416e1060dd4d3db009ae2a687719";

fn main() {
    println!("cargo:rerun-if-changed=vendor/croissant");
    println!("cargo:rerun-if-env-changed=MOSAICO_CROISSANT_BIN");
    println!("cargo:rustc-env=MOSAICO_CROISSANT_REV={CROISSANT_REV}");

    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let executable = out.join("croissant");

    if let Some(prebuilt) = env::var_os("MOSAICO_CROISSANT_BIN") {
        copy_prebuilt(Path::new(&prebuilt), &executable);
    } else {
        build_pinned_source(&manifest, &executable);
    }

    compress(&executable, &out.join("croissant.zst"));
    fs::remove_file(&executable).expect("remove uncompressed Croissant build artifact");
}

fn copy_prebuilt(source: &Path, destination: &Path) {
    fs::copy(source, destination).unwrap_or_else(|error| {
        panic!(
            "copying MOSAICO_CROISSANT_BIN {} failed: {error}",
            source.display()
        )
    });
}

fn build_pinned_source(manifest: &Path, executable: &Path) {
    let source = manifest.join("vendor/croissant");
    if !source.join("main.go").is_file() {
        panic!("Croissant source is missing; run `git submodule update --init vendor/croissant`");
    }
    if env::var_os("HOST") != env::var_os("TARGET") {
        panic!(
            "cross-compiling the bundled CGO relay requires a target-native executable in \
             MOSAICO_CROISSANT_BIN"
        );
    }
    verify_revision(&source);

    let status = Command::new("go")
        .current_dir(&source)
        .env("CGO_ENABLED", "1")
        .args(["build", "-mod=vendor", "-trimpath"])
        .arg("-ldflags")
        .arg(format!(
            "-s -w -X main.currentVersion={}",
            &CROISSANT_REV[..12]
        ))
        .arg("-o")
        .arg(executable)
        .arg(".")
        .status()
        .unwrap_or_else(|error| panic!("starting Go to build bundled Croissant failed: {error}"));
    assert!(status.success(), "building bundled Croissant failed");
}

fn verify_revision(source: &Path) {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(source)
        .output()
        .unwrap_or_else(|error| panic!("reading Croissant revision failed: {error}"));
    if output.status.success() {
        let actual = String::from_utf8_lossy(&output.stdout);
        assert_eq!(
            actual.trim(),
            CROISSANT_REV,
            "vendor/croissant must remain pinned to the reviewed fork revision"
        );
    }
}

fn compress(executable: &Path, archive: &Path) {
    let source = BufReader::new(File::open(executable).expect("open built Croissant executable"));
    let destination = BufWriter::new(File::create(archive).expect("create Croissant archive"));
    zstd::stream::copy_encode(source, destination, 19).expect("compress Croissant executable");
}
