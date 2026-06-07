//! Validation for authored economy projects and runtime tuning.

use super::runtime::{MinuteWindow, OperationalClockRuntimeTuning, RuntimeEconomyTuning};
use super::runtime_compile::compile_runtime_catalog;
use super::schema::{
    EconomyController, EconomyProfile, EconomyProject, EconomyScenario, ResourcePort, ScenarioNode,
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Serialize)]
pub(super) struct ValidationMessage {
    pub(super) severity: &'static str,
    pub(super) code: &'static str,
    pub(super) scope: String,
    pub(super) message: String,
}

pub(super) fn validate_project(project: &EconomyProject) -> Vec<ValidationMessage> {
    let mut messages = Vec::new();
    let profile_ids = duplicate_ids(project.profiles.iter().map(|profile| profile.id.as_str()));
    for duplicate in profile_ids {
        messages.push(error(
            "duplicate_profile_id",
            "project.profiles",
            format!("profile id '{duplicate}' is defined more than once"),
        ));
    }

    let controller_ids = duplicate_ids(
        project
            .controllers
            .iter()
            .map(|controller| controller.id.as_str()),
    );
    for duplicate in controller_ids {
        messages.push(error(
            "duplicate_controller_id",
            "project.controllers",
            format!("controller id '{duplicate}' is defined more than once"),
        ));
    }

    let scenario_ids = duplicate_ids(
        project
            .scenarios
            .iter()
            .map(|scenario| scenario.id.as_str()),
    );
    for duplicate in scenario_ids {
        messages.push(error(
            "duplicate_scenario_id",
            "project.scenarios",
            format!("scenario id '{duplicate}' is defined more than once"),
        ));
    }

    let profile_map: BTreeMap<&str, &EconomyProfile> = project
        .profiles
        .iter()
        .map(|profile| (profile.id.as_str(), profile))
        .collect();
    let controller_map: BTreeMap<&str, &EconomyController> = project
        .controllers
        .iter()
        .map(|controller| (controller.id.as_str(), controller))
        .collect();

    for profile in &project.profiles {
        if profile.display_name.trim().is_empty() {
            messages.push(error(
                "missing_profile_display_name",
                format!("profile.{}", profile.id),
                "profile display_name must not be empty".to_owned(),
            ));
        }
        if profile.kind.trim().is_empty() {
            messages.push(error(
                "missing_profile_kind",
                format!("profile.{}", profile.id),
                "profile kind must not be empty".to_owned(),
            ));
        }
    }
    validate_runtime_tuning_messages(&project.runtime_tuning, &mut messages);

    for controller in &project.controllers {
        if controller.display_name.trim().is_empty() {
            messages.push(error(
                "missing_controller_display_name",
                format!("controller.{}", controller.id),
                "controller display_name must not be empty".to_owned(),
            ));
        }
        if controller.kind.trim().is_empty() {
            messages.push(error(
                "missing_controller_kind",
                format!("controller.{}", controller.id),
                "controller kind must not be empty".to_owned(),
            ));
        }
    }

    for scenario in &project.scenarios {
        validate_scenario(scenario, &profile_map, &controller_map, &mut messages);
    }

    if let Err(err) = compile_runtime_catalog(&project.profiles, &project.runtime_tuning) {
        messages.push(error("invalid_runtime_catalog", "project.profiles", err));
    }

    messages
}

fn validate_runtime_tuning_messages(
    tuning: &RuntimeEconomyTuning,
    messages: &mut Vec<ValidationMessage>,
) {
    if let Err(err) = validate_runtime_tuning(tuning) {
        messages.push(error(
            "invalid_runtime_tuning",
            "project.runtime_tuning",
            err,
        ));
    }
}

