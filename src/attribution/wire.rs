use std::collections::BTreeMap;

use regex::Regex;
use serde::Serialize;

use super::types::{ConnectionAttribution, WorkloadAttribution, WorkloadKind};

impl ConnectionAttribution {
    pub(super) fn wire_fields(&self) -> BTreeMap<String, String> {
        let mut fields = BTreeMap::new();
        insert_enum(&mut fields, "interactionKind", self.interaction_kind);
        insert_bounded(
            &mut fields,
            "agentFramework",
            self.agent_framework.as_deref(),
            128,
        );
        insert_version(
            &mut fields,
            "agentFrameworkVersion",
            self.agent_framework_version.as_deref(),
        );
        insert_surface(&mut fields, "agentSurface", self.agent_surface.as_deref());
        insert_version(
            &mut fields,
            "agentSurfaceVersion",
            self.agent_surface_version.as_deref(),
        );
        insert_enum(&mut fields, "executionMode", self.execution_mode);
        fields
    }
}

impl WorkloadAttribution {
    pub(crate) fn wire_fields(&self) -> BTreeMap<String, String> {
        let mut fields = BTreeMap::new();
        insert_enum(&mut fields, "workloadKind", self.workload_kind);
        insert_bounded(
            &mut fields,
            "agentFramework",
            self.agent_framework.as_deref(),
            128,
        );
        insert_version(
            &mut fields,
            "agentFrameworkVersion",
            self.agent_framework_version.as_deref(),
        );
        insert_surface(&mut fields, "agentSurface", self.agent_surface.as_deref());
        insert_version(
            &mut fields,
            "agentSurfaceVersion",
            self.agent_surface_version.as_deref(),
        );
        insert_enum(&mut fields, "executionMode", self.execution_mode);
        insert_enum(&mut fields, "operationScope", self.operation_scope);
        insert_operation(&mut fields, "operationId", self.operation_id.as_deref());
        insert_operation(
            &mut fields,
            "rootOperationId",
            self.root_operation_id.as_deref(),
        );
        insert_operation(
            &mut fields,
            "parentOperationId",
            self.parent_operation_id.as_deref(),
        );
        fields
    }

    pub(super) fn sanitize(&mut self) {
        self.agent_framework = self
            .agent_framework
            .as_deref()
            .and_then(|value| bounded(value, 128));
        self.agent_framework_version = self
            .agent_framework_version
            .as_deref()
            .and_then(valid_version);
        self.agent_surface = self.agent_surface.as_deref().and_then(valid_surface);
        self.agent_surface_version = self
            .agent_surface_version
            .as_deref()
            .and_then(valid_version);
        self.operation_id = self.operation_id.as_deref().and_then(valid_operation_id);
        self.root_operation_id = self
            .root_operation_id
            .as_deref()
            .and_then(valid_operation_id);
        self.parent_operation_id = self
            .parent_operation_id
            .as_deref()
            .and_then(valid_operation_id);
        if self.workload_kind != Some(WorkloadKind::AgentMediated) {
            self.agent_framework = None;
            self.agent_framework_version = None;
        }
    }
}

pub(super) fn enum_value<T: Serialize>(value: T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .unwrap_or_default()
}

pub(super) fn bounded(value: &str, maximum: usize) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()
        && value.len() <= maximum
        && !value.chars().any(|character| character.is_control()))
    .then(|| value.to_owned())
}

fn valid_version(value: &str) -> Option<String> {
    let value = bounded(value, 32)?;
    Regex::new(r"^[0-9][0-9A-Za-z.+_-]*$")
        .expect("static version regex")
        .is_match(&value)
        .then_some(value)
}

pub(super) fn valid_operation_id(value: &str) -> Option<String> {
    if value.trim() != value {
        return None;
    }
    let value = bounded(value, 128)?;
    Regex::new(r"^[A-Za-z0-9][A-Za-z0-9._:-]*$")
        .expect("static operation id regex")
        .is_match(&value)
        .then_some(value)
}

fn valid_surface(value: &str) -> Option<String> {
    const ALLOWED: &[&str] = &[
        "native_web",
        "native_mobile",
        "native_desktop",
        "plugin",
        "personal_skill",
        "mcp",
        "cli",
        "sdk",
        "openai_compatible",
        "direct_api",
        "unknown",
    ];
    let value = value.trim();
    ALLOWED.contains(&value).then(|| value.to_owned())
}

fn insert_bounded(
    fields: &mut BTreeMap<String, String>,
    name: &str,
    value: Option<&str>,
    maximum: usize,
) {
    if let Some(value) = value.and_then(|value| bounded(value, maximum)) {
        fields.insert(name.into(), value);
    }
}

fn insert_version(fields: &mut BTreeMap<String, String>, name: &str, value: Option<&str>) {
    if let Some(value) = value.and_then(valid_version) {
        fields.insert(name.into(), value);
    }
}

fn insert_surface(fields: &mut BTreeMap<String, String>, name: &str, value: Option<&str>) {
    if let Some(value) = value.and_then(valid_surface) {
        fields.insert(name.into(), value);
    }
}

fn insert_operation(fields: &mut BTreeMap<String, String>, name: &str, value: Option<&str>) {
    if let Some(value) = value.and_then(valid_operation_id) {
        fields.insert(name.into(), value);
    }
}

fn insert_enum<T: Serialize + Copy>(
    fields: &mut BTreeMap<String, String>,
    name: &str,
    value: Option<T>,
) {
    if let Some(value) = value {
        fields.insert(name.into(), enum_value(value));
    }
}
