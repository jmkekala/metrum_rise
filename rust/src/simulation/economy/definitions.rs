//! Authored economy definitions used by the developer-facing economy editor.
//!
//! The runtime household/building simulation still owns live economic state, but
//! this module defines the authoritative TOML-backed profile/controller/scenario
//! data used to validate and tune the first-pass economy chains. The same data
//! will later feed the asset editor and a fuller compiled runtime representation.

use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;

const PROFILES_FILE: &str = "profiles.toml";
const CONTROLLERS_FILE: &str = "controllers.toml";
const SCENARIOS_FILE: &str = "scenarios.toml";
const INDEX_FILE: &str = "economy.index.bin";

/// Loads an authored economy project from the canonical `economy/` folder and
/// returns a JSON envelope containing the parsed project plus validation output.
pub fn load_project_json(dir_path: &Path) -> Result<String, String> {
    let project = load_project(dir_path)?;
    let validation = validate_project(&project);
    let payload = serde_json::json!({
        "ok": true,
        "source_dir": dir_path,
        "project": project,
        "validation": validation,
    });
    serde_json::to_string(&payload).map_err(|err| format!("could not encode economy project JSON: {err}"))
}

/// Validates and exports an authored economy project back to canonical TOML
/// files and regenerates the derived `economy.index.bin` cache.
pub fn export_project_json(project_json: &str, dir_path: &Path) -> Result<String, String> {
    let project: EconomyProject = serde_json::from_str(project_json)
        .map_err(|err| format!("economy project JSON parse error: {err}"))?;
    let validation = validate_project(&project);
    if validation.iter().any(|msg| msg.severity == "error") {
        let payload = serde_json::json!({
            "ok": false,
            "error": "validation failed; export aborted",
            "validation": validation,
        });
        return serde_json::to_string(&payload)
            .map_err(|err| format!("could not encode failed export JSON: {err}"));
    }

    std::fs::create_dir_all(dir_path)
        .map_err(|err| format!("could not create economy dir '{}': {err}", dir_path.display()))?;

    write_pretty_toml(&dir_path.join(PROFILES_FILE), &ProfilesFile {
        profiles: project.profiles.clone(),
    })?;
    write_pretty_toml(&dir_path.join(CONTROLLERS_FILE), &ControllersFile {
        controllers: project.controllers.clone(),
    })?;
    write_pretty_toml(&dir_path.join(SCENARIOS_FILE), &ScenariosFile {
        scenarios: project.scenarios.clone(),
    })?;

    let compiled = build_index(&project);
    let compiled_bytes = serde_json::to_vec(&compiled)
        .map_err(|err| format!("could not encode economy cache: {err}"))?;
    std::fs::write(dir_path.join(INDEX_FILE), compiled_bytes)
        .map_err(|err| format!("could not write economy cache: {err}"))?;

    let payload = serde_json::json!({
        "ok": true,
        "validation": validation,
        "cache_path": dir_path.join(INDEX_FILE),
    });
    serde_json::to_string(&payload).map_err(|err| format!("could not encode export JSON: {err}"))
}

/// Runs the small authored-economy sandbox for a selected scenario and returns
/// a JSON envelope with summary metrics and daily series data.
pub fn run_sandbox_json(project_json: &str, scenario_id: &str) -> Result<String, String> {
    let project: EconomyProject = serde_json::from_str(project_json)
        .map_err(|err| format!("economy project JSON parse error: {err}"))?;
    let validation = validate_project(&project);
    if validation.iter().any(|msg| msg.severity == "error") {
        let payload = serde_json::json!({
            "ok": false,
            "error": "validation failed; sandbox aborted",
            "validation": validation,
        });
        return serde_json::to_string(&payload)
            .map_err(|err| format!("could not encode failed sandbox JSON: {err}"));
    }

    let result = run_sandbox(&project, scenario_id)?;
    let payload = serde_json::json!({
        "ok": true,
        "validation": validation,
        "result": result,
    });
    serde_json::to_string(&payload).map_err(|err| format!("could not encode sandbox JSON: {err}"))
}

