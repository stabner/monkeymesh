use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=cuda/meshhash_mix.cu");
    println!("cargo:rustc-check-cfg=cfg(mesh_cuda)");

    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let cu = PathBuf::from("cuda/meshhash_mix.cu");
    let obj = if cfg!(windows) {
        out.join("meshhash_mix.obj")
    } else {
        out.join("meshhash_mix.o")
    };

    let nvcc = find_nvcc();
    let mut cmd = Command::new(&nvcc);
    // Shared CUDA runtime so releases can ship cudart64_*.dll next to the exe.
    cmd.args([
        "-O3",
        "-std=c++17",
        "-cudart",
        "hybrid",
        "-c",
        cu.to_str().unwrap(),
        "-o",
        obj.to_str().unwrap(),
    ]);
    if cfg!(windows) {
        if let Some(cl_dir) = find_msvc_bin() {
            // nvcc needs the MSVC host compiler on Windows.
            cmd.arg("-ccbin").arg(&cl_dir);
            let path = env::var_os("PATH").unwrap_or_default();
            let mut paths = env::split_paths(&path).collect::<Vec<_>>();
            paths.insert(0, cl_dir);
            if let Ok(joined) = env::join_paths(paths) {
                cmd.env("PATH", joined);
            }
        }
        cmd.args(["-Xcompiler", "/MD"]);
    }

    let status = cmd.status();
    match status {
        Ok(s) if s.success() => {
            // Package as a static lib so dependents (e.g. mesh-wallet-gui) also link CUDA symbols.
            let lib_name = "meshhash_cuda_mix";
            let lib_ok = if cfg!(windows) {
                let lib_path = out.join(format!("{lib_name}.lib"));
                let lib_exe = find_msvc_bin()
                    .map(|b| b.join("lib.exe"))
                    .unwrap_or_else(|| PathBuf::from("lib.exe"));
                Command::new(lib_exe)
                    .arg(format!("/OUT:{}", lib_path.display()))
                    .arg(&obj)
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false)
            } else {
                let lib_path = out.join(format!("lib{lib_name}.a"));
                Command::new("ar")
                    .args(["crus", lib_path.to_str().unwrap(), obj.to_str().unwrap()])
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false)
            };

            if lib_ok {
                println!("cargo:rustc-link-search=native={}", out.display());
                println!("cargo:rustc-link-lib=static={lib_name}");
            } else {
                // Fallback for this package's own bins only.
                println!("cargo:rustc-link-arg={}", obj.display());
                println!(
                    "cargo:warning=could not build static CUDA lib — dependents may fail to link"
                );
            }

            if let Ok(cuda) = env::var("CUDA_PATH") {
                let cuda = PathBuf::from(cuda);
                let lib = cuda.join("lib").join("x64");
                if lib.is_dir() {
                    println!("cargo:rustc-link-search=native={}", lib.display());
                }
                let bin_x64 = cuda.join("bin").join("x64");
                if bin_x64.is_dir() {
                    println!("cargo:rustc-link-search=native={}", bin_x64.display());
                }
            }
            println!("cargo:rustc-link-lib=dylib=cudart");
            println!("cargo:rustc-cfg=mesh_cuda");
        }
        Ok(s) => {
            let require = env::var("MESH_REQUIRE_CUDA").ok().as_deref() == Some("1");
            let msg = format!(
                "nvcc failed (exit {:?}) via {} — CPU-only fallback",
                s.code(),
                nvcc.display()
            );
            if require {
                panic!(
                    "{msg}. Install CUDA toolkit / set CUDA_PATH, or unset MESH_REQUIRE_CUDA."
                );
            }
            println!("cargo:warning={msg}");
        }
        Err(e) => {
            let require = env::var("MESH_REQUIRE_CUDA").ok().as_deref() == Some("1");
            let msg = format!(
                "nvcc not runnable ({e}) via {} — CPU-only fallback",
                nvcc.display()
            );
            if require {
                panic!(
                    "{msg}. Install CUDA toolkit / set CUDA_PATH, or unset MESH_REQUIRE_CUDA."
                );
            }
            println!("cargo:warning={msg}");
        }
    }
}

fn find_nvcc() -> PathBuf {
    if let Ok(p) = which_cmd("nvcc") {
        return p;
    }
    if let Ok(cuda) = env::var("CUDA_PATH") {
        let candidate = PathBuf::from(cuda).join("bin").join(if cfg!(windows) {
            "nvcc.exe"
        } else {
            "nvcc"
        });
        if candidate.exists() {
            return candidate;
        }
    }
    PathBuf::from("nvcc")
}

fn find_msvc_bin() -> Option<PathBuf> {
    let vswhere = PathBuf::from(env::var("ProgramFiles(x86)").unwrap_or_else(|_| {
        r"C:\Program Files (x86)".into()
    }))
    .join(r"Microsoft Visual Studio\Installer\vswhere.exe");
    if !vswhere.exists() {
        return None;
    }
    let out = Command::new(vswhere)
        .args([
            "-latest",
            "-products",
            "*",
            "-requires",
            "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
            "-property",
            "installationPath",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let root = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if root.is_empty() {
        return None;
    }
    let msvc = PathBuf::from(&root).join(r"VC\Tools\MSVC");
    let mut versions = std::fs::read_dir(&msvc).ok()?.filter_map(|e| e.ok()).collect::<Vec<_>>();
    versions.sort_by_key(|e| e.file_name());
    let latest = versions.last()?;
    let bin = latest.path().join(r"bin\Hostx64\x64");
    if bin.join("cl.exe").exists() {
        Some(bin)
    } else {
        None
    }
}

fn which_cmd(name: &str) -> Result<PathBuf, ()> {
    let output = Command::new(if cfg!(windows) { "where" } else { "which" })
        .arg(name)
        .output()
        .map_err(|_| ())?;
    if !output.status.success() {
        return Err(());
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let first = text.lines().next().unwrap_or("").trim();
    if first.is_empty() {
        return Err(());
    }
    Ok(PathBuf::from(first))
}
