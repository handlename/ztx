use std::process::Command;

fn main() {
    let hash = git_short_hash().unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=ZTX_GIT_HASH={hash}");

    // Rebuild when the checked-out commit changes so the embedded hash stays
    // current without a manual `cargo clean`. `--git-path` resolves the real
    // location even for a git worktree, where `.git` is a file, not a dir.
    for path in ["HEAD", "packed-refs"] {
        if let Some(resolved) = git_path(path) {
            println!("cargo:rerun-if-changed={resolved}");
        }
    }
    // A detached HEAD stores the commit directly in HEAD (tracked above);
    // on a branch, also watch the ref file that HEAD points to.
    if let Some(reference) = symbolic_ref()
        && let Some(resolved) = git_path(&reference)
    {
        println!("cargo:rerun-if-changed={resolved}");
    }
}

fn git_short_hash() -> Option<String> {
    git(&["rev-parse", "--short", "HEAD"])
}

fn git_path(name: &str) -> Option<String> {
    git(&["rev-parse", "--git-path", name])
}

fn symbolic_ref() -> Option<String> {
    git(&["symbolic-ref", "HEAD"])
}

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}
