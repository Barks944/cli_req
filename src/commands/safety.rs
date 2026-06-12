// REQ-0134: functional-safety command surface (IEC 61508).
//
// Three artifact families — hazards (HAZ), safety functions (SF), and
// safety requirements (SR) — plus `req trace`, the end-to-end safety
// case. Every mutation goes through the same load-lock-validate-save
// cycle as the requirement commands, records a reasoned history entry,
// and never lets a caller hand-set a SIL: integrity levels are always
// derived from the risk graph and the link structure.
use anyhow::{anyhow, Result};
use chrono::Utc;
use std::path::PathBuf;

use crate::cli::{
    HazardAddArgs, HazardAssessArgs, HazardCmd, HazardListArgs, HazardShowArgs, HazardUpdateArgs,
    SfAddArgs, SfCmd, SfListArgs, SfMitigateArgs, SfShowArgs, SfUpdateArgs, SreqAddArgs, SreqCmd,
    SreqListArgs, SreqRealizeArgs, SreqShowArgs, SreqUpdateArgs, SreqVerifyArgs, TraceArgs,
};
use crate::model::{
    EvidenceKind, Hazard, HazardStatus, Link, LinkKind, Project, SafetyFunction,
    SafetyFunctionStatus, SafetyRequirement, Sil, Status, TestOutcome, TestRecord,
};
use crate::storage::{self, load_for_mutation, load_resolved};

// ---------------------------------------------------------------------------
// id resolution
// ---------------------------------------------------------------------------

/// Normalise a typed id of a given family to canonical `PREFIX-NNNN`.
fn normalize(prefix: &str, raw: &str) -> String {
    let trimmed = raw.trim();
    let upper = trimmed.to_uppercase();
    let want = format!("{}-", prefix);
    let digits = if let Some(rest) = upper.strip_prefix(&want) {
        rest.to_string()
    } else if trimmed.chars().all(|c| c.is_ascii_digit()) && !trimmed.is_empty() {
        trimmed.to_string()
    } else {
        return upper;
    };
    match digits.parse::<u32>() {
        Ok(n) => format!("{}-{:04}", prefix, n),
        Err(_) => upper,
    }
}

fn resolve_haz(project: &Project, raw: &str) -> Result<String> {
    let id = normalize("HAZ", raw);
    if project.hazards.contains_key(&id) {
        Ok(id)
    } else {
        Err(anyhow!("no such hazard: {}", raw))
    }
}

fn resolve_sf(project: &Project, raw: &str) -> Result<String> {
    let id = normalize("SF", raw);
    if project.safety_functions.contains_key(&id) {
        Ok(id)
    } else {
        Err(anyhow!("no such safety function: {}", raw))
    }
}

fn resolve_sr(project: &Project, raw: &str) -> Result<String> {
    let id = normalize("SR", raw);
    if project.safety_requirements.contains_key(&id) {
        Ok(id)
    } else {
        Err(anyhow!("no such safety requirement: {}", raw))
    }
}

fn sil_str(s: Option<Sil>) -> String {
    s.map(|s| s.as_str().to_string())
        .unwrap_or_else(|| "—".to_string())
}

fn git_head() -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// hazard
// ---------------------------------------------------------------------------

pub fn run_hazard(cmd: HazardCmd, file: &Option<PathBuf>) -> Result<()> {
    match cmd {
        HazardCmd::Add(a) => hazard_add(a, file),
        HazardCmd::List(a) => hazard_list(a, file),
        HazardCmd::Show(a) => hazard_show(a, file),
        HazardCmd::Assess(a) => hazard_assess(a, file),
        HazardCmd::Update(a) => hazard_update(a, file),
    }
}

fn hazard_add(args: HazardAddArgs, file: &Option<PathBuf>) -> Result<()> {
    let (path, mut project, _lock) = load_for_mutation(file)?;
    let now = Utc::now();

    let consequence = args.consequence.map(Into::into);
    let frequency = args.frequency.map(Into::into);
    let avoidance = args.avoidance.map(Into::into);
    let probability = args.probability.map(Into::into);
    let fully_assessed = consequence.is_some()
        && frequency.is_some()
        && avoidance.is_some()
        && probability.is_some();
    let status = if fully_assessed {
        HazardStatus::Assessed
    } else {
        HazardStatus::Identified
    };

    let id = project.allocate_haz_id();
    let hazard = Hazard {
        id: id.clone(),
        title: args.title,
        description: args.description,
        operating_context: args.context,
        harm: args.harm,
        consequence,
        frequency,
        avoidance,
        probability,
        status,
        tags: args.tag,
        links: Vec::new(),
        created: now,
        updated: now,
        history: vec![super::history("created", None)],
    };
    project.hazards.insert(id.clone(), hazard.clone());
    project.updated = now;
    storage::save(&path, &project)?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&hazard)?);
    } else {
        println!("Added {}", id);
        match hazard.required_sil() {
            Some(s) => println!("Assessed: required {}", s.as_str()),
            None => println!(
                "Status: identified (run `req hazard assess {} -C .. -F .. -P .. -W ..` to derive a SIL)",
                id
            ),
        }
    }
    Ok(())
}

