use anyhow::Result;

use crate::cli::HelpArgs;
use crate::help_text;

pub fn run(args: HelpArgs) -> Result<()> {
    if args.list || args.section.is_none() {
        println!("Help sections — `req help <name>`:\n");
        for s in help_text::sections() {
            println!("  {:<14} {}", s.name, s.summary);
        }
        println!("\nTip: `req help all` to print everything.");
        return Ok(());
    }
    let want = args.section.unwrap();
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