#[derive(Clone, Serialize, Deserialize)]
struct EconomyProject {
    #[serde(default)]
    profiles: Vec<EconomyProfile>,
    #[serde(default)]
    controllers: Vec<EconomyController>,
    #[serde(default)]
    scenarios: Vec<EconomyScenario>,
}

#[derive(Clone, Serialize, Deserialize)]
struct EconomyProfile {
    id: String,
    display_name: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    description: String,
    #[serde(default, deserialize_with = "deserialize_u32_from_number")]
    worker_capacity: u32,
    #[serde(default)]
    base_rate_units_per_day: f32,
    #[serde(default)]
    wage_min_currency_per_day: f32,
    #[serde(default)]
    wage_max_currency_per_day: f32,
    #[serde(default)]
    unit_price_currency: f32,
    #[serde(default)]
    stock_target_days: f32,
    #[serde(default)]
    reorder_threshold_days: f32,
    #[serde(default)]
    critical_threshold_days: f32,
    #[serde(default)]
    min_shipment_units: f32,
    #[serde(default)]
    consumption_rate_per_resident: f32,
    #[serde(default)]
    inputs: Vec<ResourcePort>,
    #[serde(default)]
    outputs: Vec<ResourcePort>,
}

#[derive(Clone, Serialize, Deserialize)]
struct ResourcePort {
    resource: String,
    units_per_day: f32,
}

#[derive(Clone, Serialize, Deserialize)]
struct EconomyController {
    id: String,
    display_name: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    description: String,
    #[serde(default = "default_one")]
    default_weight: f32,
    #[serde(default = "default_one")]
    min_multiplier: f32,
    #[serde(default = "default_one")]
    max_multiplier: f32,
}

#[derive(Clone, Serialize, Deserialize)]
struct EconomyScenario {
    id: String,
    display_name: String,
    #[serde(default)]
    description: String,
    #[serde(default = "default_duration_days", deserialize_with = "deserialize_u32_from_number")]
    duration_days: u32,
    #[serde(default, deserialize_with = "deserialize_u32_from_number")]
    household_count: u32,
    #[serde(default = "default_one")]
    average_household_size: f32,
    #[serde(default)]
    starting_household_stock_days: f32,
    #[serde(default)]
    replenishment_target_days: f32,
    #[serde(default)]
    replenishment_trigger_days: f32,
    #[serde(default)]
    pickup_cadence_hours: f32,
    #[serde(default)]
    nodes: Vec<ScenarioNode>,
    #[serde(default)]
    edges: Vec<ScenarioEdge>,
    #[serde(default)]
    controller_links: Vec<ScenarioControllerLink>,
}

#[derive(Clone, Serialize, Deserialize)]
struct ScenarioNode {
    id: String,
    ref_kind: String,
    ref_id: String,
    position: [f32; 2],
}

