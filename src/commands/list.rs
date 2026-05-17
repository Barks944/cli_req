// Discharges REQ-0001 (list sub-surface).
use anyhow::Result;
use comfy_table::{presets::UTF8_FULL, Cell, ContentArrangement, Table};
use std::path::PathBuf;

use crate::cli::ListArgs;
use crate::model::{Kind, Priority, Project, Requirement, Status};
use crate::storage::load_resolved;

pub fn run(args: ListArgs, file: &Option<PathBuf>) -> Result<()> {
    let (_, project) = load_resolved(file)?;
    let filtered = filter(&project, &args);

    if args.json {
        let refs: Vec<&Requirement> = filtered;
        println!("{}", serde_json::to_string_pretty(&refs)?);
        return Ok(());
    }

    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(vec!["ID", "Title", "Kind", "Pri", "Status", "Tags"]);
    for r in filtered {
        table.add_row(vec![
            Cell::new(&r.id),
            Cell::new(truncate(&r.title, 60)),
            Cell::new(r.kind.as_str()),
            Cell::new(r.priority.as_str()),
            Cell::new(r.status.as_str()),
            Cell::new(r.tags.join(", ")),
        ]);
    }
    if table.row_count() == 0 {
        println!("(no requirements match)");
    } else {
        println!("{table}");
    }
    Ok(())
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(n - 1).collect();
        out.push('…');
        out
    }
}

pub fn filter<'a>(project: &'a Project, args: &ListArgs) -> Vec<&'a Requirement> {
    let kind: Option<Kind> = args.kind.map(Into::into);
    let priority: Option<Priority> = args.priority.map(Into::into);
    let status: Option<Status> = args.status.map(Into::into);
    let q = args.query.as_deref().map(str::to_lowercase);
    // REQ-0073: hide Obsolete by default. Explicit --status obsolete or
    // --include-obsolete brings them back.
    let hide_obsolete = !args.include_obsolete
        && !matches!(status, Some(Status::Obsolete));
    project
        .requirements
        .values()
        .filter(|r| !(hide_obsolete && matches!(r.status, Status::Obsolete)))
        .filter(|r| kind.map_or(true, |k| r.kind == k))
        .filter(|r| priority.map_or(true, |p| r.priority == p))
        .filter(|r| status.map_or(true, |s| r.status == s))
        .filter(|r| args.tag.iter().all(|t| r.tags.iter().any(|rt| rt == t)))
        .filter(|r| match &q {
            None => true,
            Some(needle) => {
                r.title.to_lowercase().contains(needle)
                    || r.statement.to_lowercase().contains(needle)
            }
        })
        .collect()
}