pub(super) fn validate_runtime_tuning(tuning: &RuntimeEconomyTuning) -> Result<(), String> {
    validate_range(
        tuning.operational_clock.seconds_per_day as f32,
        60.0,
        f32::INFINITY,
        "runtime_tuning.operational_clock.seconds_per_day",
    )?;
    if tuning.operational_clock.travel_estimate_refresh_minutes == 0 {
        return Err(
            "runtime_tuning.operational_clock.travel_estimate_refresh_minutes must be > 0"
                .to_owned(),
        );
    }
    if tuning
        .operational_clock
        .household_replenishment_check_interval_hours
        == 0
    {
        return Err(
            "runtime_tuning.operational_clock.household_replenishment_check_interval_hours must be > 0"
                .to_owned(),
        );
    }
    if tuning.operational_clock.household_pickup_eta_hours == 0 {
        return Err(
            "runtime_tuning.operational_clock.household_pickup_eta_hours must be > 0".to_owned(),
        );
    }
    if tuning
        .operational_clock
        .household_replenishment_retry_cooldown_hours
        == 0
    {
        return Err(
            "runtime_tuning.operational_clock.household_replenishment_retry_cooldown_hours must be > 0"
                .to_owned(),
        );
    }
    if tuning.operational_clock.shipment_retry_cooldown_hours == 0 {
        return Err(
            "runtime_tuning.operational_clock.shipment_retry_cooldown_hours must be > 0".to_owned(),
        );
    }
    validate_range(
        tuning.commercial_owa_utility_cost_per_day,
        0.0,
        f32::INFINITY,
        "runtime_tuning.commercial_owa_utility_cost_per_day",
    )?;
    validate_range(
        tuning.industrial_owa_utility_cost_per_day,
        0.0,
        f32::INFINITY,
        "runtime_tuning.industrial_owa_utility_cost_per_day",
    )?;
    validate_work_profiles(&tuning.operational_clock)?;
    validate_freight_profiles(&tuning.operational_clock)?;
    validate_range(
        tuning.households.immigrant_starting_stock_days,
        0.0,
        f32::INFINITY,
        "runtime_tuning.households.immigrant_starting_stock_days",
    )?;
    validate_range(
        tuning.households.immigrant_starting_budget_per_member,
        0.0,
        f32::INFINITY,
        "runtime_tuning.households.immigrant_starting_budget_per_member",
    )?;
    validate_range(
        tuning.households.household_starting_budget_floor,
        0.0,
        f32::INFINITY,
        "runtime_tuning.households.household_starting_budget_floor",
    )?;
    validate_range(
        tuning.households.utility_cost_per_member_per_day,
        0.0,
        f32::INFINITY,
        "runtime_tuning.households.utility_cost_per_member_per_day",
    )?;
    validate_nonempty_level_array(
        &tuning
            .households
            .residential_move_in_min_reserve_days_by_level,
        "runtime_tuning.households.residential_move_in_min_reserve_days_by_level",
        0.0,
        f32::INFINITY,
    )?;
    validate_nonempty_level_array(
        &tuning.households.residential_stay_min_reserve_days_by_level,
        "runtime_tuning.households.residential_stay_min_reserve_days_by_level",
        0.0,
        f32::INFINITY,
    )?;
    if tuning.households.stay_failure_days_before_eviction == 0 {
        return Err(
            "runtime_tuning.households.stay_failure_days_before_eviction must be > 0".to_owned(),
        );
    }
    validate_nonempty_level_array(
        &tuning.viability.residential_min_occupancy_ratio_for_upgrade,
        "runtime_tuning.viability.residential_min_occupancy_ratio_for_upgrade",
        0.0,
        1.0,
    )?;
    validate_nonempty_level_array(
        &tuning
            .viability
            .residential_max_occupancy_ratio_for_downgrade,
        "runtime_tuning.viability.residential_max_occupancy_ratio_for_downgrade",
        0.0,
        1.0,
    )?;
    validate_nonempty_level_array(
        &tuning.viability.nonresidential_min_buffer_days_by_level,
        "runtime_tuning.viability.nonresidential_min_buffer_days_by_level",
        0.0,
        f32::INFINITY,
    )?;
    validate_nonempty_level_array(
        &tuning
            .viability
            .nonresidential_max_buffer_days_for_downgrade,
        "runtime_tuning.viability.nonresidential_max_buffer_days_for_downgrade",
        0.0,
        f32::INFINITY,
    )?;
    validate_range(
        tuning
            .viability
            .nonresidential_min_staffing_ratio_for_upgrade,
        0.0,
        1.0,
        "runtime_tuning.viability.nonresidential_min_staffing_ratio_for_upgrade",
    )?;
    validate_range(
        tuning
            .viability
            .nonresidential_max_staffing_ratio_for_downgrade,
        0.0,
        1.0,
        "runtime_tuning.viability.nonresidential_max_staffing_ratio_for_downgrade",
    )?;
    validate_range(
        tuning.viability.industrial_min_input_coverage_for_upgrade,
        0.0,
        1.0,
        "runtime_tuning.viability.industrial_min_input_coverage_for_upgrade",
    )?;
    validate_range(
        tuning.viability.industrial_min_output_headroom_for_upgrade,
        0.0,
        1.0,
        "runtime_tuning.viability.industrial_min_output_headroom_for_upgrade",
    )?;
    validate_range(
        tuning.viability.industrial_max_input_coverage_for_downgrade,
        0.0,
        1.0,
        "runtime_tuning.viability.industrial_max_input_coverage_for_downgrade",
    )?;
    validate_range(
        tuning
            .viability
            .industrial_max_output_headroom_for_downgrade,
        0.0,
        1.0,
        "runtime_tuning.viability.industrial_max_output_headroom_for_downgrade",
    )?;
    validate_range(
        tuning.owa_import_price_multiplier,
        1.0,
        f32::INFINITY,
        "runtime_tuning.owa_import_price_multiplier",
    )?;
    validate_range(
        tuning.owa_export_price_multiplier,
        0.0,
        1.0,
        "runtime_tuning.owa_export_price_multiplier",
    )?;
    Ok(())
}