#[derive(Clone, Serialize, Deserialize)]
struct ScenarioEdge {
    from: String,
    to: String,
    resource: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct ScenarioControllerLink {
    controller_node_id: String,
    target_node_id: String,
}

#[derive(Serialize, Deserialize)]
struct ProfilesFile {
    #[serde(default)]
    profiles: Vec<EconomyProfile>,
}

#[derive(Serialize, Deserialize)]
struct ControllersFile {
    #[serde(default)]
    controllers: Vec<EconomyController>,
}

#[derive(Serialize, Deserialize)]
struct ScenariosFile {
    #[serde(default)]
    scenarios: Vec<EconomyScenario>,
}

#[derive(Serialize)]
struct ValidationMessage {
    severity: &'static str,
    code: &'static str,
    scope: String,
    message: String,
}

#[derive(Serialize)]
struct CompiledEconomyIndex {
    profile_ids: Vec<String>,
    controller_ids: Vec<String>,
    scenario_ids: Vec<String>,
    compatibility: Vec<CompiledCompatibility>,
}

#[derive(Serialize)]
struct CompiledCompatibility {
    resource: String,
    source_profile_id: String,
    target_profile_ids: Vec<String>,
}

#[derive(Serialize)]
struct SandboxResult {
    scenario_id: String,
    display_name: String,
    duration_days: u32,
    daily_household_demand_units: f32,
    final_household_stock_days: f32,
    lowest_household_stock_days: f32,
    total_delivered_units: f32,
    total_unmet_units: f32,
    average_household_cost_per_day: f32,
    bottlenecks: Vec<String>,
    daily: Vec<DailySandboxMetric>,
}

#[derive(Serialize)]
struct DailySandboxMetric {
    day: u32,
    household_stock_days: f32,
    delivered_units: f32,
    unmet_units: f32,
    average_household_cost: f32,
}

fn default_duration_days() -> u32 {
    30
}

fn default_one() -> f32 {
    1.0
}

fn deserialize_u32_from_number<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::Number(number) => {
            if let Some(unsigned) = number.as_u64() {
                return u32::try_from(unsigned)
                    .map_err(|_| serde::de::Error::custom("numeric value exceeds u32 range"));
            }
            if let Some(signed) = number.as_i64() {
                return u32::try_from(signed)
                    .map_err(|_| serde::de::Error::custom("numeric value must be >= 0 and within u32 range"));
            }
            if let Some(float) = number.as_f64() {
                if !float.is_finite() || float < 0.0 {
                    return Err(serde::de::Error::custom("numeric value must be finite and >= 0"));
                }
                let rounded = float.round();
                if (float - rounded).abs() > f64::EPSILON {
                    return Err(serde::de::Error::custom("numeric value must be a whole number"));
                }
                return u32::try_from(rounded as i64)
                    .map_err(|_| serde::de::Error::custom("numeric value exceeds u32 range"));
            }
            Err(serde::de::Error::custom("unsupported numeric representation"))
        }
        serde_json::Value::Null => Ok(0),
        other => Err(serde::de::Error::custom(format!(
            "expected numeric value for u32 field, got {other}"
        ))),
    }
}

fn load_project(dir_path: &Path) -> Result<EconomyProject, String> {
    let profiles: ProfilesFile = parse_toml_file(&dir_path.join(PROFILES_FILE))?;
    let controllers: ControllersFile = parse_toml_file(&dir_path.join(CONTROLLERS_FILE))?;
    let scenarios: ScenariosFile = parse_toml_file(&dir_path.join(SCENARIOS_FILE))?;
    Ok(EconomyProject {
        profiles: profiles.profiles,
        controllers: controllers.controllers,
        scenarios: scenarios.scenarios,
    })
}

fn parse_toml_file<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|err| format!("could not read '{}': {err}", path.display()))?;
    toml::from_str(&content).map_err(|err| format!("could not parse '{}': {err}", path.display()))
}

fn write_pretty_toml<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let encoded = toml::to_string_pretty(value)
        .map_err(|err| format!("could not encode '{}': {err}", path.display()))?;
    std::fs::write(path, encoded)
        .map_err(|err| format!("could not write '{}': {err}", path.display()))
}

