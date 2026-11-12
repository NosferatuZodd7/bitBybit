//! `audio.restartServices`: stops Windows Audio and starts both audio services again, once (no retry loop).
//! Changes no configuration, so nothing is journaled; the plan warns that sound stops for a few seconds.

use bitbybit_core::{Result, RiskLevel, text};
use bitbybit_domain::snapshot::ServiceSummary;
use bitbybit_ipc::operation::{BrokerOperation, CallContext, Category, Descriptor, RiskTier};
use bitbybit_ipc::plan::OperationPlan;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;

/// Windows: `platform-windows::diagnostics::restart_audio_services`.
pub trait AudioServices: Send + Sync {
    fn restart(&self) -> Result<Vec<ServiceSummary>>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServicesAfter {
    pub services: Vec<ServiceSummary>,
}

pub struct RestartAudioServicesOperation {
    descriptor: Descriptor,
    services: Arc<dyn AudioServices>,
}

impl RestartAudioServicesOperation {
    pub fn new(services: Arc<dyn AudioServices>) -> Self {
        Self {
            descriptor: Descriptor {
                name: "audio.restartServices",
                tier: RiskTier::Elevated,
                category: Category::AUDIO,
                requires_confirmation: true,
                timeout: Some(Duration::from_secs(120)),
            },
            services,
        }
    }
}

impl BrokerOperation for RestartAudioServicesOperation {
    type Request = crate::NoParameters;
    type Response = ServicesAfter;
    fn descriptor(&self) -> &Descriptor {
        &self.descriptor
    }
    fn validate(&self, _: &crate::NoParameters) -> Result<()> {
        Ok(())
    }
    fn plan(&self, _: &crate::NoParameters, _: &CallContext) -> Result<Option<OperationPlan>> {
        let mut plan = OperationPlan::new(
            text!("audio.plan.restartServices", "Restart the Windows audio services (Audiosrv, AudioEndpointBuilder)"),
            vec![text!(
                "audio.plan.restartServicesImpact",
                "Sound stops for a few seconds; applications playing audio may need to be restarted."
            )],
            RiskLevel::Medium,
        );
        plan.may_interrupt_audio = true;
        Ok(Some(plan))
    }
    fn execute(&self, _: &crate::NoParameters, _: &CallContext) -> Result<ServicesAfter> {
        Ok(ServicesAfter { services: self.services.restart()? })
    }
}