fn validate_work_profiles(clock: &OperationalClockRuntimeTuning) -> Result<(), String> {
    if clock.work_profiles.is_empty() {
        return Err("runtime_tuning.operational_clock.work_profiles must not be empty".to_owned());
    }
    let duplicates = duplicate_ids(
        clock
            .work_profiles
            .iter()
            .map(|profile| profile.id.as_str()),
    );
    if let Some(duplicate) = duplicates.into_iter().next() {
        return Err(format!(
            "runtime_tuning.operational_clock.work_profiles contains duplicate id '{duplicate}'"
        ));
    }
    for profile in &clock.work_profiles {
        if profile.id.trim().is_empty() {
            return Err(
                "runtime_tuning.operational_clock.work_profiles[*].id must not be empty".to_owned(),
            );
        }
        if profile.arrival_windows.is_empty() || profile.departure_windows.is_empty() {
            return Err(format!(
                "runtime_tuning.operational_clock.work_profiles.{} must define arrival_windows and departure_windows",
                profile.id
            ));
        }
        if profile.arrival_windows.len() != profile.departure_windows.len() {
            return Err(format!(
                "runtime_tuning.operational_clock.work_profiles.{} must use matching arrival/departure window counts",
                profile.id
            ));
        }
        if profile.reliability_buffer_minutes == 0 {
            return Err(format!(
                "runtime_tuning.operational_clock.work_profiles.{}.reliability_buffer_minutes must be > 0",
                profile.id
            ));
        }
        for (idx, window) in profile.arrival_windows.iter().enumerate() {
            validate_minute_window(
                *window,
                &format!(
                    "runtime_tuning.operational_clock.work_profiles.{}.arrival_windows[{idx}]",
                    profile.id
                ),
            )?;
        }
        for (idx, window) in profile.departure_windows.iter().enumerate() {
            validate_minute_window(
                *window,
                &format!(
                    "runtime_tuning.operational_clock.work_profiles.{}.departure_windows[{idx}]",
                    profile.id
                ),
            )?;
        }
    }
    for (zone_type, profile_id) in &clock.work_profile_by_zone_type {
        validate_zone_type_key(
            zone_type,
            "runtime_tuning.operational_clock.work_profile_by_zone_type",
        )?;
        if !clock
            .work_profiles
            .iter()
            .any(|profile| profile.id == *profile_id)
        {
            return Err(format!(
                "runtime_tuning.operational_clock.work_profile_by_zone_type.{zone_type} references missing profile '{profile_id}'"
            ));
        }
    }
    Ok(())
}