fn validate_project(project: &EconomyProject) -> Vec<ValidationMessage> {
    let mut messages = Vec::new();
    let profile_ids = duplicate_ids(project.profiles.iter().map(|profile| profile.id.as_str()));
    for duplicate in profile_ids {
        messages.push(error(
            "duplicate_profile_id",
            "project.profiles",
            format!("profile id '{duplicate}' is defined more than once"),
        ));
    }

    let controller_ids = duplicate_ids(project.controllers.iter().map(|controller| controller.id.as_str()));
    for duplicate in controller_ids {
        messages.push(error(
            "duplicate_controller_id",
            "project.controllers",
            format!("controller id '{duplicate}' is defined more than once"),
        ));
    }

    let scenario_ids = duplicate_ids(project.scenarios.iter().map(|scenario| scenario.id.as_str()));
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
        validate_scenario(
            scenario,
            &profile_map,
            &controller_map,
            &mut messages,
        );
    }

    messages
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
                        format!("scenario node references missing controller '{}'", node.ref_id),
                    ));
                }
            }
            other => messages.push(error(
                "invalid_node_kind",
                format!("scenario.{}.node.{}", scenario.id, node.id),
                format!("scenario node kind '{other}' is invalid; expected 'profile' or 'controller'"),
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
            "scenario includes multiple demand sinks; sandbox playback uses the first one".to_owned(),
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
        *profile_graph_incoming_degree.entry(edge.to.as_str()).or_insert(0) += 1;
        profile_graph_incoming_degree.entry(edge.from.as_str()).or_insert(0);
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
                format!("controller node '{}' does not exist", link.controller_node_id),
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

    let cycle_detected = has_profile_cycle(&scenario.nodes, &profile_graph_outgoing, &profile_graph_incoming_degree);
    if cycle_detected {
        messages.push(error(
            "cyclic_scenario_graph",
            format!("scenario.{}", scenario.id),
            "scenario graph contains a cycle; bootstrap playback requires an acyclic profile chain".to_owned(),
        ));
    }
}

fn run_sandbox(project: &EconomyProject, scenario_id: &str) -> Result<SandboxResult, String> {
    let scenario = project
        .scenarios
        .iter()
        .find(|scenario| scenario.id == scenario_id)
        .ok_or_else(|| format!("scenario '{scenario_id}' not found"))?;

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
    let node_map: BTreeMap<&str, &ScenarioNode> = scenario
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect();

    let topo_order = topological_profile_node_order(scenario)?;
    let demand_sink_node = scenario
        .nodes
        .iter()
        .find(|node| {
            node.ref_kind == "profile"
                && profile_map
                    .get(node.ref_id.as_str())
                    .is_some_and(|profile| profile.kind == "demand_sink")
        })
        .ok_or_else(|| format!("scenario '{}' has no demand sink", scenario.id))?;
    let demand_sink_profile = profile_map
        .get(demand_sink_node.ref_id.as_str())
        .copied()
        .ok_or_else(|| format!("demand sink profile '{}' is missing", demand_sink_node.ref_id))?;

    let household_demand_per_day =
        scenario.household_count as f32
        * scenario.average_household_size
        * demand_sink_profile.consumption_rate_per_resident.max(0.0);

    let mut household_stock_units = household_demand_per_day * scenario.starting_household_stock_days.max(0.0);
    let mut lowest_stock_days = if household_demand_per_day > 0.0 {
        household_stock_units / household_demand_per_day
    } else {
        0.0
    };
    let mut total_delivered_units = 0.0;
    let mut total_unmet_units = 0.0;
    let mut total_household_cost = 0.0;
    let mut daily = Vec::with_capacity(scenario.duration_days as usize);
    let mut inventories: BTreeMap<String, BTreeMap<String, f32>> = BTreeMap::new();

    let outgoing_edges = build_outgoing_edges(scenario);
    let household_price_multiplier = household_cost_multiplier(
        scenario,
        demand_sink_node.id.as_str(),
        &node_map,
        &controller_map,
    );

    for day in 1..=scenario.duration_days {
        let mut delivered_today = 0.0;
        let mut unmet_today = 0.0;
        let mut household_cost_today = 0.0;

        for node_id in &topo_order {
            let node = node_map
                .get(node_id.as_str())
                .copied()
                .ok_or_else(|| format!("scenario node '{}' missing during sandbox playback", node_id))?;
            let profile = profile_map
                .get(node.ref_id.as_str())
                .copied()
                .ok_or_else(|| format!("profile '{}' missing during sandbox playback", node.ref_id))?;

            if profile.kind == "demand_sink" {
                let delivered_to_sink = take_all_incoming_stock(
                    &mut inventories,
                    node.id.as_str(),
                    &profile.inputs,
                );
                delivered_today += delivered_to_sink;
                household_stock_units += delivered_to_sink;
                let consumed = household_stock_units.min(household_demand_per_day);
                unmet_today = household_demand_per_day - consumed;
                household_stock_units -= consumed;
                household_cost_today += delivered_to_sink * inferred_unit_price(scenario, node.id.as_str(), &outgoing_edges, &node_map, &profile_map) * household_price_multiplier;
                continue;
            }

            let throughput = compute_throughput(profile, &inventories, node.id.as_str());
            if profile.inputs.is_empty() {
                add_outputs_to_inventory(&mut inventories, node.id.as_str(), &profile.outputs, 1.0);
            } else if throughput > 0.0 && profile.base_rate_units_per_day > 0.0 {
                let scale = throughput / profile.base_rate_units_per_day;
                consume_inputs_from_inventory(&mut inventories, node.id.as_str(), &profile.inputs, scale);
                add_outputs_to_inventory(&mut inventories, node.id.as_str(), &profile.outputs, scale);
            }

            transfer_outgoing_stock(
                &mut inventories,
                node.id.as_str(),
                outgoing_edges.get(node.id.as_str()),
            );
        }

        let stock_days = if household_demand_per_day > 0.0 {
            household_stock_units / household_demand_per_day
        } else {
            0.0
        };
        lowest_stock_days = lowest_stock_days.min(stock_days);
        total_delivered_units += delivered_today;
        total_unmet_units += unmet_today;
        total_household_cost += household_cost_today;
        daily.push(DailySandboxMetric {
            day,
            household_stock_days: stock_days,
            delivered_units: delivered_today,
            unmet_units: unmet_today,
            average_household_cost: if scenario.household_count > 0 {
                household_cost_today / scenario.household_count as f32
            } else {
                0.0
            },
        });
    }

    let final_stock_days = if household_demand_per_day > 0.0 {
        household_stock_units / household_demand_per_day
    } else {
        0.0
    };

    let mut bottlenecks = Vec::new();
    if lowest_stock_days < 1.0 {
        bottlenecks.push(format!(
            "Household stock drops below 1.0 days and bottoms out at {:.2} days.",
            lowest_stock_days
        ));
    }
    if total_unmet_units > 0.0 {
        bottlenecks.push(format!(
            "Sandbox leaves {:.1} household_supplies units unmet across {} days.",
            total_unmet_units,
            scenario.duration_days
        ));
    }
    if bottlenecks.is_empty() {
        bottlenecks.push("Starter chain remains stocked for the whole sandbox run.".to_owned());
    }

    Ok(SandboxResult {
        scenario_id: scenario.id.clone(),
        display_name: scenario.display_name.clone(),
        duration_days: scenario.duration_days,
        daily_household_demand_units: household_demand_per_day,
        final_household_stock_days: final_stock_days,
        lowest_household_stock_days: lowest_stock_days,
        total_delivered_units,
        total_unmet_units,
        average_household_cost_per_day: if scenario.duration_days > 0 && scenario.household_count > 0 {
            total_household_cost / (scenario.duration_days as f32 * scenario.household_count as f32)
        } else {
            0.0
        },
        bottlenecks,
        daily,
    })
}

fn build_index(project: &EconomyProject) -> CompiledEconomyIndex {
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
        profile_ids: project.profiles.iter().map(|profile| profile.id.clone()).collect(),
        controller_ids: project.controllers.iter().map(|controller| controller.id.clone()).collect(),
        scenario_ids: project.scenarios.iter().map(|scenario| scenario.id.clone()).collect(),
        compatibility,
    }
}

