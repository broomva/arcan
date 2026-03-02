use std::path::Path;
use std::process::Command;

fn main() {
    // Only build the frontend when the embed feature is active AND the
    // frontend directory actually exists (i.e., not on CI without node_modules).
    if cfg!(feature = "embed") {
        let frontend_dir = Path::new("frontend");

        if !frontend_dir.join("package.json").exists() {
            println!(
                "cargo:warning=Console frontend not found — skipping build. The console will serve a placeholder."
            );
            let dist = frontend_dir.join("dist");
            std::fs::create_dir_all(&dist).ok();
            if !dist.join("index.html").exists() {
                std::fs::write(
                    dist.join("index.html"),
                    "<html><body><h1>Arcan Console</h1><p>Frontend not built. Run <code>bun install && bun run build</code> in <code>crates/arcan-console/frontend/</code>.</p></body></html>",
                ).ok();
            }
            return;
        }

        // Prefer bun, fall back to npm.
        let pkg_mgr = if which_exists("bun") { "bun" } else { "npm" };

        // Check if node_modules exists; if not, install deps.
        if !frontend_dir.join("node_modules").exists() {
            let status = Command::new(pkg_mgr)
                .arg("install")
                .current_dir(frontend_dir)
                .status();
            match status {
                Ok(s) if s.success() => {}
                _ => {
                    println!(
                        "cargo:warning={pkg_mgr} install failed — console will use placeholder"
                    );
                    ensure_placeholder_dist(frontend_dir);
                    return;
                }
            }
        }

        // Run the production build.
        let status = Command::new(pkg_mgr)
            .args(["run", "build"])
            .current_dir(frontend_dir)
            .status();
        match status {
            Ok(s) if s.success() => {
                println!("cargo:rerun-if-changed=frontend/src");
                println!("cargo:rerun-if-changed=frontend/index.html");
                println!("cargo:rerun-if-changed=frontend/package.json");
                println!("cargo:rerun-if-changed=frontend/vite.config.ts");
            }
            _ => {
                println!("cargo:warning=Frontend build failed — console will use placeholder");
                ensure_placeholder_dist(frontend_dir);
            }
        }
    }
}

fn which_exists(name: &str) -> bool {
    Command::new("which")
        .arg(name)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn ensure_placeholder_dist(frontend_dir: &Path) {
    let dist = frontend_dir.join("dist");
    std::fs::create_dir_all(&dist).ok();
    if !dist.join("index.html").exists() {
        std::fs::write(
            dist.join("index.html"),
            "<html><body><h1>Arcan Console</h1><p>Frontend build failed.</p></body></html>",
        )
        .ok();
    }
}