fn validate_freight_profiles(clock: &OperationalClockRuntimeTuning) -> Result<(), String> {
    if clock.freight_profiles.is_empty() {
        return Err(
            "runtime_tuning.operational_clock.freight_profiles must not be empty".to_owned(),
        );
    }
    let duplicates = duplicate_ids(
        clock
            .freight_profiles
            .iter()
            .map(|profile| profile.id.as_str()),
    );
    if let Some(duplicate) = duplicates.into_iter().next() {
        return Err(format!(
            "runtime_tuning.operational_clock.freight_profiles contains duplicate id '{duplicate}'"
        ));
    }
    for profile in &clock.freight_profiles {
        if profile.id.trim().is_empty() {
            return Err(
                "runtime_tuning.operational_clock.freight_profiles[*].id must not be empty"
                    .to_owned(),
            );
        }
        if profile.preferred_windows.is_empty() {
            return Err(format!(
                "runtime_tuning.operational_clock.freight_profiles.{} must define preferred_windows",
                profile.id
            ));
        }
        for (idx, window) in profile.preferred_windows.iter().enumerate() {
            validate_minute_window(
                *window,
                &format!(
                    "runtime_tuning.operational_clock.freight_profiles.{}.preferred_windows[{idx}]",
                    profile.id
                ),
            )?;
        }
        validate_range(
            profile.outside_window_cost_multiplier,
            1.0,
            10.0,
            &format!(
                "runtime_tuning.operational_clock.freight_profiles.{}.outside_window_cost_multiplier",
                profile.id
            ),
        )?;
    }
    for (zone_type, profile_id) in &clock.freight_profile_by_zone_type {
        validate_zone_type_key(
            zone_type,
            "runtime_tuning.operational_clock.freight_profile_by_zone_type",
        )?;
        if !clock
            .freight_profiles
            .iter()
            .any(|profile| profile.id == *profile_id)
        {
            return Err(format!(
                "runtime_tuning.operational_clock.freight_profile_by_zone_type.{zone_type} references missing profile '{profile_id}'"
            ));
        }
    }
    Ok(())
}

fn validate_minute_window(window: MinuteWindow, label: &str) -> Result<(), String> {
    let day_minutes: u16 = 24 * 60;
    if window.start_minute >= day_minutes
        || window.end_minute > day_minutes
        || window.start_minute >= window.end_minute
    {
        return Err(format!(
            "{label} must satisfy 0 <= start_minute < end_minute <= {}",
            day_minutes
        ));
    }
    Ok(())
}

fn validate_zone_type_key(zone_type: &str, label: &str) -> Result<(), String> {
    match zone_type {
        "commercial" | "industrial" | "residential" => Ok(()),
        other => Err(format!(
            "{label} contains unsupported zone_type key '{other}'"
        )),
    }
}

fn validate_nonempty_level_array(
    values: &[f32],
    label: &str,
    min_value: f32,
    max_value: f32,
) -> Result<(), String> {
    if values.is_empty() {
        return Err(format!("{label} must contain at least one level value"));
    }
    for (idx, value) in values.iter().copied().enumerate() {
        validate_range(value, min_value, max_value, &format!("{label}[{idx}]"))?;
    }
    Ok(())
}

fn validate_range(value: f32, min_value: f32, max_value: f32, label: &str) -> Result<(), String> {
    if !value.is_finite() || value < min_value || value > max_value {
        Err(format!(
            "{label} must be finite and in [{}..={}]",
            min_value, max_value
        ))
    } else {
        Ok(())
    }
}

