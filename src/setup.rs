//! `zedic setup zed`: merges the selection-sending task and its keybinding
//! into the user's Zed configuration.
//!
//! Safety rules (see the plan's risk table): changes are shown first and
//! applied only after confirmation (or `--yes`), a `.zedic.bak` backup is
//! written next to each modified file, and files that fail to parse as plain
//! JSON (Zed allows comments) are never rewritten — the snippet is printed
//! for manual merging instead.

use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::cli::SetupScope;

const TASK_LABEL: &str = "zedic: send selection";
const KEY_BINDING: &str = "cmd-alt-z";

fn zed_config_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("ZEDIC_ZED_CONFIG_DIR")
        && !dir.is_empty()
    {
        return PathBuf::from(dir);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".config/zed")
}

/// Project-local Zed config directory: `<worktree>/.zed`, where the worktree
/// is `$ZED_WORKTREE_ROOT` (as used elsewhere for session keying) or the
/// current directory.
fn project_config_dir() -> PathBuf {
    let root = std::env::var("ZED_WORKTREE_ROOT")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| ".".into());
    PathBuf::from(root).join(".zed")
}

fn task_entry() -> Value {
    json!({
        "label": TASK_LABEL,
        "command": "zedic",
        // `--from-zed-env` reads the selection from the ZED_* environment
        // variables. Passing them as $ZED_* args instead would let Zed's
        // shell re-execute the selected text (it interpolates into a
        // `zsh -c "..."` line), so keep the selection off the command line.
        "args": ["send", "--from-zed-env"],
        "reveal": "never",
        "hide": "always"
    })
}

fn keymap_entry() -> Value {
    json!({
        "context": "Editor",
        "bindings": {
            KEY_BINDING: ["task::Spawn", { "task_name": TASK_LABEL }]
        }
    })
}

/// Entry point for `zedic setup zed`.
///
/// `scope` selects the destination (global `~/.config/zed/` or project-local
/// `<worktree>/.zed/`). `preview` shows the changes without writing anything.
///
/// Zed keymaps are global-only (there is no project-local `keymap.json`), so
/// in project scope the keybinding is printed for manual addition instead of
/// being merged.
pub fn zed(assume_yes: bool, preview: bool, scope: SetupScope) -> io::Result<()> {
    let dir = match scope {
        SetupScope::Global => zed_config_dir(),
        SetupScope::Project => project_config_dir(),
    };

    if !preview {
        std::fs::create_dir_all(&dir)?;
    }

    merge_array_file(
        &dir.join("tasks.json"),
        task_entry(),
        |existing| existing.get("label").and_then(Value::as_str) == Some(TASK_LABEL),
        assume_yes,
        preview,
    )?;

    match scope {
        SetupScope::Global => {
            merge_array_file(
                &dir.join("keymap.json"),
                keymap_entry(),
                |existing| {
                    existing
                        .pointer(&format!("/bindings/{KEY_BINDING}"))
                        .is_some()
                },
                assume_yes,
                preview,
            )?;
        }
        SetupScope::Project => {
            // Zed keymaps are global-only; there is no project-local
            // keymap.json. Print the binding for manual merging instead.
            println!(
                "\nZed keymaps are global-only (no project-local keymap.json). \
                 Add this to ~/.config/zed/keymap.json manually, or run \
                 `zedic setup zed` with the default --scope global:\n{}\n",
                serde_json::to_string_pretty(&keymap_entry())?
            );
        }
    }

    // Zed replaces (not extends) `terminal.path_hyperlink_regexes` when the
    // user sets it, so auto-merging could drop Zed's built-in patterns.
    // Present the suggestion instead of writing it.
    println!(
        "\nOptional: Zed's built-in cmd+click already detects `path:line:col`. \
         If some path formats in your agent CLI's output are not clickable, \
         extend `terminal.path_hyperlink_regexes` in Zed's settings.json \
         (note: setting it replaces Zed's defaults, so include the patterns \
         you still want). zedic's hint mode (ctrl-] f) works regardless."
    );

    if preview {
        println!("\nPreview only: no files were modified.");
    } else {
        println!(
            "\nDone. In Zed, select text and press {KEY_BINDING} to send the \
             selection into the running zedic session."
        );
    }
    Ok(())
}