fn hazard_list(args: HazardListArgs, file: &Option<PathBuf>) -> Result<()> {
    let (_path, project) = load_resolved(file)?;
    let status_filter: Option<HazardStatus> = args.status.map(Into::into);
    let sil_filter = args.sil.as_deref().map(|s| s.to_uppercase());

    let mut rows: Vec<&Hazard> = project
        .hazards
        .values()
        .filter(|h| status_filter.map(|s| h.status == s).unwrap_or(true))
        .filter(|h| {
            sil_filter
                .as_ref()
                .map(|want| {
                    h.required_sil()
                        .map(|s| s.as_str().to_uppercase() == *want)
                        .unwrap_or(false)
                })
                .unwrap_or(true)
        })
        .filter(|h| {
            if !args.unmitigated {
                return true;
            }
            !project
                .safety_functions
                .values()
                .any(|sf| mitigates(sf, &h.id))
        })
        .collect();
    rows.sort_by(|a, b| a.id.cmp(&b.id));

    if args.json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }
    if rows.is_empty() {
        println!("No hazards.");
        return Ok(());
    }
    println!("{:<9}  {:<6}  {:<11}  TITLE", "ID", "SIL", "STATUS");
    for h in rows {
        println!(
            "{:<9}  {:<6}  {:<11}  {}",
            h.id,
            sil_str(h.required_sil()),
            h.status.as_str(),
            h.title
        );
    }
    Ok(())
}

fn hazard_show(args: HazardShowArgs, file: &Option<PathBuf>) -> Result<()> {
    let (_path, project) = load_resolved(file)?;
    let id = resolve_haz(&project, &args.id)?;
    let h = &project.hazards[&id];
    if args.json {
        println!("{}", serde_json::to_string_pretty(h)?);
        return Ok(());
    }
    println!("{}  {}", h.id, h.title);
    println!("  status:      {}", h.status.as_str());
    if !h.description.is_empty() {
        println!("  description: {}", h.description);
    }
    if !h.operating_context.is_empty() {
        println!("  context:     {}", h.operating_context);
    }
    println!("  harm:        {}", h.harm);
    match (h.consequence, h.frequency, h.avoidance, h.probability) {
        (Some(c), Some(f), Some(p), Some(w)) => {
            println!(
                "  risk:        {} · {} · {} · {}  ──►  required {}",
                c.as_str(),
                f.as_str(),
                p.as_str(),
                w.as_str(),
                sil_str(h.required_sil())
            );
        }
        _ => println!("  risk:        not yet assessed"),
    }
    let sfs: Vec<&SafetyFunction> = project
        .safety_functions
        .values()
        .filter(|sf| mitigates(sf, &h.id))
        .collect();
    if sfs.is_empty() {
        println!("  mitigated by: (none)");
    } else {
        println!("  mitigated by:");
        for sf in sfs {
            println!("    {} — {} [{}]", sf.id, sf.title, sf.status.as_str());
        }
    }
    if !h.tags.is_empty() {
        println!("  tags:        {}", h.tags.join(", "));
    }
    println!("\nRun `req trace {}` for the full safety case.", h.id);
    Ok(())
}

fn hazard_assess(args: HazardAssessArgs, file: &Option<PathBuf>) -> Result<()> {
    let (path, mut project, _lock) = load_for_mutation(file)?;
    let id = resolve_haz(&project, &args.id)?;
    let now = Utc::now();
    {
        let h = project.hazards.get_mut(&id).unwrap();
        h.consequence = Some(args.consequence.into());
        h.frequency = Some(args.frequency.into());
        h.avoidance = Some(args.avoidance.into());
        h.probability = Some(args.probability.into());
        if matches!(h.status, HazardStatus::Identified) {
            h.status = HazardStatus::Assessed;
        }
        h.updated = now;
        h.history.push(super::history("assessed", args.reason.clone()));
    }
    project.updated = now;
    let derived = project.hazards[&id].required_sil();
    storage::save(&path, &project)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&project.hazards[&id])?);
    } else {
        println!("Assessed {} ──► required {}", id, sil_str(derived));
    }
    Ok(())
}

