// SPDX-License-Identifier: Apache-2.0

mod adapter;
mod credential;
mod transport;

pub mod error;
pub mod integration;
pub mod protocol;
pub mod worker;

pub use error::{ProviderError, ProviderErrorKind};
pub use integration::{JsonLineProviderIntegration, ProviderWorkerIntegration};
pub use protocol::{
    ApiBilling, ConsumerSubscription, FinishReason, InferenceRequest, InferenceResponse, Message,
    ProviderCapabilities, ProviderId, ProviderVerification, Role, TokenUsage, WorkerCommand,
    WorkerEvent, PROTOCOL_VERSION,
};
