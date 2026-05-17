// Implements REQ-0018 (structured sectioned help) and REQ-0031 (idempotent
// install of any section into AGENTS.md via sentinel-delimited managed blocks).
use anyhow::{anyhow, Context, Result};
use std::fs;

use crate::cli::HelpArgs;
use crate::help_text::{self, Section};

pub fn run(args: HelpArgs) -> Result<()> {
    if args.list || args.section.is_none() {
        println!("Help sections — `req help <name>`:\n");
        for s in help_text::sections() {
            println!("  {:<14} {}", s.name, s.summary);
        }
        println!(
            "\nTip: `req help all` to print everything.\n     \
             `req help <section> --install` to write the section into AGENTS.md."
        );
        return Ok(());
    }
    let want = args.section.unwrap();
    if args.install {
        if want == "all" {
            return Err(anyhow!("--install requires a specific section, not 'all'"));
        }
        let s = help_text::section(&want)
            .ok_or_else(|| anyhow!("no such section: {}. Try `req help --list`.", want))?;
        return install_section(s, &args.path);
    }
    if want == "all" {
        for s in help_text::sections() {
            println!("## {}\n", s.name);
            println!("{}\n", s.body);
        }
        return Ok(());
    }
    match help_text::section(&want) {
        Some(s) => {
            println!("{}\n", s.name);
            println!("{}", s.body);
        }
        None => {
            eprintln!("No such section: {}. Try `req help --list`.", want);
            std::process::exit(2);
        }
    }
    Ok(())
}

fn install_section(section: &Section, path: &std::path::Path) -> Result<()> {
    let begin = format!("<!-- req:help:{}:begin -->", section.name);
    let end = format!("<!-- req:help:{}:end -->", section.name);
    let block = format!(
        "{begin}\n\n\
         <!-- Managed by `req help {} --install`. Re-run to refresh; edit OUTSIDE the markers to add your own notes. -->\n\n\
         ## req — {}\n\n\
         _{}_\n\n\
         ```\n{}\n```\n\n\
         {end}",
        section.name, section.name, section.summary, section.body
    );

    let existing = fs::read_to_string(path).unwrap_or_default();
    let new_contents = if let (Some(b), Some(e)) = (existing.find(&begin), existing.find(&end)) {
        let after_end = e + end.len();
        let mut s = String::new();
        s.push_str(&existing[..b]);
        s.push_str(&block);
        s.push_str(&existing[after_end..]);
        s
    } else {
        let mut s = existing.clone();
        if !s.is_empty() && !s.ends_with('\n') {
            s.push('\n');
        }
        if !s.is_empty() {
            s.push('\n');
        }
        s.push_str(&block);
        s.push('\n');
        s
    };

    fs::write(path, new_contents).with_context(|| format!("write {}", path.display()))?;
    println!(
        "Installed `{}` section into {} (between {} and {}).",
        section.name,
        path.display(),
        begin,
        end
    );
    Ok(())
}