fn hazard_update(args: HazardUpdateArgs, file: &Option<PathBuf>) -> Result<()> {
    let (path, mut project, _lock) = load_for_mutation(file)?;
    let id = resolve_haz(&project, &args.id)?;
    let now = Utc::now();
    {
        let h = project.hazards.get_mut(&id).unwrap();
        if let Some(t) = args.title {
            h.title = t;
        }
        if let Some(d) = args.description {
            h.description = d;
        }
        if let Some(c) = args.context {
            h.operating_context = c;
        }
        if let Some(harm) = args.harm {
            h.harm = harm;
        }
        if let Some(s) = args.status {
            h.status = s.into();
        }
        for t in &args.add_tag {
            if !h.tags.contains(t) {
                h.tags.push(t.clone());
            }
        }
        h.tags.retain(|t| !args.remove_tag.contains(t));
        h.updated = now;
        h.history.push(super::history("updated", args.reason.clone()));
    }
    project.updated = now;
    storage::save(&path, &project)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&project.hazards[&id])?);
    } else {
        println!("Updated {}", id);
    }
    Ok(())
}

fn mitigates(sf: &SafetyFunction, haz_id: &str) -> bool {
    sf.links
        .iter()
        .any(|l| l.kind == LinkKind::Mitigates && l.target == haz_id)
}

fn realizes(sr: &SafetyRequirement, sf_id: &str) -> bool {
    sr.links
        .iter()
        .any(|l| l.kind == LinkKind::Realizes && l.target == sf_id)
}

// ---------------------------------------------------------------------------
// safety function
// ---------------------------------------------------------------------------

pub fn run_sf(cmd: SfCmd, file: &Option<PathBuf>) -> Result<()> {
    match cmd {
        SfCmd::Add(a) => sf_add(a, file),
        SfCmd::List(a) => sf_list(a, file),
        SfCmd::Show(a) => sf_show(a, file),
        SfCmd::Update(a) => sf_update(a, file),
        SfCmd::Mitigate(a) => sf_mitigate(a, file),
    }
}

fn sf_add(args: SfAddArgs, file: &Option<PathBuf>) -> Result<()> {
    let (path, mut project, _lock) = load_for_mutation(file)?;
    let now = Utc::now();

    let mut links = Vec::new();
    for raw in &args.mitigates {
        let hid = resolve_haz(&project, raw)?;
        links.push(Link {
            kind: LinkKind::Mitigates,
            target: hid,
        });
    }
    let status = if links.is_empty() {
        SafetyFunctionStatus::Proposed
    } else {
        SafetyFunctionStatus::Allocated
    };

    let id = project.allocate_sf_id();
    let sf = SafetyFunction {
        id: id.clone(),
        title: args.title,
        description: args.description,
        safe_state: args.safe_state,
        status,
        tags: args.tag,
        links: links.clone(),
        created: now,
        updated: now,
        history: vec![super::history("created", None)],
    };
    project.safety_functions.insert(id.clone(), sf.clone());
    // A hazard that just gained a mitigation advances to Mitigated.
    for l in &links {
        if let Some(h) = project.hazards.get_mut(&l.target) {
            if matches!(h.status, HazardStatus::Identified | HazardStatus::Assessed) {
                h.status = HazardStatus::Mitigated;
                h.updated = now;
                h.history
                    .push(super::history(format!("mitigated by {}", id), None));
            }
        }
    }
    project.updated = now;
    let alloc = project.allocated_sil(&sf);
    storage::save(&path, &project)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&sf)?);
    } else {
        println!("Added {}", id);
        println!("  allocated SIL: {}", sil_str(alloc));
    }
    Ok(())
}

fn sf_list(args: SfListArgs, file: &Option<PathBuf>) -> Result<()> {
    let (_path, project) = load_resolved(file)?;
    let status_filter: Option<SafetyFunctionStatus> = args.status.map(Into::into);
    let sil_filter = args.sil.as_deref().map(|s| s.to_uppercase());

    let mut rows: Vec<&SafetyFunction> = project
        .safety_functions
        .values()
        .filter(|sf| status_filter.map(|s| sf.status == s).unwrap_or(true))
        .filter(|sf| {
            sil_filter
                .as_ref()
                .map(|want| {
                    project
                        .allocated_sil(sf)
                        .map(|s| s.as_str().to_uppercase() == *want)
                        .unwrap_or(false)
                })
                .unwrap_or(true)
        })
        .filter(|sf| {
            if !args.unrealized {
                return true;
            }
            !project
                .safety_requirements
                .values()
                .any(|sr| realizes(sr, &sf.id))
        })
        .collect();
    rows.sort_by(|a, b| a.id.cmp(&b.id));

    if args.json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }
    if rows.is_empty() {
        println!("No safety functions.");
        return Ok(());
    }
    println!("{:<8}  {:<6}  {:<12}  TITLE", "ID", "SIL", "STATUS");
    for sf in rows {
        println!(
            "{:<8}  {:<6}  {:<12}  {}",
            sf.id,
            sil_str(project.allocated_sil(sf)),
            sf.status.as_str(),
            sf.title
        );
    }
    Ok(())
}

