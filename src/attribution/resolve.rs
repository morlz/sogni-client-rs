use std::collections::BTreeMap;

use super::{
    types::{Attribution, ConnectionAttribution, OperationScope, WorkloadAttribution},
    wire::{bounded, valid_operation_id},
};

impl Attribution {
    pub(crate) fn connection_query(&self) -> Vec<(String, String)> {
        self.connection
            .as_ref()
            .map(ConnectionAttribution::wire_fields)
            .unwrap_or_default()
            .into_iter()
            .collect()
    }

    pub(crate) fn resolve_workload(
        &self,
        override_value: Option<&WorkloadAttribution>,
        fallback_operation_id: Option<&str>,
    ) -> Option<WorkloadAttribution> {
        let mut resolved = self.workload.clone().unwrap_or_default();
        if let Some(override_value) = override_value {
            if override_value.agent_framework != resolved.agent_framework
                && override_value.agent_framework.is_some()
                && override_value.agent_framework_version.is_none()
            {
                resolved.agent_framework_version = None;
            }
            if override_value.agent_surface != resolved.agent_surface
                && override_value.agent_surface.is_some()
                && override_value.agent_surface_version.is_none()
            {
                resolved.agent_surface_version = None;
            }
            macro_rules! replace_some {
                ($field:ident) => {
                    if override_value.$field.is_some() {
                        resolved.$field.clone_from(&override_value.$field);
                    }
                };
            }
            replace_some!(workload_kind);
            replace_some!(agent_framework);
            replace_some!(agent_framework_version);
            replace_some!(agent_surface);
            replace_some!(agent_surface_version);
            replace_some!(execution_mode);
            replace_some!(operation_scope);
            replace_some!(operation_id);
            replace_some!(root_operation_id);
            replace_some!(parent_operation_id);
        }
        resolved.sanitize();
        // An absent or invalid attribution must not become attributed traffic
        // merely because the transport has allocated a request identifier.
        if resolved.wire_fields().is_empty() {
            return None;
        }
        if resolved.operation_id.is_none() {
            resolved.operation_id = fallback_operation_id.and_then(valid_operation_id);
        }
        if resolved.operation_id.is_some() && resolved.operation_scope.is_none() {
            resolved.operation_scope = Some(
                if resolved.parent_operation_id.is_some()
                    || resolved.root_operation_id.as_ref() != resolved.operation_id.as_ref()
                        && resolved.root_operation_id.is_some()
                {
                    OperationScope::Child
                } else {
                    OperationScope::TopLevel
                },
            );
        }
        match resolved.operation_scope {
            Some(OperationScope::Child) => {
                if resolved.parent_operation_id.is_none() {
                    resolved
                        .parent_operation_id
                        .clone_from(&resolved.root_operation_id);
                }
            }
            Some(OperationScope::TopLevel) => {
                resolved
                    .root_operation_id
                    .clone_from(&resolved.operation_id);
                resolved.parent_operation_id = None;
            }
            _ => {}
        }
        (!resolved.wire_fields().is_empty()).then_some(resolved)
    }

    pub(crate) fn headers(
        &self,
        app_source: Option<&str>,
        workload: Option<&WorkloadAttribution>,
    ) -> BTreeMap<String, String> {
        let mut headers = BTreeMap::new();
        if let Some(value) = app_source.and_then(|value| bounded(value, 128)) {
            headers.insert("X-App-Source".into(), value);
        }
        if let Some(connection) = &self.connection {
            if let Some(value) = connection.interaction_kind {
                headers.insert(
                    "X-Sogni-Interaction-Kind".into(),
                    super::wire::enum_value(value),
                );
            }
        }
        if let Some(workload) = workload {
            let names = [
                ("workloadKind", "X-Sogni-Workload-Kind"),
                ("agentFramework", "X-Sogni-Agent-Framework"),
                ("agentFrameworkVersion", "X-Sogni-Agent-Framework-Version"),
                ("agentSurface", "X-Sogni-Agent-Surface"),
                ("agentSurfaceVersion", "X-Sogni-Agent-Surface-Version"),
                ("executionMode", "X-Sogni-Execution-Mode"),
                ("operationScope", "X-Sogni-Operation-Scope"),
                ("operationId", "X-Sogni-Operation-Id"),
                ("rootOperationId", "X-Sogni-Root-Operation-Id"),
                ("parentOperationId", "X-Sogni-Parent-Operation-Id"),
            ];
            let fields = workload.wire_fields();
            for (field, header) in names {
                if let Some(value) = fields.get(field) {
                    headers.insert(header.into(), value.clone());
                }
            }
        }
        headers
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ExecutionMode, WorkloadKind};

    #[test]
    fn missing_or_empty_attribution_does_not_invent_request_lineage() {
        let empty = Attribution::default();
        assert!(empty.resolve_workload(None, Some("request-123")).is_none());
        assert!(
            empty
                .resolve_workload(Some(&WorkloadAttribution::default()), Some("request-123"))
                .is_none()
        );
        let invalid = WorkloadAttribution {
            operation_id: Some(" ".into()),
            ..WorkloadAttribution::default()
        };
        assert!(
            empty
                .resolve_workload(Some(&invalid), Some("request-123"))
                .is_none()
        );
    }

    #[test]
    fn explicit_workload_metadata_and_lineage_remain_intact() {
        let defaults = Attribution {
            workload: Some(WorkloadAttribution {
                workload_kind: Some(WorkloadKind::Service),
                execution_mode: Some(ExecutionMode::Server),
                ..WorkloadAttribution::default()
            }),
            ..Attribution::default()
        };
        let input = WorkloadAttribution {
            operation_scope: Some(OperationScope::Child),
            operation_id: Some("child-123".into()),
            root_operation_id: Some("root-123".into()),
            parent_operation_id: Some("parent-123".into()),
            ..WorkloadAttribution::default()
        };
        let actual = defaults
            .resolve_workload(Some(&input), Some("unused-fallback"))
            .unwrap();
        assert_eq!(actual.workload_kind, Some(WorkloadKind::Service));
        assert_eq!(actual.execution_mode, Some(ExecutionMode::Server));
        assert_eq!(actual.operation_scope, input.operation_scope);
        assert_eq!(actual.operation_id, input.operation_id);
        assert_eq!(actual.root_operation_id, input.root_operation_id);
        assert_eq!(actual.parent_operation_id, input.parent_operation_id);
    }

    #[test]
    fn explicit_framework_without_workload_kind_keeps_metadata_and_lineage() {
        let input = WorkloadAttribution {
            agent_framework: Some("fixture-framework".into()),
            agent_framework_version: Some("1.2.3".into()),
            ..WorkloadAttribution::default()
        };
        let actual = Attribution::default()
            .resolve_workload(Some(&input), Some("request-123"))
            .unwrap();
        assert_eq!(actual.workload_kind, None);
        assert_eq!(actual.agent_framework, input.agent_framework);
        assert_eq!(
            actual.agent_framework_version,
            input.agent_framework_version
        );
        assert_eq!(actual.operation_scope, Some(OperationScope::TopLevel));
        assert_eq!(actual.operation_id.as_deref(), Some("request-123"));
        assert_eq!(actual.root_operation_id.as_deref(), Some("request-123"));
    }
}