fn build_outgoing_edges<'a>(scenario: &'a EconomyScenario) -> BTreeMap<&'a str, Vec<&'a ScenarioEdge>> {
    let mut outgoing: BTreeMap<&str, Vec<&ScenarioEdge>> = BTreeMap::new();
    for edge in &scenario.edges {
        outgoing.entry(edge.from.as_str()).or_default().push(edge);
    }
    outgoing
}

fn household_cost_multiplier(
    scenario: &EconomyScenario,
    demand_sink_node_id: &str,
    node_map: &BTreeMap<&str, &ScenarioNode>,
    controller_map: &BTreeMap<&str, &EconomyController>,
) -> f32 {
    for link in &scenario.controller_links {
        if link.target_node_id != demand_sink_node_id {
            continue;
        }
        let Some(controller_node) = node_map.get(link.controller_node_id.as_str()) else {
            continue;
        };
        if controller_node.ref_kind != "controller" {
            continue;
        }
        let Some(controller) = controller_map.get(controller_node.ref_id.as_str()) else {
            continue;
        };
        if controller.kind != "household_restock_cost" {
            continue;
        }
        let t = controller.default_weight.clamp(0.0, 1.0);
        return controller.min_multiplier + (controller.max_multiplier - controller.min_multiplier) * t;
    }
    1.0
}

