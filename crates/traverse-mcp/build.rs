//! Embed the release commit for `traverse-mcp --version` (Spec 119 FR-006).
fn main() {
    let sha = std::env::var("GITHUB_SHA")
        .or_else(|_| std::env::var("TRAVERSE_GIT_SHA"))
        .unwrap_or_else(|_| "unknown".to_string());
    // Only accept hex so a hostile env cannot inject rustc-env newlines.
    let sha = if sha.chars().all(|c| c.is_ascii_hexdigit()) && (7..=64).contains(&sha.len()) {
        sha
    } else if sha == "unknown" {
        sha
    } else {
        "unknown".to_string()
    };
    println!("cargo:rerun-if-env-changed=GITHUB_SHA");
    println!("cargo:rerun-if-env-changed=TRAVERSE_GIT_SHA");
    println!("cargo:rustc-env=TRAVERSE_GIT_SHA={sha}");
}
