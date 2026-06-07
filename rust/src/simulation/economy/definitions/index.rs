//! Export cache index generation for authored economy projects.

use super::schema::{EconomyProject, ResourcePort};
use serde::Serialize;

#[derive(Serialize)]
pub(super) struct CompiledEconomyIndex {
    pub(super) profile_ids: Vec<String>,
    pub(super) controller_ids: Vec<String>,
    pub(super) scenario_ids: Vec<String>,
    pub(super) compatibility: Vec<CompiledCompatibility>,
}

#[derive(Serialize)]
pub(super) struct CompiledCompatibility {
    pub(super) resource: String,
    pub(super) source_profile_id: String,
    pub(super) target_profile_ids: Vec<String>,
}

pub(super) fn build_index(project: &EconomyProject) -> CompiledEconomyIndex {
    let mut compatibility = Vec::new();
    for source in &project.profiles {
        for output in &source.outputs {
            let mut targets = Vec::new();
            for target in &project.profiles {
                if port_exists(&target.inputs, output.resource.as_str()) {
                    targets.push(target.id.clone());
                }
            }
            if !targets.is_empty() {
                compatibility.push(CompiledCompatibility {
                    resource: output.resource.clone(),
                    source_profile_id: source.id.clone(),
                    target_profile_ids: targets,
                });
            }
        }
    }

    CompiledEconomyIndex {
        profile_ids: project
            .profiles
            .iter()
            .map(|profile| profile.id.clone())
            .collect(),
        controller_ids: project
            .controllers
            .iter()
            .map(|controller| controller.id.clone())
            .collect(),
        scenario_ids: project
            .scenarios
            .iter()
            .map(|scenario| scenario.id.clone())
            .collect(),
        compatibility,
    }
}

fn port_exists(ports: &[ResourcePort], resource: &str) -> bool {
    ports.iter().any(|port| port.resource == resource)
}
