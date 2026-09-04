use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InteractionKind {
    HumanUi,
    ExternalAgent,
    Service,
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkloadKind {
    Direct,
    AgentMediated,
    Service,
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OperationScope {
    TopLevel,
    Child,
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    Browser,
    Durable,
    Server,
    Unknown,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionAttribution {
    pub interaction_kind: Option<InteractionKind>,
    pub agent_framework: Option<String>,
    pub agent_framework_version: Option<String>,
    pub agent_surface: Option<String>,
    pub agent_surface_version: Option<String>,
    pub execution_mode: Option<ExecutionMode>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkloadAttribution {
    pub workload_kind: Option<WorkloadKind>,
    pub agent_framework: Option<String>,
    pub agent_framework_version: Option<String>,
    pub agent_surface: Option<String>,
    pub agent_surface_version: Option<String>,
    pub execution_mode: Option<ExecutionMode>,
    pub operation_scope: Option<OperationScope>,
    pub operation_id: Option<String>,
    pub root_operation_id: Option<String>,
    pub parent_operation_id: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct Attribution {
    pub connection: Option<ConnectionAttribution>,
    pub workload: Option<WorkloadAttribution>,
}