fn sf_show(args: SfShowArgs, file: &Option<PathBuf>) -> Result<()> {
    let (_path, project) = load_resolved(file)?;
    let id = resolve_sf(&project, &args.id)?;
    let sf = &project.safety_functions[&id];
    if args.json {
        println!("{}", serde_json::to_string_pretty(sf)?);
        return Ok(());
    }
    println!("{}  {}", sf.id, sf.title);
    println!("  status:        {}", sf.status.as_str());
    if !sf.description.is_empty() {
        println!("  description:   {}", sf.description);
    }
    if !sf.safe_state.is_empty() {
        println!("  safe state:    {}", sf.safe_state);
    }
    println!("  allocated SIL: {}", sil_str(project.allocated_sil(sf)));
    let hazards: Vec<&Link> = sf
        .links
        .iter()
        .filter(|l| l.kind == LinkKind::Mitigates)
        .collect();
    if hazards.is_empty() {
        println!("  mitigates:     (no hazard)");
    } else {
        println!("  mitigates:");
        for l in hazards {
            let title = project
                .hazards
                .get(&l.target)
                .map(|h| h.title.as_str())
                .unwrap_or("<missing>");
            println!("    {} — {}", l.target, title);
        }
    }
    let srs: Vec<&SafetyRequirement> = project
        .safety_requirements
        .values()
        .filter(|sr| realizes(sr, &sf.id))
        .collect();
    if srs.is_empty() {
        println!("  realized by:   (none)");
    } else {
        println!("  realized by:");
        for sr in srs {
            println!("    {} — {} [{}]", sr.id, sr.title, sr.status.as_str());
        }
    }
    println!("\nRun `req trace {}` for the full safety case.", sf.id);
    Ok(())
}

fn sf_update(args: SfUpdateArgs, file: &Option<PathBuf>) -> Result<()> {
    let (path, mut project, _lock) = load_for_mutation(file)?;
    let id = resolve_sf(&project, &args.id)?;
    let now = Utc::now();
    {
        let sf = project.safety_functions.get_mut(&id).unwrap();
        if let Some(t) = args.title {
            sf.title = t;
        }
        if let Some(d) = args.description {
            sf.description = d;
        }
        if let Some(s) = args.safe_state {
            sf.safe_state = s;
        }
        if let Some(s) = args.status {
            sf.status = s.into();
        }
        for t in &args.add_tag {
            if !sf.tags.contains(t) {
                sf.tags.push(t.clone());
            }
        }
        sf.tags.retain(|t| !args.remove_tag.contains(t));
        sf.updated = now;
        sf.history.push(super::history("updated", args.reason.clone()));
    }
    project.updated = now;
    storage::save(&path, &project)?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&project.safety_functions[&id])?
        );
    } else {
        println!("Updated {}", id);
    }
    Ok(())
}

