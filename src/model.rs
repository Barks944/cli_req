// Implements REQ-0010 (sequential IDs via allocate_id) and the data shape
// behind REQ-0011 (append-only history).
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::cli::{KindArg, LinkKindArg, PriorityArg, StatusArg};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub name: String,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
    pub next_id: u32,
    pub requirements: BTreeMap<String, Requirement>,
}

impl Project {
    pub fn new(name: String) -> Self {
        let now = Utc::now();
        Self {
            name,
            created: now,
            updated: now,
            next_id: 1,
            requirements: BTreeMap::new(),
        }
    }

    pub fn allocate_id(&mut self) -> String {
        let id = format!("REQ-{:04}", self.next_id);
        self.next_id += 1;
        id
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Requirement {
    pub id: String,
    pub title: String,
    pub statement: String,
    pub rationale: String,
    pub acceptance: Vec<String>,
    pub kind: Kind,
    pub priority: Priority,
    pub status: Status,
    pub tags: Vec<String>,
    pub links: Vec<Link>,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
    pub history: Vec<HistoryEntry>,
    /// Test records (REQ-0049 / REQ-0050). Defaults to empty so older files
    /// load forward-compatibly.
    #[serde(default)]
    pub tests: Vec<TestRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestRecord {
    pub at: DateTime<Utc>,
    pub actor: String,
    pub commit: String,
    pub outcome: TestOutcome,
    pub notes: String,
}

#[derive(Debug, Copy, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TestOutcome {
    Pass,
    Fail,
}

impl TestOutcome {
    pub fn as_str(&self) -> &'static str {
        match self { TestOutcome::Pass => "pass", TestOutcome::Fail => "fail" }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Link {
    pub kind: LinkKind,
    pub target: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub at: DateTime<Utc>,
    pub actor: String,
    /// Implements REQ-0043: human vs agent vs unknown. Defaults to Unknown so
    /// older files (where the field is absent) load forward-compatibly.
    #[serde(default = "ActorKind::unknown")]
    pub actor_kind: ActorKind,
    pub action: String,
    pub reason: Option<String>,
}

#[derive(Debug, Copy, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ActorKind {
    Human,
    Agent,
    Unknown,
}

impl ActorKind {
    pub fn unknown() -> Self { ActorKind::Unknown }
    pub fn as_str(&self) -> &'static str {
        match self {
            ActorKind::Human => "human",
            ActorKind::Agent => "agent",
            ActorKind::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Copy, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Kind {
    Functional,
    NonFunctional,
    Constraint,
    Interface,
    Business,
}

#[derive(Debug, Copy, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Priority {
    Must,
    Should,
    Could,
    Wont,
}

#[derive(Debug, Copy, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Status {
    Draft,
    Proposed,
    Approved,
    Implemented,
    Verified,
    Obsolete,
}

#[derive(Debug, Copy, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum LinkKind {
    Parent,
    DependsOn,
    Conflicts,
    Refines,
    Verifies,
}

impl From<KindArg> for Kind {
    fn from(k: KindArg) -> Self {
        match k {
            KindArg::Functional => Kind::Functional,
            KindArg::NonFunctional => Kind::NonFunctional,
            KindArg::Constraint => Kind::Constraint,
            KindArg::Interface => Kind::Interface,
            KindArg::Business => Kind::Business,
        }
    }
}

impl From<PriorityArg> for Priority {
    fn from(p: PriorityArg) -> Self {
        match p {
            PriorityArg::Must => Priority::Must,
            PriorityArg::Should => Priority::Should,
            PriorityArg::Could => Priority::Could,
            PriorityArg::Wont => Priority::Wont,
        }
    }
}

impl From<StatusArg> for Status {
    fn from(s: StatusArg) -> Self {
        match s {
            StatusArg::Draft => Status::Draft,
            StatusArg::Proposed => Status::Proposed,
            StatusArg::Approved => Status::Approved,
            StatusArg::Implemented => Status::Implemented,
            StatusArg::Verified => Status::Verified,
            StatusArg::Obsolete => Status::Obsolete,
        }
    }
}

impl From<LinkKindArg> for LinkKind {
    fn from(l: LinkKindArg) -> Self {
        match l {
            LinkKindArg::Parent => LinkKind::Parent,
            LinkKindArg::DependsOn => LinkKind::DependsOn,
            LinkKindArg::Conflicts => LinkKind::Conflicts,
            LinkKindArg::Refines => LinkKind::Refines,
            LinkKindArg::Verifies => LinkKind::Verifies,
        }
    }
}

impl Kind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Kind::Functional => "functional",
            Kind::NonFunctional => "non-functional",
            Kind::Constraint => "constraint",
            Kind::Interface => "interface",
            Kind::Business => "business",
        }
    }
}

impl Priority {
    pub fn as_str(&self) -> &'static str {
        match self {
            Priority::Must => "must",
            Priority::Should => "should",
            Priority::Could => "could",
            Priority::Wont => "wont",
        }
    }
}

impl Status {
    pub fn as_str(&self) -> &'static str {
        match self {
            Status::Draft => "draft",
            Status::Proposed => "proposed",
            Status::Approved => "approved",
            Status::Implemented => "implemented",
            Status::Verified => "verified",
            Status::Obsolete => "obsolete",
        }
    }
}

impl LinkKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            LinkKind::Parent => "parent",
            LinkKind::DependsOn => "depends-on",
            LinkKind::Conflicts => "conflicts",
            LinkKind::Refines => "refines",
            LinkKind::Verifies => "verifies",
        }
    }
}