fn inferred_unit_price(
    scenario: &EconomyScenario,
    demand_sink_node_id: &str,
    outgoing_edges: &BTreeMap<&str, Vec<&ScenarioEdge>>,
    node_map: &BTreeMap<&str, &ScenarioNode>,
    profile_map: &BTreeMap<&str, &EconomyProfile>,
) -> f32 {
    for edge in &scenario.edges {
        if edge.to != demand_sink_node_id {
            continue;
        }
        let Some(source_node) = node_map.get(edge.from.as_str()) else {
            continue;
        };
        let Some(source_profile) = profile_map.get(source_node.ref_id.as_str()) else {
            continue;
        };
        if source_profile.unit_price_currency > 0.0 {
            return source_profile.unit_price_currency;
        }
    }
    for (node_id, edges) in outgoing_edges {
        if edges.iter().any(|edge| edge.to == demand_sink_node_id) {
            let Some(source_node) = node_map.get(node_id) else {
                continue;
            };
            let Some(source_profile) = profile_map.get(source_node.ref_id.as_str()) else {
                continue;
            };
            if source_profile.unit_price_currency > 0.0 {
                return source_profile.unit_price_currency;
            }
        }
    }
    0.0
}

fn compute_throughput(
    profile: &EconomyProfile,
    inventories: &BTreeMap<String, BTreeMap<String, f32>>,
    node_id: &str,
) -> f32 {
    if profile.inputs.is_empty() {
        return profile.base_rate_units_per_day.max(0.0);
    }
    if profile.base_rate_units_per_day <= 0.0 {
        return 0.0;
    }

    let mut throughput = profile.base_rate_units_per_day;
    for input in &profile.inputs {
        let available = inventories
            .get(node_id)
            .and_then(|stock| stock.get(input.resource.as_str()))
            .copied()
            .unwrap_or(0.0);
        if input.units_per_day <= 0.0 {
            continue;
        }
        let allowed = available / input.units_per_day * profile.base_rate_units_per_day;
        throughput = throughput.min(allowed);
    }
    throughput.max(0.0)
}

fn add_outputs_to_inventory(
    inventories: &mut BTreeMap<String, BTreeMap<String, f32>>,
    node_id: &str,
    outputs: &[ResourcePort],
    scale: f32,
) {
    let stock = inventories.entry(node_id.to_owned()).or_default();
    for output in outputs {
        *stock.entry(output.resource.clone()).or_default() += output.units_per_day * scale;
    }
}

fn consume_inputs_from_inventory(
    inventories: &mut BTreeMap<String, BTreeMap<String, f32>>,
    node_id: &str,
    inputs: &[ResourcePort],
    scale: f32,
) {
    let stock = inventories.entry(node_id.to_owned()).or_default();
    for input in inputs {
        let entry = stock.entry(input.resource.clone()).or_default();
        *entry = (*entry - input.units_per_day * scale).max(0.0);
    }
}

fn take_all_incoming_stock(
    inventories: &mut BTreeMap<String, BTreeMap<String, f32>>,
    node_id: &str,
    inputs: &[ResourcePort],
) -> f32 {
    let stock = inventories.entry(node_id.to_owned()).or_default();
    let mut total = 0.0;
    for input in inputs {
        if let Some(entry) = stock.get_mut(input.resource.as_str()) {
            total += *entry;
            *entry = 0.0;
        }
    }
    total
}

fn transfer_outgoing_stock(
    inventories: &mut BTreeMap<String, BTreeMap<String, f32>>,
    from_node_id: &str,
    outgoing_edges: Option<&Vec<&ScenarioEdge>>,
) {
    let Some(outgoing_edges) = outgoing_edges else {
        return;
    };

    let mut transfers: Vec<(String, String, f32)> = Vec::new();
    {
        let Some(stock) = inventories.get_mut(from_node_id) else {
            return;
        };
        for edge in outgoing_edges {
            let Some(amount) = stock.get_mut(edge.resource.as_str()) else {
                continue;
            };
            if *amount <= 0.0 {
                continue;
            }
            let moved = *amount;
            *amount = 0.0;
            transfers.push((edge.to.clone(), edge.resource.clone(), moved));
        }
    }

    for (target_node, resource, amount) in transfers {
        *inventories
            .entry(target_node)
            .or_default()
            .entry(resource)
            .or_default() += amount;
    }
}

