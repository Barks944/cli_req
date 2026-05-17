// Implements REQ-0078: publish JSON Schemas for structured CLI inputs.
// Schema version is aligned with the project.req format tag so agents can
// pin a schema to a tool version.
use anyhow::Result;
use serde_json::{json, Value};

use crate::cli::{SchemaArgs, SchemaWhich};
use crate::storage::FORMAT_TAG;

pub fn run(args: SchemaArgs) -> Result<()> {
    let schema = match args.which {
        SchemaWhich::Add => add_schema(),
        SchemaWhich::Batch => batch_schema(),
        SchemaWhich::Import => import_schema(),
    };
    println!("{}", serde_json::to_string_pretty(&schema)?);
    Ok(())
}

fn id_url(name: &str) -> String {
    format!("https://github.com/Barks944/cli_req/schema/{}/{}.json", FORMAT_TAG, name)
}

fn requirement_props() -> Value {
    json!({
        "title":      { "type": "string", "minLength": 5, "maxLength": 120 },
        "statement":  { "type": "string", "minLength": 1 },
        "rationale":  { "type": "string", "minLength": 1 },
        "kind":       { "type": "string", "enum": ["functional","non-functional","constraint","interface","business"] },
        "priority":   { "type": "string", "enum": ["must","should","could","wont"] },
        "acceptance": { "type": "array", "items": { "type": "string" } },
        "tags":       { "type": "array", "items": { "type": "string" } },
        "parent":     { "type": "string", "pattern": "^REQ-\\d{4}$" }
    })
}

fn add_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": id_url("add"),
        "title": "req add --from-json input",
        "description": "Structured input for creating a single requirement via the req CLI.",
        "type": "object",
        "required": ["title", "statement", "rationale"],
        "additionalProperties": false,
        "properties": requirement_props(),
        "_format": FORMAT_TAG
    })
}

fn batch_schema() -> Value {
    let req = requirement_props();
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": id_url("batch"),
        "title": "req batch input",
        "description": "Transactional batch of mutations against a single project.req.",
        "type": "object",
        "required": ["mutations"],
        "additionalProperties": false,
        "properties": {
            "reason": { "type": "string", "description": "Default reason applied to each mutation that omits its own." },
            "mutations": {
                "type": "array",
                "items": {
                    "oneOf": [
                        {
                            "type": "object",
                            "required": ["kind","title","statement","rationale"],
                            "properties": {
                                "kind": { "const": "add" },
                                "title": req["title"].clone(),
                                "statement": req["statement"].clone(),
                                "rationale": req["rationale"].clone(),
                                "req_kind": req["kind"].clone(),
                                "priority": req["priority"].clone(),
                                "acceptance": req["acceptance"].clone(),
                                "tags": req["tags"].clone(),
                                "parent": req["parent"].clone(),
                                "reason": { "type": "string" }
                            }
                        },
                        {
                            "type": "object",
                            "required": ["kind","id"],
                            "properties": {
                                "kind": { "const": "update" },
                                "id": { "type": "string", "pattern": "^REQ-\\d{4}$" },
                                "title": { "type": "string" },
                                "statement": { "type": "string" },
                                "rationale": { "type": "string" },
                                "acceptance": { "type": "array", "items": { "type": "string" } },
                                "add_acceptance": { "type": "array", "items": { "type": "string" } },
                                "req_kind": req["kind"].clone(),
                                "priority": req["priority"].clone(),
                                "status": { "type": "string", "enum": ["draft","proposed","approved","implemented","verified","obsolete"] },
                                "add_tag": { "type": "array", "items": { "type": "string" } },
                                "remove_tag": { "type": "array", "items": { "type": "string" } },
                                "reason": { "type": "string" }
                            }
                        },
                        {
                            "type": "object",
                            "required": ["kind","id"],
                            "properties": {
                                "kind": { "const": "delete" },
                                "id": { "type": "string", "pattern": "^REQ-\\d{4}$" },
                                "hard": { "type": "boolean" },
                                "reason": { "type": "string" }
                            }
                        },
                        {
                            "type": "object",
                            "required": ["kind","from","to"],
                            "properties": {
                                "kind": { "const": "link" },
                                "from": { "type": "string", "pattern": "^REQ-\\d{4}$" },
                                "to": { "type": "string", "pattern": "^REQ-\\d{4}$" },
                                "link_kind": { "type": "string", "enum": ["parent","depends_on","conflicts","refines","verifies"] },
                                "remove": { "type": "boolean" },
                                "reason": { "type": "string" }
                            }
                        }
                    ]
                }
            }
        },
        "_format": FORMAT_TAG
    })
}

fn import_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": id_url("import"),
        "title": "req import --format json input",
        "description": "Flat array of candidate requirements for bulk import.",
        "type": "array",
        "items": {
            "type": "object",
            "required": ["title", "statement"],
            "additionalProperties": false,
            "properties": requirement_props()
        },
        "_format": FORMAT_TAG
    })
}
