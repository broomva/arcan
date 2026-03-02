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
            // Create a minimal dist so rust-embed doesn't fail on a missing folder.
            let dist = frontend_dir.join("dist");
            std::fs::create_dir_all(&dist).ok();
            if !dist.join("index.html").exists() {
                std::fs::write(
                    dist.join("index.html"),
                    "<html><body><h1>Arcan Console</h1><p>Frontend not built. Run <code>npm install && npm run build</code> in <code>crates/arcan-console/frontend/</code>.</p></body></html>",
                ).ok();
            }
            return;
        }

        // Check if node_modules exists; if not, run npm install.
        if !frontend_dir.join("node_modules").exists() {
            let status = Command::new("npm")
                .arg("install")
                .current_dir(frontend_dir)
                .status();
            match status {
                Ok(s) if s.success() => {}
                _ => {
                    println!("cargo:warning=npm install failed — console will use placeholder");
                    ensure_placeholder_dist(frontend_dir);
                    return;
                }
            }
        }

        // Run the production build.
        let status = Command::new("npm")
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
