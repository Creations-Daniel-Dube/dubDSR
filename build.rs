// build.rs

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Dernier nightly d'avant le relèvement du plancher NVPTX à `sm_70` (Rust 1.97, 29 avril 2026).
const PASCAL_TOOLCHAIN: &str = "nightly-2026-04-28";

fn main() {
    let manifest = Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).to_path_buf();
    embed_windows_icon(&manifest);
    let kernel_root = manifest.join("kernels").join("src");
    rerun_tree(&kernel_root);

    let out_dir = Path::new(&std::env::var("OUT_DIR").unwrap()).to_path_buf();
    let source = kernel_root.join("lib.rs");
    let modern = compile_ptx(&out_dir.join("kernels.ptx"), &source, "nightly", "sm_70");
    let pascal = compile_ptx(&out_dir.join("kernels-sm61.ptx"), &source, PASCAL_TOOLCHAIN, "sm_61");

    println!("cargo:rustc-env=DUBDSR_PTX={}", modern.display());
    println!("cargo:rustc-env=DUBDSR_PTX_STAMP={}", stamp_of(&modern));
    println!("cargo:rustc-env=DUBDSR_PTX_SM61={}", pascal.display());
    println!("cargo:rustc-env=DUBDSR_PTX_SM61_STAMP={}", stamp_of(&pascal));
    compile_libraw(&manifest);
}

fn compile_ptx(dest: &Path, source: &Path, toolchain: &str, cpu: &str) -> PathBuf {
    let emit = format!("asm={}", dest.display());
    let cpu_flag = format!("target-cpu={cpu}");
    let scratch = dest.parent().unwrap_or(dest).join(format!("ptx-{cpu}"));
    let _ = fs::create_dir_all(&scratch);
    let output = Command::new("rustup")
        .args([
            "run",
            toolchain,
            "rustc",
            "--edition",
            "2024",
            "--crate-name",
            "dubdsr_kernels",
            "--crate-type",
            "rlib",
            "--target",
            "nvptx64-nvidia-cuda",
            "--out-dir",
        ])
        .arg(&scratch)
        .args([
            "-C",
            "panic=abort",
            "-C",
            "opt-level=3",
            "-C",
            "overflow-checks=off",
            "-C",
            "debug-assertions=off",
            "-C",
            &cpu_flag,
            "--emit",
            &emit,
        ])
        .arg(source)
        .output()
        .unwrap_or_else(|err| panic!("impossible de lancer rustup {toolchain} : {err}"));

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!("la compilation PTX {cpu} ({toolchain}) a échoué.\n{stderr}");
    }

    let ptx = fs::read_to_string(dest).unwrap_or_else(|err| panic!("lecture du PTX {cpu} : {err}"));
    let marker = format!(".target {cpu}");
    if !ptx.lines().any(|line| line.trim() == marker) {
        panic!("le PTX {cpu} ne contient pas `{marker}`");
    }
    let ptx = stub_extern_funcs(&ptx);
    if ptx.contains(".extern .func") {
        panic!("le PTX {cpu} contient encore `.extern .func`");
    }
    fs::write(dest, &ptx).unwrap_or_else(|err| panic!("écriture du PTX {cpu} : {err}"));
    dest.to_path_buf()
}

fn stamp_of(path: &Path) -> u64 {
    fs::read(path)
        .unwrap_or_default()
        .into_iter()
        .fold(0u64, |acc, byte| acc.wrapping_mul(16777619).wrapping_add(byte as u64))
}

