// Implements REQ-0014 (markdown / json / csv / html exports).
use anyhow::Result;
use std::fs;
use std::path::PathBuf;

use crate::cli::{ExportArgs, ExportFormat};
use crate::model::{Project, Requirement};
use crate::storage::load_resolved;

pub fn run(args: ExportArgs, file: &Option<PathBuf>) -> Result<()> {
    let (_, project) = load_resolved(file)?;

    let output = match args.format {
        ExportFormat::Markdown => to_markdown(&project),
        ExportFormat::Json => serde_json::to_string_pretty(&project)?,
        ExportFormat::Csv => to_csv(&project)?,
        ExportFormat::Html => to_html(&project),
    };

    if args.output == "-" {
        print!("{}", output);
    } else {
        fs::write(&args.output, output)?;
        eprintln!("Wrote {}", args.output);
    }
    Ok(())
}

pub fn to_markdown(p: &Project) -> String {
    let mut s = String::new();
    s.push_str(&format!("# {}\n\n", p.name));
    s.push_str(&format!("_{} requirement(s). Generated {}._\n\n",
        p.requirements.len(),
        chrono::Utc::now().format("%Y-%m-%d %H:%M UTC")));
    for r in p.requirements.values() {
        s.push_str(&fmt_md(r));
        s.push_str("\n---\n\n");
    }
    s
}

fn fmt_md(r: &Requirement) -> String {
    let mut s = String::new();
    s.push_str(&format!("## {} — {}\n\n", r.id, r.title));
    s.push_str(&format!(
        "- **Kind:** {}  \n- **Priority:** {}  \n- **Status:** {}\n",
        r.kind.as_str(),
        r.priority.as_str(),
        r.status.as_str()
    ));
    if !r.tags.is_empty() {
        s.push_str(&format!("- **Tags:** {}\n", r.tags.join(", ")));
    }
    s.push_str(&format!("\n**Statement.** {}\n\n", r.statement));
    s.push_str(&format!("**Rationale.** {}\n\n", r.rationale));
    if !r.acceptance.is_empty() {
        s.push_str("**Acceptance criteria:**\n\n");
        for ac in &r.acceptance {
            s.push_str(&format!("- {}\n", ac));
        }
        s.push('\n');
    }
    if !r.links.is_empty() {
        s.push_str("**Links:**\n\n");
        for l in &r.links {
            s.push_str(&format!("- _{}_ → `{}`\n", l.kind.as_str(), l.target));
        }
    }
    s
}

fn to_csv(p: &Project) -> Result<String> {
    let mut out = String::from("id,title,kind,priority,status,tags,statement\n");
    for r in p.requirements.values() {
        out.push_str(&format!(
            "{},{},{},{},{},{},{}\n",
            csv_field(&r.id),
            csv_field(&r.title),
            r.kind.as_str(),
            r.priority.as_str(),
            r.status.as_str(),
            csv_field(&r.tags.join("|")),
            csv_field(&r.statement),
        ));
    }
    Ok(out)
}

fn csv_field(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn to_html(p: &Project) -> String {
    let body = to_markdown(p);
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{}</title>\
        <style>body{{font-family:system-ui,sans-serif;max-width:48rem;margin:2rem auto;padding:0 1rem;line-height:1.5;color:#222}}\
        h1,h2{{border-bottom:1px solid #ddd;padding-bottom:.3rem}}code{{background:#f4f4f4;padding:.1rem .3rem;border-radius:3px}}</style>\
        </head><body><pre style=\"white-space:pre-wrap;font-family:inherit\">{}</pre></body></html>",
        html_escape(&p.name),
        html_escape(&body)
    )
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}