fn sf_mitigate(args: SfMitigateArgs, file: &Option<PathBuf>) -> Result<()> {
    let (path, mut project, _lock) = load_for_mutation(file)?;
    let sf_id = resolve_sf(&project, &args.sf)?;
    let haz_id = resolve_haz(&project, &args.hazard)?;
    let now = Utc::now();
    {
        let sf = project.safety_functions.get_mut(&sf_id).unwrap();
        if args.remove {
            sf.links
                .retain(|l| !(l.kind == LinkKind::Mitigates && l.target == haz_id));
            sf.history
                .push(super::history(format!("unlinked mitigates {}", haz_id), None));
        } else if mitigates(sf, &haz_id) {
            return Err(anyhow!("{} already mitigates {}", sf_id, haz_id));
        } else {
            sf.links.push(Link {
                kind: LinkKind::Mitigates,
                target: haz_id.clone(),
            });
            if matches!(sf.status, SafetyFunctionStatus::Proposed) {
                sf.status = SafetyFunctionStatus::Allocated;
            }
            sf.history
                .push(super::history(format!("mitigates {}", haz_id), None));
        }
        sf.updated = now;
    }
    // Advance the hazard to Mitigated when it first acquires a mitigation.
    if !args.remove {
        if let Some(h) = project.hazards.get_mut(&haz_id) {
            if matches!(h.status, HazardStatus::Identified | HazardStatus::Assessed) {
                h.status = HazardStatus::Mitigated;
                h.updated = now;
                h.history
                    .push(super::history(format!("mitigated by {}", sf_id), None));
            }
        }
    }
    project.updated = now;
    storage::save(&path, &project)?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&project.safety_functions[&sf_id])?
        );
    } else if args.remove {
        println!("{} no longer mitigates {}", sf_id, haz_id);
    } else {
        println!("{} mitigates {}", sf_id, haz_id);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// safety requirement
// ---------------------------------------------------------------------------

pub fn run_sreq(cmd: SreqCmd, file: &Option<PathBuf>) -> Result<()> {
    match cmd {
        SreqCmd::Add(a) => sreq_add(a, file),
        SreqCmd::List(a) => sreq_list(a, file),
        SreqCmd::Show(a) => sreq_show(a, file),
        SreqCmd::Update(a) => sreq_update(a, file),
        SreqCmd::Realize(a) => sreq_realize(a, file),
        SreqCmd::Verify(a) => sreq_verify(a, file),
    }
}

fn sreq_add(args: SreqAddArgs, file: &Option<PathBuf>) -> Result<()> {
    let (path, mut project, _lock) = load_for_mutation(file)?;
    let now = Utc::now();

    let mut links = Vec::new();
    for raw in &args.realizes {
        let sfid = resolve_sf(&project, raw)?;
        links.push(Link {
            kind: LinkKind::Realizes,
            target: sfid,
        });
    }

    let id = project.allocate_sr_id();
    let sr = SafetyRequirement {
        id: id.clone(),
        title: args.title,
        statement: args.statement,
        rationale: args.rationale,
        acceptance: args.acceptance,
        priority: args.priority.into(),
        status: Status::Draft,
        tags: args.tag,
        links,
        created: now,
        updated: now,
        history: vec![super::history("created", None)],
        tests: Vec::new(),
    };
    project.safety_requirements.insert(id.clone(), sr.clone());
    project.updated = now;
    let sil = project.inherited_sil(&sr);
    storage::save(&path, &project)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&sr)?);
    } else {
        println!("Added {}", id);
        println!("  inherits SIL: {}", sil_str(sil));
        println!(
            "Next: add `// {}:` to the source that implements this, then \
             `req sreq verify {} --by automated ...`.",
            id, id
        );
    }
    Ok(())
}

fn sreq_list(args: SreqListArgs, file: &Option<PathBuf>) -> Result<()> {
    let (_path, project) = load_resolved(file)?;
    let status_filter: Option<Status> = args.status.map(Into::into);
    let sil_filter = args.sil.as_deref().map(|s| s.to_uppercase());

    let mut rows: Vec<&SafetyRequirement> = project
        .safety_requirements
        .values()
        .filter(|sr| status_filter.map(|s| sr.status == s).unwrap_or(true))
        .filter(|sr| {
            sil_filter
                .as_ref()
                .map(|want| {
                    project
                        .inherited_sil(sr)
                        .map(|s| s.as_str().to_uppercase() == *want)
                        .unwrap_or(false)
                })
                .unwrap_or(true)
        })
        .filter(|sr| !args.unverified || !matches!(sr.status, Status::Verified))
        .collect();
    rows.sort_by(|a, b| a.id.cmp(&b.id));

    if args.json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }
    if rows.is_empty() {
        println!("No safety requirements.");
        return Ok(());
    }
    println!("{:<8}  {:<6}  {:<12}  TITLE", "ID", "SIL", "STATUS");
    for sr in rows {
        println!(
            "{:<8}  {:<6}  {:<12}  {}",
            sr.id,
            sil_str(project.inherited_sil(sr)),
            sr.status.as_str(),
            sr.title
        );
    }
    Ok(())
}

