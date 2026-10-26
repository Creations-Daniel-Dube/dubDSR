use std::fs;
use std::path::Path;
use std::process::Command;

fn main() {
    let manifest = Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).to_path_buf();
    embed_windows_icon(&manifest);
    let kernel_root = manifest.join("kernels").join("src");
    rerun_tree(&kernel_root);

    let out_dir = Path::new(&std::env::var("OUT_DIR").unwrap()).to_path_buf();
    let ptx_path = out_dir.join("kernels.ptx");
    let source = kernel_root.join("lib.rs");
    let emit = format!("asm={}", ptx_path.display());

    let status = Command::new("rustup")
        .args([
            "run",
            "nightly",
            "rustc",
            "--edition",
            "2024",
            "--crate-name",
            "dubdsr_kernels",
            "--crate-type",
            "rlib",
            "--target",
            "nvptx64-nvidia-cuda",
            "-C",
            "panic=abort",
            "-C",
            "opt-level=3",
            "-C",
            "overflow-checks=off",
            "-C",
            "debug-assertions=off",
            "--emit",
            &emit,
        ])
        .arg(&source)
        .status()
        .expect("rustup nightly est requis pour compiler les noyaux GPU");

    if !status.success() {
        panic!("la compilation PTX des noyaux a échoué");
    }

    println!("cargo:rustc-env=DUBDSR_PTX={}", ptx_path.display());
    compile_libraw(&manifest);
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
