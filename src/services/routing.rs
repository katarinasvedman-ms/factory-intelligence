use crate::domain::{DataClassification, RequestedTarget, RoutingDecision, Scenario, TargetType};
use anyhow::{Result, bail};

#[derive(Clone, Default)]
pub struct RoutingPolicy;

impl RoutingPolicy {
    pub fn select(
        &self,
        scenario: &Scenario,
        requested: RequestedTarget,
    ) -> Result<RoutingDecision> {
        if requested != RequestedTarget::Auto {
            let selected = match requested {
                RequestedTarget::Device => TargetType::Device,
                RequestedTarget::Edge => TargetType::Edge,
                RequestedTarget::Cloud => TargetType::Cloud,
                RequestedTarget::Auto => unreachable!(),
            };
            if selected == TargetType::Cloud
                && scenario.data_classification == DataClassification::LocalOnly
            {
                bail!("cloud routing is prohibited for local_only scenarios");
            }
            return Ok(RoutingDecision {
                requested_target: requested,
                selected_target: selected,
                rule_id: "operator-selection".into(),
                reason: format!("Operator selected {selected:?}").to_lowercase(),
            });
        }

        let (selected_target, rule_id, reason) =
            if scenario.data_classification == DataClassification::LocalOnly {
                (
                    if scenario.requires_shared_context {
                        TargetType::Edge
                    } else {
                        TargetType::Device
                    },
                    "local-only-boundary",
                    "Data classification prohibits cloud processing",
                )
            } else if scenario.recommended_target == RequestedTarget::Cloud {
                (
                    TargetType::Cloud,
                    "approved-fleet-analysis",
                    "Scenario is approved for cloud management analysis",
                )
            } else if scenario.requires_shared_context {
                (
                    TargetType::Edge,
                    "requires-shared-context",
                    "Scenario requires factory-level shared context",
                )
            } else {
                (
                    TargetType::Device,
                    "single-machine-alarm",
                    "Routine single-machine work stays on the device",
                )
            };

        Ok(RoutingDecision {
            requested_target: requested,
            selected_target,
            rule_id: rule_id.into(),
            reason: reason.into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scenario(classification: DataClassification, shared: bool) -> Scenario {
        Scenario {
            id: "test".into(),
            title: "Test".into(),
            description: "Test".into(),
            default_prompt: "Test".into(),
            recommended_target: RequestedTarget::Device,
            data_classification: classification,
            requires_fast_slow: false,
            requires_shared_context: shared,
            event_id: None,
        }
    }

    #[test]
    fn routes_local_shared_context_to_edge() {
        let decision = RoutingPolicy
            .select(
                &scenario(DataClassification::LocalOnly, true),
                RequestedTarget::Auto,
            )
            .unwrap();
        assert_eq!(decision.selected_target, TargetType::Edge);
    }

    #[test]
    fn blocks_local_only_cloud_selection() {
        assert!(
            RoutingPolicy
                .select(
                    &scenario(DataClassification::LocalOnly, false),
                    RequestedTarget::Cloud,
                )
                .is_err()
        );
    }
}