fn sreq_show(args: SreqShowArgs, file: &Option<PathBuf>) -> Result<()> {
    let (_path, project) = load_resolved(file)?;
    let id = resolve_sr(&project, &args.id)?;
    let sr = &project.safety_requirements[&id];
    if args.json {
        println!("{}", serde_json::to_string_pretty(sr)?);
        return Ok(());
    }
    println!("{}  {}", sr.id, sr.title);
    println!("  status:       {}", sr.status.as_str());
    println!("  priority:     {}", sr.priority.as_str());
    println!("  inherits SIL: {}", sil_str(project.inherited_sil(sr)));
    println!("  statement:    {}", sr.statement);
    println!("  rationale:    {}", sr.rationale);
    if !sr.acceptance.is_empty() {
        println!("  acceptance:");
        for (i, a) in sr.acceptance.iter().enumerate() {
            println!("    {}. {}", i + 1, a);
        }
    }
    let sfs: Vec<&Link> = sr
        .links
        .iter()
        .filter(|l| l.kind == LinkKind::Realizes)
        .collect();
    if sfs.is_empty() {
        println!("  realizes:     (no safety function)");
    } else {
        println!("  realizes:");
        for l in sfs {
            let title = project
                .safety_functions
                .get(&l.target)
                .map(|sf| sf.title.as_str())
                .unwrap_or("<missing>");
            println!("    {} — {}", l.target, title);
        }
    }
    match sr.tests.last() {
        Some(t) => println!(
            "  evidence:     {} · {} · {}",
            t.kind.as_str(),
            if t.commit.is_empty() { "—" } else { &t.commit[..t.commit.len().min(8)] },
            t.outcome.as_str()
        ),
        None => println!("  evidence:     none"),
    }
    println!("\nRun `req trace {}` for the full safety case.", sr.id);
    Ok(())
}

fn sreq_update(args: SreqUpdateArgs, file: &Option<PathBuf>) -> Result<()> {
    let (path, mut project, _lock) = load_for_mutation(file)?;
    let id = resolve_sr(&project, &args.id)?;
    let now = Utc::now();
    {
        let sr = project.safety_requirements.get_mut(&id).unwrap();
        if let Some(t) = args.title {
            sr.title = t;
        }
        if let Some(s) = args.statement {
            sr.statement = s;
        }
        if let Some(r) = args.rationale {
            sr.rationale = r;
        }
        if let Some(a) = args.acceptance {
            sr.acceptance = a;
        }
        for a in &args.add_acceptance {
            sr.acceptance.push(a.clone());
        }
        if let Some(p) = args.priority {
            sr.priority = p.into();
        }
        if let Some(s) = args.status {
            sr.status = s.into();
        }
        for t in &args.add_tag {
            if !sr.tags.contains(t) {
                sr.tags.push(t.clone());
            }
        }
        sr.tags.retain(|t| !args.remove_tag.contains(t));
        sr.updated = now;
        sr.history.push(super::history("updated", args.reason.clone()));
    }
    project.updated = now;
    storage::save(&path, &project)?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&project.safety_requirements[&id])?
        );
    } else {
        println!("Updated {}", id);
    }
    Ok(())
}

fn sreq_realize(args: SreqRealizeArgs, file: &Option<PathBuf>) -> Result<()> {
    let (path, mut project, _lock) = load_for_mutation(file)?;
    let sr_id = resolve_sr(&project, &args.sreq)?;
    let sf_id = resolve_sf(&project, &args.sf)?;
    let now = Utc::now();
    {
        let sr = project.safety_requirements.get_mut(&sr_id).unwrap();
        if args.remove {
            sr.links
                .retain(|l| !(l.kind == LinkKind::Realizes && l.target == sf_id));
            sr.history
                .push(super::history(format!("unlinked realizes {}", sf_id), None));
        } else if realizes(sr, &sf_id) {
            return Err(anyhow!("{} already realizes {}", sr_id, sf_id));
        } else {
            sr.links.push(Link {
                kind: LinkKind::Realizes,
                target: sf_id.clone(),
            });
            sr.history
                .push(super::history(format!("realizes {}", sf_id), None));
        }
        sr.updated = now;
    }
    project.updated = now;
    storage::save(&path, &project)?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&project.safety_requirements[&sr_id])?
        );
    } else if args.remove {
        println!("{} no longer realizes {}", sr_id, sf_id);
    } else {
        println!("{} realizes {}", sr_id, sf_id);
    }
    Ok(())
}