fn validate_scenario(
    scenario: &EconomyScenario,
    profile_map: &BTreeMap<&str, &EconomyProfile>,
    controller_map: &BTreeMap<&str, &EconomyController>,
    messages: &mut Vec<ValidationMessage>,
) {
    if scenario.nodes.is_empty() {
        messages.push(error(
            "empty_scenario",
            format!("scenario.{}", scenario.id),
            "scenario has no graph nodes".to_owned(),
        ));
        return;
    }

    let duplicate_node_ids = duplicate_ids(scenario.nodes.iter().map(|node| node.id.as_str()));
    for duplicate in duplicate_node_ids {
        messages.push(error(
            "duplicate_scenario_node_id",
            format!("scenario.{}", scenario.id),
            format!("scenario node id '{duplicate}' is defined more than once"),
        ));
    }

    let node_map: BTreeMap<&str, &ScenarioNode> = scenario
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect();
    let mut demand_sink_count = 0u32;

    for node in &scenario.nodes {
        match node.ref_kind.as_str() {
            "profile" => {
                let Some(profile) = profile_map.get(node.ref_id.as_str()) else {
                    messages.push(error(
                        "missing_profile_ref",
                        format!("scenario.{}.node.{}", scenario.id, node.id),
                        format!("scenario node references missing profile '{}'", node.ref_id),
                    ));
                    continue;
                };
                if profile.kind == "demand_sink" {
                    demand_sink_count += 1;
                }
            }
            "controller" => {
                if !controller_map.contains_key(node.ref_id.as_str()) {
                    messages.push(error(
                        "missing_controller_ref",
                        format!("scenario.{}.node.{}", scenario.id, node.id),
                        format!(
                            "scenario node references missing controller '{}'",
                            node.ref_id
                        ),
                    ));
                }
            }
            other => messages.push(error(
                "invalid_node_kind",
                format!("scenario.{}.node.{}", scenario.id, node.id),
                format!(
                    "scenario node kind '{other}' is invalid; expected 'profile' or 'controller'"
                ),
            )),
        }
    }

    if demand_sink_count == 0 {
        messages.push(error(
            "missing_demand_sink",
            format!("scenario.{}", scenario.id),
            "scenario must include one household demand sink node".to_owned(),
        ));
    } else if demand_sink_count > 1 {
        messages.push(warning(
            "multiple_demand_sinks",
            format!("scenario.{}", scenario.id),
            "scenario includes multiple demand sinks; sandbox playback uses the first one"
                .to_owned(),
        ));
    }

    let mut incoming_resources: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    let mut profile_graph_outgoing: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    let mut profile_graph_incoming_degree: BTreeMap<&str, u32> = BTreeMap::new();

    for edge in &scenario.edges {
        let Some(from_node) = node_map.get(edge.from.as_str()) else {
            messages.push(error(
                "missing_edge_source_node",
                format!("scenario.{}.edge.{}->{}", scenario.id, edge.from, edge.to),
                format!("edge source node '{}' does not exist", edge.from),
            ));
            continue;
        };
        let Some(to_node) = node_map.get(edge.to.as_str()) else {
            messages.push(error(
                "missing_edge_target_node",
                format!("scenario.{}.edge.{}->{}", scenario.id, edge.from, edge.to),
                format!("edge target node '{}' does not exist", edge.to),
            ));
            continue;
        };
        if from_node.ref_kind != "profile" || to_node.ref_kind != "profile" {
            messages.push(error(
                "invalid_edge_endpoint_kind",
                format!("scenario.{}.edge.{}->{}", scenario.id, edge.from, edge.to),
                "scenario edges may connect profile nodes only".to_owned(),
            ));
            continue;
        }

        let Some(from_profile) = profile_map.get(from_node.ref_id.as_str()) else {
            continue;
        };
        let Some(to_profile) = profile_map.get(to_node.ref_id.as_str()) else {
            continue;
        };

        if !port_exists(&from_profile.outputs, edge.resource.as_str()) {
            messages.push(error(
                "edge_resource_not_produced",
                format!("scenario.{}.edge.{}->{}", scenario.id, edge.from, edge.to),
                format!(
                    "source profile '{}' does not output resource '{}'",
                    from_profile.id, edge.resource
                ),
            ));
        }
        if !port_exists(&to_profile.inputs, edge.resource.as_str()) {
            messages.push(error(
                "edge_resource_not_consumed",
                format!("scenario.{}.edge.{}->{}", scenario.id, edge.from, edge.to),
                format!(
                    "target profile '{}' does not consume resource '{}'",
                    to_profile.id, edge.resource
                ),
            ));
        }

        incoming_resources
            .entry(edge.to.as_str())
            .or_default()
            .insert(edge.resource.as_str());

        profile_graph_outgoing
            .entry(edge.from.as_str())
            .or_default()
            .insert(edge.to.as_str());
        *profile_graph_incoming_degree
            .entry(edge.to.as_str())
            .or_insert(0) += 1;
        profile_graph_incoming_degree
            .entry(edge.from.as_str())
            .or_insert(0);
    }

    for node in &scenario.nodes {
        if node.ref_kind != "profile" {
            continue;
        }
        let Some(profile) = profile_map.get(node.ref_id.as_str()) else {
            continue;
        };
        let received = incoming_resources.get(node.id.as_str());
        for input in &profile.inputs {
            if received.is_none_or(|resources| !resources.contains(input.resource.as_str())) {
                messages.push(error(
                    "disconnected_required_input",
                    format!("scenario.{}.node.{}", scenario.id, node.id),
                    format!(
                        "profile '{}' requires input '{}' but no edge supplies it",
                        profile.id, input.resource
                    ),
                ));
            }
        }
    }

    for link in &scenario.controller_links {
        let Some(controller_node) = node_map.get(link.controller_node_id.as_str()) else {
            messages.push(error(
                "missing_controller_link_source",
                format!(
                    "scenario.{}.controller_link.{}->{}",
                    scenario.id, link.controller_node_id, link.target_node_id
                ),
                format!(
                    "controller node '{}' does not exist",
                    link.controller_node_id
                ),
            ));
            continue;
        };
        let Some(target_node) = node_map.get(link.target_node_id.as_str()) else {
            messages.push(error(
                "missing_controller_link_target",
                format!(
                    "scenario.{}.controller_link.{}->{}",
                    scenario.id, link.controller_node_id, link.target_node_id
                ),
                format!("target node '{}' does not exist", link.target_node_id),
            ));
            continue;
        };
        if controller_node.ref_kind != "controller" {
            messages.push(error(
                "invalid_controller_link_source_kind",
                format!(
                    "scenario.{}.controller_link.{}->{}",
                    scenario.id, link.controller_node_id, link.target_node_id
                ),
                "controller link source must be a controller node".to_owned(),
            ));
        }
        if target_node.ref_kind != "profile" {
            messages.push(error(
                "invalid_controller_link_target_kind",
                format!(
                    "scenario.{}.controller_link.{}->{}",
                    scenario.id, link.controller_node_id, link.target_node_id
                ),
                "controller links must target profile nodes".to_owned(),
            ));
        }
    }

    let cycle_detected = has_profile_cycle(
        &scenario.nodes,
        &profile_graph_outgoing,
        &profile_graph_incoming_degree,
    );
    if cycle_detected {
        messages.push(error(
            "cyclic_scenario_graph",
            format!("scenario.{}", scenario.id),
            "scenario graph contains a cycle; bootstrap playback requires an acyclic profile chain"
                .to_owned(),
        ));
    }
}
fn has_profile_cycle(
    nodes: &[ScenarioNode],
    outgoing: &BTreeMap<&str, BTreeSet<&str>>,
    indegree: &BTreeMap<&str, u32>,
) -> bool {
    let profile_node_count = nodes
        .iter()
        .filter(|node| node.ref_kind == "profile")
        .count();
    if profile_node_count == 0 {
        return false;
    }

    let mut indegree = indegree.clone();
    let mut queue = VecDeque::new();
    for node in nodes.iter().filter(|node| node.ref_kind == "profile") {
        if indegree.get(node.id.as_str()).copied().unwrap_or(0) == 0 {
            queue.push_back(node.id.as_str());
        }
    }

    let mut visited = 0usize;
    while let Some(node_id) = queue.pop_front() {
        visited += 1;
        if let Some(targets) = outgoing.get(node_id) {
            for &target in targets {
                if let Some(entry) = indegree.get_mut(target) {
                    *entry -= 1;
                    if *entry == 0 {
                        queue.push_back(target);
                    }
                }
            }
        }
    }

    visited != profile_node_count
}

fn duplicate_ids<'a>(ids: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut duplicates = BTreeSet::new();
    for id in ids {
        if !seen.insert(id.to_owned()) {
            duplicates.insert(id.to_owned());
        }
    }
    duplicates.into_iter().collect()
}

fn port_exists(ports: &[ResourcePort], resource: &str) -> bool {
    ports.iter().any(|port| port.resource == resource)
}

fn error(
    code: &'static str,
    scope: impl Into<String>,
    message: impl Into<String>,
) -> ValidationMessage {
    ValidationMessage {
        severity: "error",
        code,
        scope: scope.into(),
        message: message.into(),
    }
}

fn warning(
    code: &'static str,
    scope: impl Into<String>,
    message: impl Into<String>,
) -> ValidationMessage {
    ValidationMessage {
        severity: "warning",
        code,
        scope: scope.into(),
        message: message.into(),
    }
}