/// Le pilote refuse un module qui déclare `panic_bounds_check` sans la définir.
/// Le PTX Pascal (ISA 5.0) termine la déclaration par `;`, le PTX récent par `.noreturn;`.
fn stub_extern_funcs(ptx: &str) -> String {
    let mut out = String::with_capacity(ptx.len() + 256);
    let mut lines = ptx.lines();
    while let Some(line) = lines.next() {
        if line.trim_start().starts_with(".extern .func") {
            out.push_str(&line.replacen(".extern .func", ".visible .func", 1));
            out.push('\n');
            let mut closed = false;
            for next in lines.by_ref() {
                match next.trim() {
                    ".noreturn;" => {
                        out.push_str(".noreturn\n{\n\ttrap;\n\texit;\n}\n");
                        closed = true;
                        break;
                    }
                    ";" => {
                        out.push_str("{\n\ttrap;\n\texit;\n}\n");
                        closed = true;
                        break;
                    }
                    ");" => {
                        out.push_str(")\n{\n\ttrap;\n\texit;\n}\n");
                        closed = true;
                        break;
                    }
                    _ => {
                        out.push_str(next);
                        out.push('\n');
                    }
                }
            }
            if !closed {
                panic!("PTX : fonction externe sans terminateur");
            }
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

fn compile_libraw(manifest: &Path) {
    let root = manifest.join("third_party").join("LibRaw");
    let mosaic = manifest.join("native").join("mosaic.cpp");
    println!("cargo:rerun-if-changed={}", mosaic.display());
    println!("cargo:rerun-if-changed={}", root.join("src").display());
    println!("cargo:rerun-if-changed={}", root.join("libraw").display());
    println!("cargo:rerun-if-changed={}", root.join("internal").display());

    let target = std::env::var("TARGET").unwrap_or_default();
    let mut build = cc::Build::new();
    build
        .cpp(true)
        .include(&root)
        .define("LIBRAW_NODLL", None)
        .define("NO_LCMS", None)
        .warnings(false)
        .extra_warnings(false)
        .opt_level(2)
        .file(&mosaic);
    if target.contains("windows") {
        build.define("NOMINMAX", None);
        build.define("LIBRAW_WIN32_UNICODEPATHS", None);
    }
    if target.contains("windows-msvc") {
        build.static_crt(true);
        build.flag_if_supported("/EHsc");
    } else {
        build.flag_if_supported("-std=gnu++14");
        build.flag_if_supported("-fno-strict-aliasing");
        if target.contains("windows-gnu") {
            // Par défaut cc lie libstdc++.dll.a. L'autre PC n'a pas MinGW.
            build.cpp_link_stdlib_static(true);
        }
    }
    for file in libraw_cpp_files(&root.join("src")) {
        build.file(file);
    }
    build.compile("libraw_vendor");

    if target.contains("windows") {
        println!("cargo:rustc-link-lib=ws2_32");
    }
}

fn libraw_cpp_files(src: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    collect_cpp(src, &mut files);
    files
}

fn collect_cpp(dir: &Path, files: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_cpp(&path, files);
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("cpp") {
            continue;
        }
        let name = path.file_name().and_then(|value| value.to_str()).unwrap_or("");
        if name.ends_with("_ph.cpp") {
            continue;
        }
        files.push(path);
    }
}

fn embed_windows_icon(manifest: &Path) {
    let target = std::env::var("TARGET").unwrap_or_default();
    if !target.contains("windows") {
        return;
    }
    let icon = manifest.join("images").join("dubdsr.ico");
    println!("cargo:rerun-if-changed={}", icon.display());
    if !icon.is_file() {
        panic!("icône introuvable : {}", icon.display());
    }

    let out_dir = Path::new(&std::env::var("OUT_DIR").unwrap()).to_path_buf();
    let rc_path = out_dir.join("dubdsr_icon.rc");
    let icon_rc = icon.to_string_lossy().replace('\\', "/");
    fs::write(&rc_path, format!("1 ICON \"{icon_rc}\"\n")).expect("écriture du script de ressource");

    if target.contains("windows-gnu") {
        let object = out_dir.join("dubdsr_icon.o");
        let status = Command::new("windres")
            .args(["-O", "coff", "-i"])
            .arg(&rc_path)
            .arg("-o")
            .arg(&object)
            .status()
            .expect("windres est requis pour l'icône de l'exécutable Windows");
        if !status.success() {
            panic!("windres n'a pas pu intégrer images/dubdsr.ico");
        }
        println!("cargo:rustc-link-arg-bins={}", object.display());
    } else {
        let resource = out_dir.join("dubdsr_icon.res");
        let status = Command::new("llvm-rc")
            .arg(format!("/fo{}", resource.display()))
            .arg(&rc_path)
            .status()
            .expect("llvm-rc est requis pour l'icône de l'exécutable Windows");
        if !status.success() {
            panic!("llvm-rc n'a pas pu intégrer images/dubdsr.ico");
        }
        println!("cargo:rustc-link-arg-bins={}", resource.display());
    }
}

fn rerun_tree(dir: &Path) {
    println!("cargo:rerun-if-changed={}", dir.display());
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rerun_tree(&path);
        } else {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
}
