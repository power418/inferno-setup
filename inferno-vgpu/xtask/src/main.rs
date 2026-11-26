//! Workspace build, staging (`dist`), header verification, testkit guard, and `check` tasks.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
#![warn(missing_docs)]

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn run(cmd: &mut Command) -> Result<(), String> {
    let status = cmd
        .status()
        .map_err(|e| format!("failed to spawn {:?}: {e}", cmd.get_program()))?;
    if !status.success() {
        return Err(format!(
            "command {:?} failed with status {}",
            cmd.get_program(),
            status
        ));
    }
    Ok(())
}

fn check_testkit_gate(root: &Path) -> Result<(), String> {
    let crates_dir = root.join("crates");
    let entries = fs::read_dir(&crates_dir).map_err(|e| format!("read crates/: {e}"))?;
    for entry in entries.flatten() {
        let toml_path = entry.path().join("Cargo.toml");
        if !toml_path.is_file() {
            continue;
        }
        let content = fs::read_to_string(&toml_path)
            .map_err(|e| format!("read {}: {e}", toml_path.display()))?;
        let mut in_deps = false;
        for (line_no, raw) in content.lines().enumerate() {
            let line = raw.trim();
            if line.starts_with('[') {
                in_deps = line == "[dependencies]" || line.starts_with("[dependencies.");
                continue;
            }
            if in_deps && line.contains("testkit") && !line.starts_with('#') {
                return Err(format!(
                    "{}:{}: non-dev [dependencies] must not enable `testkit` (D50)",
                    toml_path.display(),
                    line_no + 1
                ));
            }
        }
    }
    println!("[xtask] testkit gate: OK");
    Ok(())
}

fn check_headers(root: &Path) -> Result<(), String> {
    let wire_h = root.join("crates/inferno-vgpu-graphics/include/inferno_graphics_wire.h");
    let api_h = root.join("crates/inferno-vgpu-c/include/inferno_vgpu.h");
    for h in [&wire_h, &api_h] {
        if !h.is_file() {
            return Err(format!("missing header: {}", h.display()));
        }
        // Syntax-check C header with clang when available
        if Command::new("clang").arg("--version").output().is_ok() {
            run(Command::new("clang")
                .args(["-x", "c-header", "-std=c11", "-fsyntax-only", "-Wall", "-Wextra", "-Werror"])
                .arg(h))?;
        }
    }
    println!("[xtask] headers syntax check: OK");
    Ok(())
}

fn dist(root: &Path) -> Result<(), String> {
    run(Command::new("cargo")
        .current_dir(root)
        .args(["build", "--release", "-p", "inferno-vgpu-c", "-p", "inferno-vgpu-server"]))?;

    let target_release = root.join("target/release");
    let dist_dir = root.join("target/dist-vulkan");
    let inc_dir = dist_dir.join("include");
    let lib_dir = dist_dir.join("lib");
    let pc_dir = lib_dir.join("pkgconfig");
    let bin_dir = dist_dir.join("bin");

    for d in [&inc_dir, &lib_dir, &pc_dir, &bin_dir] {
        fs::create_dir_all(d).map_err(|e| format!("mkdir {}: {e}", d.display()))?;
    }

    fs::copy(
        root.join("crates/inferno-vgpu-c/include/inferno_vgpu.h"),
        inc_dir.join("inferno_vgpu.h"),
    )
    .map_err(|e| format!("copy inferno_vgpu.h: {e}"))?;
    fs::copy(
        root.join("crates/inferno-vgpu-graphics/include/inferno_graphics_wire.h"),
        inc_dir.join("inferno_graphics_wire.h"),
    )
    .map_err(|e| format!("copy inferno_graphics_wire.h: {e}"))?;

    for name in ["libinferno_vgpu_c.a", "libinferno_vgpu_c.so"] {
        let src = target_release.join(name);
        if src.is_file() {
            let dst_name = name.replace("libinferno_vgpu_c", "libinferno_vgpu");
            fs::copy(&src, lib_dir.join(&dst_name))
                .map_err(|e| format!("copy {name}: {e}"))?;
        }
    }

    let srv_src = target_release.join("inferno-vgpu-server");
    if srv_src.is_file() {
        fs::copy(&srv_src, bin_dir.join("inferno-vgpu-server"))
            .map_err(|e| format!("copy inferno-vgpu-server: {e}"))?;
    }

    let pc_content = format!(
        "prefix={prefix}\n\
         exec_prefix=${{prefix}}\n\
         libdir=${{prefix}}/lib\n\
         includedir=${{prefix}}/include\n\
         \n\
         Name: inferno-vgpu\n\
         Description: Metal-to-Vulkan virtual GPU backend for ChefKiss Inferno\n\
         Version: 0.1.0\n\
         Cflags: -I${{includedir}}\n\
         Libs: -L${{libdir}} -linferno_vgpu\n\
         Libs.private: -lpthread -ldl -lm\n",
        prefix = dist_dir.display()
    );
    fs::write(pc_dir.join("inferno-vgpu.pc"), pc_content)
        .map_err(|e| format!("write pkg-config: {e}"))?;

    println!("[xtask] dist staged at {}", dist_dir.display());
    Ok(())
}

fn check(root: &Path) -> Result<(), String> {
    check_testkit_gate(root)?;
    check_headers(root)?;
    run(Command::new("cargo")
        .current_dir(root)
        .args(["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"]))?;
    run(Command::new("cargo")
        .current_dir(root)
        .args(["test", "--workspace"]))?;
    println!("[xtask] check: ALL PASSED");
    Ok(())
}

fn main() {
    let root = repo_root();
    let Some(cmd) = env::args().nth(1) else {
        println!("Usage: cargo xtask <check|dist|header|testkit>");
        return;
    };

    let res = match cmd.as_str() {
        "testkit" => check_testkit_gate(&root),
        "header" => check_headers(&root),
        "dist" => dist(&root),
        "check" => check(&root),
        other => Err(format!("unknown xtask command: {other}")),
    };

    if let Err(e) = res {
        eprintln!("[xtask] ERROR: {e}");
        std::process::exit(1);
    }
}