/// Merges `entry` into the JSON array stored at `path`.
fn merge_array_file(
    path: &Path,
    entry: Value,
    already_present: impl Fn(&Value) -> bool,
    assume_yes: bool,
    preview: bool,
) -> io::Result<()> {
    let pretty_entry = serde_json::to_string_pretty(&entry)?;

    let mut items = if path.exists() {
        let content = std::fs::read_to_string(path)?;
        match serde_json::from_str::<Value>(&content) {
            Ok(Value::Array(items)) => items,
            _ => {
                // Comments or an unexpected shape: never rewrite blindly.
                println!(
                    "{} could not be parsed as plain JSON (comments?).\n\
                     Add this entry manually:\n{pretty_entry}\n",
                    path.display()
                );
                return Ok(());
            }
        }
    } else {
        Vec::new()
    };

    if items.iter().any(&already_present) {
        println!(
            "{}: zedic entry already present, skipping",
            path.display()
        );
        return Ok(());
    }

    println!("Will add to {}:\n{pretty_entry}\n", path.display());

    if preview {
        return Ok(());
    }

    if !assume_yes && !confirm()? {
        println!("Skipped {}", path.display());
        return Ok(());
    }

    if path.exists() {
        std::fs::copy(path, path.with_extension("json.zedic.bak"))?;
    }
    items.push(entry);
    std::fs::write(path, serde_json::to_string_pretty(&Value::Array(items))?)?;
    println!("Updated {}", path.display());
    Ok(())
}

fn confirm() -> io::Result<bool> {
    print!("Apply? [y/N] ");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().lock().read_line(&mut answer)?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Serializes tests that mutate process-global environment variables.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn in_config_dir<T>(test: impl FnOnce(&Path) -> T) -> T {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: guarded by ENV_LOCK; tests in this module run one at a time.
        unsafe { std::env::set_var("ZEDIC_ZED_CONFIG_DIR", dir.path()) };
        let result = test(dir.path());
        unsafe { std::env::remove_var("ZEDIC_ZED_CONFIG_DIR") };
        result
    }

    fn in_worktree<T>(test: impl FnOnce(&Path) -> T) -> T {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: guarded by ENV_LOCK; tests in this module run one at a time.
        unsafe { std::env::set_var("ZED_WORKTREE_ROOT", dir.path()) };
        let result = test(dir.path());
        unsafe { std::env::remove_var("ZED_WORKTREE_ROOT") };
        result
    }

    #[test]
    fn creates_fresh_config_files() {
        in_config_dir(|dir| {
            zed(true, false, SetupScope::Global).unwrap();
            let tasks: Value =
                serde_json::from_str(&std::fs::read_to_string(dir.join("tasks.json")).unwrap())
                    .unwrap();
            assert_eq!(tasks[0]["label"], TASK_LABEL);
            let keymap: Value =
                serde_json::from_str(&std::fs::read_to_string(dir.join("keymap.json")).unwrap())
                    .unwrap();
            assert!(keymap[0]["bindings"][KEY_BINDING].is_array());
        });
    }

    #[test]
    fn merge_preserves_existing_entries_and_backs_up() {
        in_config_dir(|dir| {
            std::fs::write(
                dir.join("tasks.json"),
                r#"[{"label": "user task", "command": "make"}]"#,
            )
            .unwrap();
            zed(true, false, SetupScope::Global).unwrap();
            let tasks: Value =
                serde_json::from_str(&std::fs::read_to_string(dir.join("tasks.json")).unwrap())
                    .unwrap();
            assert_eq!(tasks.as_array().unwrap().len(), 2);
            assert_eq!(tasks[0]["label"], "user task");
            assert!(dir.join("tasks.json.zedic.bak").exists());
        });
    }

    #[test]
    fn running_twice_is_idempotent() {
        in_config_dir(|dir| {
            zed(true, false, SetupScope::Global).unwrap();
            zed(true, false, SetupScope::Global).unwrap();
            let tasks: Value =
                serde_json::from_str(&std::fs::read_to_string(dir.join("tasks.json")).unwrap())
                    .unwrap();
            assert_eq!(tasks.as_array().unwrap().len(), 1);
        });
    }

    #[test]
    fn commented_json_is_left_untouched() {
        in_config_dir(|dir| {
            let original = "// my tasks\n[]";
            std::fs::write(dir.join("tasks.json"), original).unwrap();
            zed(true, false, SetupScope::Global).unwrap();
            assert_eq!(
                std::fs::read_to_string(dir.join("tasks.json")).unwrap(),
                original
            );
        });
    }

    #[test]
    fn preview_writes_nothing() {
        in_config_dir(|dir| {
            zed(false, true, SetupScope::Global).unwrap();
            assert!(!dir.join("tasks.json").exists());
            assert!(!dir.join("keymap.json").exists());
        });
    }

    #[test]
    fn project_scope_writes_task_only_under_dot_zed() {
        in_worktree(|root| {
            zed(true, false, SetupScope::Project).unwrap();
            let tasks: Value = serde_json::from_str(
                &std::fs::read_to_string(root.join(".zed/tasks.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(tasks[0]["label"], TASK_LABEL);
            // Zed has no project-local keymap; the binding is only printed.
            assert!(!root.join(".zed/keymap.json").exists());
        });
    }
}
