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

    // REQ-0163: page the result set. `total` is the full match count before
    // paging; `page` is the slice the caller asked for.
    let total = filtered.len();
    let paginated = args.offset.is_some() || args.limit.is_some();
    let offset = args.offset.unwrap_or(0).min(total);
    let page: Vec<&Requirement> = match args.limit {
        Some(limit) => filtered.into_iter().skip(offset).take(limit).collect(),
        None => filtered.into_iter().skip(offset).collect(),
    };

    if args.json {
        // REQ-0163: when paged, wrap with the total so a caller can drive
        // further pages; unpaged callers keep the bare-array shape.
        if paginated {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "total": total,
                    "offset": offset,
                    "limit": args.limit,
                    "count": page.len(),
                    "items": page,
                }))?
            );
        } else {
            println!("{}", serde_json::to_string_pretty(&page)?);
        }
        return Ok(());
    }

    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(vec!["ID", "Title", "Kind", "Pri", "Status", "Tags"]);
    let shown = page.len();
    for r in page {
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
    // REQ-0163: always state the total so a paged view is unambiguous.
    if paginated {
        let last = offset + shown;
        println!("Showing {}-{} of {} match(es).", offset + 1, last, total);
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
    let hide_obsolete = !args.include_obsolete && !matches!(status, Some(Status::Obsolete));
    project
        .requirements
        .values()
        .filter(|r| !(hide_obsolete && matches!(r.status, Status::Obsolete)))
        .filter(|r| kind.is_none_or(|k| r.kind == k))
        .filter(|r| priority.is_none_or(|p| r.priority == p))
        .filter(|r| status.is_none_or(|s| r.status == s))
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
