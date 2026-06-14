// Implements REQ-0025 (renumber colliding IDs after merge, rewrite links,
// record history, support --dry-run).
// REQ-0159: collision renumber + link rewrite now covers the safety
// artifact families (HAZ / SF / SR) as well as ordinary requirements, so a
// merge that renumbers a hazard or safety function no longer leaves its
// inbound `mitigates` / `realizes` links dangling.
use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;

use crate::cli::RenumberArgs;
use crate::model::{Hazard, Link, Project, Requirement, SafetyFunction, SafetyRequirement};
use crate::storage::{self, load_with_options, resolve_path};

/// The fields renumber needs from any artifact family: a collision key
/// (created + title), an id to rewrite, link targets to rewrite, and a
/// history trail to annotate.
trait Artifact {
    fn created(&self) -> DateTime<Utc>;
    fn title(&self) -> &str;
    fn set_id(&mut self, id: String);
    fn note_renamed(&mut self, old: &str);
    fn links_mut(&mut self) -> &mut Vec<Link>;
}

macro_rules! impl_artifact {
    ($t:ty) => {
        impl Artifact for $t {
            fn created(&self) -> DateTime<Utc> {
                self.created
            }
            fn title(&self) -> &str {
                &self.title
            }
            fn set_id(&mut self, id: String) {
                self.id = id;
            }
            fn note_renamed(&mut self, old: &str) {
                self.history.push(super::history(
                    format!("renumbered from {} (merge with base)", old),
                    None,
                ));
            }
            fn links_mut(&mut self) -> &mut Vec<Link> {
                &mut self.links
            }
        }
    };
}
impl_artifact!(Requirement);
impl_artifact!(Hazard);
impl_artifact!(SafetyFunction);
impl_artifact!(SafetyRequirement);

pub fn run(args: RenumberArgs, file: &Option<PathBuf>) -> Result<()> {
    let path = resolve_path(file);
    let _lock = crate::storage::acquire_lock(&path)?;
    // After a merge resolution the integrity hash is typically wrong — force load.
    let mut current = load_with_options(&path, true)?;
    let base = load_from_git_ref(&args.base, path.file_name().unwrap().to_str().unwrap())?;

    // REQ-0159: plan collision renames per family, each drawing from its own
    // counter, then rewrite links across all families through one map.
    let mut next_id = current.next_id.max(base.next_id);
    let mut next_haz = current.next_haz_id.max(base.next_haz_id);
    let mut next_sf = current.next_sf_id.max(base.next_sf_id);
    let mut next_sr = current.next_sr_id.max(base.next_sr_id);

    let req_renames = plan_renames(
        &current.requirements,
        &base.requirements,
        &mut next_id,
        "REQ",
    );
    let haz_renames = plan_renames(&current.hazards, &base.hazards, &mut next_haz, "HAZ");
    let sf_renames = plan_renames(
        &current.safety_functions,
        &base.safety_functions,
        &mut next_sf,
        "SF",
    );
    let sr_renames = plan_renames(
        &current.safety_requirements,
        &base.safety_requirements,
        &mut next_sr,
        "SR",
    );

    let all_renames: Vec<(String, String)> = req_renames
        .iter()
        .chain(&haz_renames)
        .chain(&sf_renames)
        .chain(&sr_renames)
        .cloned()
        .collect();

    if all_renames.is_empty() {
        println!("No ID collisions against {}.", args.base);
        return Ok(());
    }

    println!("Planned renames:");
    for (old, new) in &all_renames {
        println!("  {} -> {}", old, new);
    }
    if args.dry_run {
        return Ok(());
    }

    // Re-key each family, then rewrite every link target across all families
    // through one combined map so a renamed HAZ/SF is followed by its inbound
    // `mitigates` / `realizes` links wherever they live.
    rekey(&mut current.requirements, &req_renames);
    rekey(&mut current.hazards, &haz_renames);
    rekey(&mut current.safety_functions, &sf_renames);
    rekey(&mut current.safety_requirements, &sr_renames);

    let map: HashMap<String, String> = all_renames.iter().cloned().collect();
    rewrite_links(current.requirements.values_mut(), &map);
    rewrite_links(current.hazards.values_mut(), &map);
    rewrite_links(current.safety_functions.values_mut(), &map);
    rewrite_links(current.safety_requirements.values_mut(), &map);

    current.next_id = next_id;
    current.next_haz_id = next_haz;
    current.next_sf_id = next_sf;
    current.next_sr_id = next_sr;
    storage::save(&path, &current)?;
    println!(
        "Renumbered {} artifact(s) and re-signed {}.",
        all_renames.len(),
        path.display()
    );
    Ok(())
}

/// REQ-0159: collisions are detected per artifact family (REQ/HAZ/SF/SR).
/// Same ID in both current and base but content differs — i.e. the ID was
/// reused on our side for a different artifact than base's.
fn plan_renames<T: Artifact>(
    current: &BTreeMap<String, T>,
    base: &BTreeMap<String, T>,
    next: &mut u32,
    prefix: &str,
) -> Vec<(String, String)> {
    let mut renames = Vec::new();
    let mut ids: Vec<&String> = current.keys().collect();
    ids.sort();
    for id in ids {
        let (Some(a), Some(b)) = (current.get(id), base.get(id)) else {
            continue;
        };
        let differs = a.created() != b.created() || a.title() != b.title();
        if differs {
            let new_id = format!("{}-{:04}", prefix, *next);
            *next += 1;
            renames.push((id.clone(), new_id));
        }
    }
    renames
}

fn rekey<T: Artifact>(map: &mut BTreeMap<String, T>, renames: &[(String, String)]) {
    for (old, new) in renames {
        if let Some(mut a) = map.remove(old) {
            a.set_id(new.clone());
            a.note_renamed(old);
            map.insert(new.clone(), a);
        }
    }
}

/// REQ-0159: rewrite every link target through the combined rename map so a
/// renamed hazard/SF is followed by its inbound mitigates/realizes links.
fn rewrite_links<'a, T: Artifact + 'a>(
    artifacts: impl Iterator<Item = &'a mut T>,
    map: &HashMap<String, String>,
) {
    for a in artifacts {
        for link in a.links_mut().iter_mut() {
            if let Some(new) = map.get(&link.target) {
                link.target = new.clone();
            }
        }
    }
}

fn load_from_git_ref(reference: &str, filename: &str) -> Result<Project> {
    let spec = format!("{}:{}", reference, filename);
    let output = Command::new("git")
        .args(["show", &spec])
        .output()
        .with_context(|| format!("run git show {}", spec))?;
    if !output.status.success() {
        return Err(anyhow!(
            "git show {} failed: {}",
            spec,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let tmp = std::env::temp_dir().join(format!("req-base-{}.req", std::process::id()));
    std::fs::write(&tmp, &output.stdout)?;
    let project = load_with_options(&tmp, true)?;
    std::fs::remove_file(&tmp).ok();
    Ok(project)
}