fn topological_profile_node_order(scenario: &EconomyScenario) -> Result<Vec<String>, String> {
    let mut outgoing: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let mut indegree: BTreeMap<&str, u32> = scenario
        .nodes
        .iter()
        .filter(|node| node.ref_kind == "profile")
        .map(|node| (node.id.as_str(), 0))
        .collect();

    for edge in &scenario.edges {
        outgoing.entry(edge.from.as_str()).or_default().push(edge.to.as_str());
        *indegree.entry(edge.to.as_str()).or_insert(0) += 1;
    }

    let mut queue = VecDeque::new();
    for (&node_id, &degree) in &indegree {
        if degree == 0 {
            queue.push_back(node_id);
        }
    }

    let mut ordered = Vec::with_capacity(indegree.len());
    while let Some(node_id) = queue.pop_front() {
        ordered.push(node_id.to_owned());
        if let Some(next) = outgoing.get(node_id) {
            for &target in next {
                if let Some(degree) = indegree.get_mut(target) {
                    *degree -= 1;
                    if *degree == 0 {
                        queue.push_back(target);
                    }
                }
            }
        }
    }

    if ordered.len() != indegree.len() {
        return Err(format!("scenario '{}' contains a profile-cycle", scenario.id));
    }

    Ok(ordered)
}