fn sreq_verify(args: SreqVerifyArgs, file: &Option<PathBuf>) -> Result<()> {
    let (path, mut project, _lock) = load_for_mutation(file)?;
    let id = resolve_sr(&project, &args.id)?;
    let kind: EvidenceKind = args.by.into();
    let inherited = project.inherited_sil(&project.safety_requirements[&id]);

    // REQ-0135: the SIL-rigour gate. A SIL 3/4 safety requirement cannot
    // reach Verified on inspection alone — it needs automated or
    // composition evidence. Block by default; --force records an
    // explicit, audited exception in the notes.
    if let Some(sil) = inherited {
        let needs_strong = sil.rank() >= Sil::Sil3.rank();
        if needs_strong && matches!(kind, EvidenceKind::Inspection) && !args.force {
            return Err(anyhow!(
                "SIL-rigour gate: {} inherits {} — inspection-only evidence is not \
                 sufficient. Provide automated or composition evidence, or pass \
                 --force to record an explicit exception.",
                id,
                sil.as_str()
            ));
        }
    }

    let now = Utc::now();
    let mut notes = args.notes.clone();
    if !args.cites.is_empty() {
        notes = format!("cites {} — {}", args.cites.join(", "), notes);
    }
    if args.force && matches!(kind, EvidenceKind::Inspection) {
        notes = format!("[SIL-gate exception] {}", notes);
    }
    let record = TestRecord {
        at: now,
        actor: super::current_actor(),
        commit: git_head(),
        outcome: TestOutcome::Pass,
        notes,
        kind,
        content_hash: None,
        linked_files: None,
    };
    {
        let sr = project.safety_requirements.get_mut(&id).unwrap();
        sr.tests.push(record);
        if args.promote {
            sr.status = Status::Verified;
        }
        sr.updated = now;
        sr.history.push(super::history(
            if args.promote {
                "verified (promoted)"
            } else {
                "evidence recorded"
            },
            None,
        ));
    }
    project.updated = now;
    storage::save(&path, &project)?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&project.safety_requirements[&id])?
        );
    } else {
        println!(
            "Recorded {} evidence for {}{}",
            kind.as_str(),
            id,
            if args.promote { " → Verified" } else { "" }
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// trace — end-to-end safety case
// ---------------------------------------------------------------------------

pub fn run_trace(args: TraceArgs, file: &Option<PathBuf>) -> Result<()> {
    let (_path, project) = load_resolved(file)?;
    let raw = args.id.trim().to_uppercase();
    if raw.starts_with("HAZ") {
        let id = resolve_haz(&project, &args.id)?;
        trace_hazard(&project, &id, args.json)
    } else if raw.starts_with("SF") {
        let id = resolve_sf(&project, &args.id)?;
        // Trace each hazard the SF mitigates; if none, trace the SF alone.
        trace_from_sf(&project, &id, args.json)
    } else if raw.starts_with("SR") {
        let id = resolve_sr(&project, &args.id)?;
        trace_from_sr(&project, &id, args.json)
    } else {
        Err(anyhow!(
            "trace expects a HAZ-/SF-/SR- id; got {}",
            args.id
        ))
    }
}

/// Verdict for a single hazard's safety case.
struct Verdict {
    required: Option<Sil>,
    allocated: Option<Sil>,
    sr_total: usize,
    sr_verified: usize,
    adequate: bool,
    complete: bool,
    blocking: Vec<String>,
}

fn assess_hazard(project: &Project, haz_id: &str) -> Verdict {
    let h = &project.hazards[haz_id];
    let required = h.required_sil();
    let sfs: Vec<&SafetyFunction> = project
        .safety_functions
        .values()
        .filter(|sf| mitigates(sf, haz_id))
        .collect();
    let allocated = sfs
        .iter()
        .filter_map(|sf| project.allocated_sil(sf))
        .max_by_key(|s| s.rank());
    let adequate = match (required, allocated) {
        (Some(r), Some(a)) => a.rank() >= r.rank(),
        (Some(_), None) => false,
        (None, _) => true, // not assessed: adequacy undecided, treat as not-blocking
    };
    let mut sr_total = 0;
    let mut sr_verified = 0;
    let mut blocking = Vec::new();
    for sf in &sfs {
        for sr in project
            .safety_requirements
            .values()
            .filter(|sr| realizes(sr, &sf.id))
        {
            sr_total += 1;
            if matches!(sr.status, Status::Verified) {
                sr_verified += 1;
            } else {
                blocking.push(format!("{} not verified", sr.id));
            }
        }
    }
    if sfs.is_empty() {
        blocking.push("no mitigating safety function".to_string());
    } else if sr_total == 0 {
        blocking.push("no realizing safety requirement".to_string());
    }
    let complete = adequate && blocking.is_empty();
    Verdict {
        required,
        allocated,
        sr_total,
        sr_verified,
        adequate,
        complete,
        blocking,
    }
}

fn trace_hazard(project: &Project, haz_id: &str, json: bool) -> Result<()> {
    let h = &project.hazards[haz_id];
    let v = assess_hazard(project, haz_id);
    if json {
        let out = serde_json::json!({
            "hazard": h,
            "required_sil": v.required.map(|s| s.as_str()),
            "allocated_sil": v.allocated.map(|s| s.as_str()),
            "adequate": v.adequate,
            "complete": v.complete,
            "safety_requirements": { "total": v.sr_total, "verified": v.sr_verified },
            "blocking": v.blocking,
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    println!("{}  {}  [{}]", h.id, h.title, h.status.as_str());
    println!("  harm:     {}", h.harm);
    if !h.operating_context.is_empty() {
        println!("  context:  {}", h.operating_context);
    }
    match (h.consequence, h.frequency, h.avoidance, h.probability) {
        (Some(c), Some(f), Some(p), Some(w)) => println!(
            "  risk:     {} · {} · {} · {}  ──►  required {}",
            c.as_str(), f.as_str(), p.as_str(), w.as_str(), sil_str(v.required)
        ),
        _ => println!("  risk:     not yet assessed"),
    }

    let sfs: Vec<&SafetyFunction> = project
        .safety_functions
        .values()
        .filter(|sf| mitigates(sf, haz_id))
        .collect();
    if sfs.is_empty() {
        println!("  │");
        println!("  └─ mitigated by ── (none)");
    }
    for sf in &sfs {
        let alloc = project.allocated_sil(sf);
        let meets = match (v.required, alloc) {
            (Some(r), Some(a)) => {
                if a.rank() >= r.rank() {
                    "✓ meets required"
                } else {
                    "✗ below required"
                }
            }
            _ => "",
        };
        println!("  │");
        println!("  └─ mitigated by ─────────────────────────────────");
        println!("     {}  {}  [{}]", sf.id, sf.title, sf.status.as_str());
        if !sf.safe_state.is_empty() {
            println!("       safe state:    {}", sf.safe_state);
        }
        println!("       allocated SIL: {}   {}", sil_str(alloc), meets);
        let srs: Vec<&SafetyRequirement> = project
            .safety_requirements
            .values()
            .filter(|sr| realizes(sr, &sf.id))
            .collect();
        if srs.is_empty() {
            println!("       └─ realized by ── (none)");
        } else {
            println!("       └─ realized by ───────────────────────");
        }
        for sr in srs {
            let mark = if matches!(sr.status, Status::Verified) {
                "✓"
            } else {
                "⚠"
            };
            println!(
                "          {}  {}  [{}] {}",
                sr.id, sr.title, sr.status.as_str(), mark
            );
            println!("            inherits SIL {}", sil_str(project.inherited_sil(sr)));
            match sr.tests.last() {
                Some(t) => println!(
                    "            evidence: {} · {}",
                    t.kind.as_str(),
                    if t.commit.is_empty() {
                        "—".to_string()
                    } else {
                        t.commit[..t.commit.len().min(8)].to_string()
                    }
                ),
                None => println!("            evidence: none                       ✗ unverified"),
            }
        }
    }

    println!();
    let verdict = if v.complete {
        "✓ COMPLETE"
    } else {
        "⚠ INCOMPLETE"
    };
    println!("  SAFETY CASE:  {}", verdict);
    println!(
        "    required {} — allocated {}    {}",
        sil_str(v.required),
        sil_str(v.allocated),
        if v.adequate { "✓ adequate" } else { "✗ inadequate" }
    );
    println!(
        "    safety requirements: {} verified of {}",
        v.sr_verified, v.sr_total
    );
    if !v.blocking.is_empty() {
        println!("    blocking: {}", v.blocking.join("; "));
    }
    Ok(())
}

fn trace_from_sf(project: &Project, sf_id: &str, json: bool) -> Result<()> {
    let sf = &project.safety_functions[sf_id];
    let hazards: Vec<String> = sf
        .links
        .iter()
        .filter(|l| l.kind == LinkKind::Mitigates)
        .map(|l| l.target.clone())
        .filter(|t| project.hazards.contains_key(t))
        .collect();
    if hazards.is_empty() {
        if json {
            println!("{}", serde_json::to_string_pretty(sf)?);
        } else {
            println!(
                "{} mitigates no hazard yet — nothing to trace upward.",
                sf_id
            );
            println!("Run `req sf show {}` for its realizing requirements.", sf_id);
        }
        return Ok(());
    }
    for (i, hid) in hazards.iter().enumerate() {
        if i > 0 {
            println!();
        }
        trace_hazard(project, hid, json)?;
    }
    Ok(())
}

fn trace_from_sr(project: &Project, sr_id: &str, json: bool) -> Result<()> {
    let sr = &project.safety_requirements[sr_id];
    let sfs: Vec<String> = sr
        .links
        .iter()
        .filter(|l| l.kind == LinkKind::Realizes)
        .map(|l| l.target.clone())
        .filter(|t| project.safety_functions.contains_key(t))
        .collect();
    if sfs.is_empty() {
        if json {
            println!("{}", serde_json::to_string_pretty(sr)?);
        } else {
            println!(
                "{} realizes no safety function yet — nothing to trace upward.",
                sr_id
            );
        }
        return Ok(());
    }
    for (i, sfid) in sfs.iter().enumerate() {
        if i > 0 {
            println!();
        }
        trace_from_sf(project, sfid, json)?;
    }
    Ok(())
}
