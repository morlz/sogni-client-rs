//! Validated connection- and workload-attribution metadata.

mod resolve;
mod types;
mod wire;

pub use types::{
    Attribution, ConnectionAttribution, ExecutionMode, InteractionKind, OperationScope,
    WorkloadAttribution, WorkloadKind,
};