fn has_profile_cycle(
    nodes: &[ScenarioNode],
    outgoing: &BTreeMap<&str, BTreeSet<&str>>,
    indegree: &BTreeMap<&str, u32>,
) -> bool {
    let profile_node_count = nodes.iter().filter(|node| node.ref_kind == "profile").count();
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

fn error(code: &'static str, scope: impl Into<String>, message: impl Into<String>) -> ValidationMessage {
    ValidationMessage {
        severity: "error",
        code,
        scope: scope.into(),
        message: message.into(),
    }
}

fn warning(code: &'static str, scope: impl Into<String>, message: impl Into<String>) -> ValidationMessage {
    ValidationMessage {
        severity: "warning",
        code,
        scope: scope.into(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn project_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(name)
    }

    fn write_fixture_project(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join(PROFILES_FILE),
            r#"
[[profiles]]
id = "food_processor_basic"
display_name = "Food Processor"
kind = "producer"
description = "Starter processor"
worker_capacity = 4
base_rate_units_per_day = 160.0
wage_min_currency_per_day = 90.0
wage_max_currency_per_day = 110.0
unit_price_currency = 4.0

[[profiles.outputs]]
resource = "staple_food"
units_per_day = 160.0

[[profiles]]
id = "grocery_basic"
display_name = "Grocery"
kind = "store"
description = "Starter grocery"
worker_capacity = 3
base_rate_units_per_day = 200.0
wage_min_currency_per_day = 80.0
wage_max_currency_per_day = 100.0
unit_price_currency = 6.0
stock_target_days = 3.0
reorder_threshold_days = 2.0
critical_threshold_days = 0.5
min_shipment_units = 40.0

[[profiles.inputs]]
resource = "staple_food"
units_per_day = 160.0

[[profiles.outputs]]
resource = "household_supplies"
units_per_day = 200.0

[[profiles]]
id = "household_demand_sink"
display_name = "Household Demand Sink"
kind = "demand_sink"
description = "Starter sink"
consumption_rate_per_resident = 1.0

[[profiles.inputs]]
resource = "household_supplies"
units_per_day = 1.0
"#,
        )
        .unwrap();

        std::fs::write(
            dir.join(CONTROLLERS_FILE),
            r#"
[[controllers]]
id = "household_restock_cost_basic"
display_name = "Household Restock Cost"
kind = "household_restock_cost"
description = "Starter household cost controller"
default_weight = 0.5
min_multiplier = 0.9
max_multiplier = 1.1
"#,
        )
        .unwrap();

        std::fs::write(
            dir.join(SCENARIOS_FILE),
            r#"
[[scenarios]]
id = "grocery_bottleneck"
display_name = "Grocery Bottleneck"
description = "Starter bottleneck test"
duration_days = 30
household_count = 60
average_household_size = 2.0
starting_household_stock_days = 3.0
replenishment_target_days = 3.0
replenishment_trigger_days = 1.5
pickup_cadence_hours = 6.0

[[scenarios.nodes]]
id = "food_processor"
ref_kind = "profile"
ref_id = "food_processor_basic"
position = [120.0, 180.0]

[[scenarios.nodes]]
id = "grocery"
ref_kind = "profile"
ref_id = "grocery_basic"
position = [460.0, 180.0]

[[scenarios.nodes]]
id = "households"
ref_kind = "profile"
ref_id = "household_demand_sink"
position = [820.0, 180.0]

[[scenarios.nodes]]
id = "replenishment_cost"
ref_kind = "controller"
ref_id = "household_restock_cost_basic"
position = [820.0, 40.0]

[[scenarios.edges]]
from = "food_processor"
to = "grocery"
resource = "staple_food"

[[scenarios.edges]]
from = "grocery"
to = "households"
resource = "household_supplies"

[[scenarios.controller_links]]
controller_node_id = "replenishment_cost"
target_node_id = "households"
"#,
        )
        .unwrap();
    }

    #[test]
    fn load_project_returns_valid_json() {
        let dir = project_dir("metrum_economy_editor_load");
        let _ = std::fs::remove_dir_all(&dir);
        write_fixture_project(&dir);

        let json = load_project_json(&dir).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed["ok"].as_bool().unwrap());
        assert_eq!(parsed["project"]["profiles"].as_array().unwrap().len(), 3);
        assert!(parsed["validation"].as_array().unwrap().is_empty());
    }

    #[test]
    fn export_project_writes_cache_file() {
        let dir = project_dir("metrum_economy_editor_export");
        let _ = std::fs::remove_dir_all(&dir);
        write_fixture_project(&dir);
        let loaded = load_project_json(&dir).unwrap();
        let project_json = serde_json::to_string(&serde_json::from_str::<serde_json::Value>(&loaded).unwrap()["project"]).unwrap();

        let out_dir = project_dir("metrum_economy_editor_export_out");
        let _ = std::fs::remove_dir_all(&out_dir);
        let result = export_project_json(&project_json, &out_dir).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert!(parsed["ok"].as_bool().unwrap());
        assert!(out_dir.join(PROFILES_FILE).exists());
        assert!(out_dir.join(CONTROLLERS_FILE).exists());
        assert!(out_dir.join(SCENARIOS_FILE).exists());
        assert!(out_dir.join(INDEX_FILE).exists());
    }

    #[test]
    fn sandbox_returns_daily_metrics() {
        let dir = project_dir("metrum_economy_editor_sandbox");
        let _ = std::fs::remove_dir_all(&dir);
        write_fixture_project(&dir);
        let loaded = load_project_json(&dir).unwrap();
        let loaded_value: serde_json::Value = serde_json::from_str(&loaded).unwrap();
        let project_json = serde_json::to_string(&loaded_value["project"]).unwrap();

        let result = run_sandbox_json(&project_json, "grocery_bottleneck").unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert!(parsed["ok"].as_bool().unwrap());
        assert_eq!(parsed["result"]["daily"].as_array().unwrap().len(), 30);
        assert!(parsed["result"]["final_household_stock_days"].as_f64().unwrap() >= 0.0);
    }

    #[test]
    fn sandbox_accepts_integer_like_float_fields_from_editor_json() {
        let dir = project_dir("metrum_economy_editor_sandbox_float_ints");
        let _ = std::fs::remove_dir_all(&dir);
        write_fixture_project(&dir);
        let loaded = load_project_json(&dir).unwrap();
        let mut loaded_value: serde_json::Value = serde_json::from_str(&loaded).unwrap();
        let project = loaded_value
            .get_mut("project")
            .and_then(serde_json::Value::as_object_mut)
            .unwrap();

        project["profiles"][0]["worker_capacity"] = serde_json::json!(4.0);
        project["profiles"][1]["worker_capacity"] = serde_json::json!(3.0);
        project["scenarios"][0]["duration_days"] = serde_json::json!(30.0);
        project["scenarios"][0]["household_count"] = serde_json::json!(60.0);

        let project_json = serde_json::to_string(project).unwrap();
        let result = run_sandbox_json(&project_json, "grocery_bottleneck").unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert!(parsed["ok"].as_bool().unwrap());
    }
}
