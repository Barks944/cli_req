// REQ-0105: one-shot project bootstrap. The friction-removal command
// for first-time adopters — runs init + hooks install + agents
// block install, then prints next-steps. Idempotent: re-running on
// an already-set-up project does not double-write or error.
use anyhow::Result;
use std::path::PathBuf;
use std::process::Command;

use crate::cli::SetupArgs;
use crate::storage;

pub fn run(args: SetupArgs) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let project_file = cwd.join("project.req");

    println!("req setup — bootstrapping this project for managed requirements\n");

    // 1. Init (if needed).
    if project_file.exists() {
        println!(
            "  [skip] project.req already exists at {}",
            project_file.display()
        );
    } else {
        let name = args
            .name
            .clone()
            .or_else(|| {
                cwd.file_name()
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| "unnamed project".into());
        let init_args = crate::cli::InitArgs {
            name: name.clone(),
            output: project_file.clone(),
            force: false,
            layout: crate::cli::LayoutArg::Single,
        };
        crate::commands::init::run(init_args)?;
        println!("  [done] req init -n \"{}\"", name);
    }

    // 2. Hooks (unless --no-hooks).
    let is_git = cwd.join(".git").exists();
    if args.no_hooks {
        println!("  [skip] hooks (--no-hooks)");
    } else if !is_git {
        println!("  [skip] hooks (.git directory not found — run `git init` first)");
    } else {
        let hooks_args = crate::cli::HooksArgs {
            action: "install".into(),
            repo: Some(cwd.clone()),
            force: args.force,
            claude_code: false,
            strict: args.strict,
        };
        crate::commands::hooks::run(hooks_args)?;
        println!(
            "  [done] req hooks install{}",
            if args.strict { " --strict" } else { "" }
        );

        // REQ-0105: auto-register the merge driver. Without this, the
        // .gitattributes line is dead — git would refuse the merge
        // driver call and `req doctor` would flag it as inactive on a
        // fresh setup. We know we're in a git repo at this branch, so
        // running `git config` is safe and one-shot.
        let registered_name = std::process::Command::new("git")
            .args(["config", "merge.req-merge.name", "req merge driver"])
            .current_dir(&cwd)
            .status();
        let registered_driver = std::process::Command::new("git")
            .args([
                "config",
                "merge.req-merge.driver",
                "req renumber --base %O || true",
            ])
            .current_dir(&cwd)
            .status();
        match (registered_name, registered_driver) {
            (Ok(s1), Ok(s2)) if s1.success() && s2.success() => {
                println!("  [done] git merge driver registered");
            }
            _ => {
                println!(
                    "  [skip] git merge driver (could not run `git config` — register manually)"
                );
            }
        }
    }

    // 3. AGENTS.md (unless --no-agents).
    if args.no_agents {
        println!("  [skip] AGENTS.md (--no-agents)");
    } else {
        let installed = install_agents_block(&cwd)?;
        if installed {
            println!("  [done] AGENTS.md managed block installed");
        } else {
            println!("  [skip] AGENTS.md already contains the managed block");
        }
    }

    // 4. Next steps.
    println!();
    println!("You're set up. Next steps:");
    println!();
    // REQ-0105: the example must be copy-paste-runnable. Functional
    // requirements need at least one --accept entry, so the example
    // includes one. Constraint/non-functional could omit -a but we
    // pick functional as the most common starting kind.
    println!("  req add -t \"...\" -s \"The system shall ...\" \\");
    println!("          -r \"...\" -k functional -p must \\");
    println!("          -a \"Concrete observable behaviour to check\"");
    println!("                                        # write your first requirement");
    println!("  req brief                             # session-start summary");
    println!("  req help agents                       # the agent workflow guide");
    println!();
    println!("Or, if you have an existing spec to ingest:");
    println!("  req import -f markdown your-spec.md");
    println!();
    Ok(())
}

fn install_agents_block(cwd: &PathBuf) -> Result<bool> {
    // Run `req help agents --install` via the existing help command
    // pipeline. Use a subprocess so the file-write logic stays in
    // help_cmd::run and we don't duplicate it.
    let bin = std::env::current_exe()?;
    let out = Command::new(bin)
        .current_dir(cwd)
        .args(["help", "agents", "--install"])
        .output()?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    Ok(stdout.contains("Installed") || stdout.contains("Updated"))
}

// Keep this dead-code suppression: storage isn't referenced directly
// here, only used through the sub-commands above.
#[allow(dead_code)]
fn _silence_unused() {
    let _ = storage::FORMAT_TAG;
}
