// SPDX-License-Identifier: GPL-3.0-or-later

//! The asset editor browser's rows (`TOOLS-34`): registered assets grouped by pack, then by what
//! their manifests say they are, with the members of one upgrade family (`asset_set`) on one row.
//!
//! Groups rest on manifest fields only, never on the asset id text. Their order is fixed by
//! [`BrowserGroup`]'s declaration order: zoned buildings (zone, then density), city services,
//! parking lots, resource extractors, farms, other explicit buildings, props, vehicles,
//! characters, and a fallback. A building's kind follows the editor's own authoring precedence
//! (`authoring::kind`): extractor, then field, then service class, then plain explicit.
//!
//! With no search, the browser also lists the groups the game names without any asset (zoning
//! profiles, service subtypes, parking, extractor and field resources) that no pack fills
//! ([`missing_groups`]).
//!
//! Runs when the browser refreshes or its search changes, never per frame: for `N` registered
//! assets, `Q` query terms, `L` bytes of searchable text per asset and `G` catalogue groups, one
//! call is `O(N (Q L + log N + log G) + G log G)`, one lowercase text per asset when a query is
//! given.

use crate::assets::asset::{AssetManifest, BuildingData, PlacementMode, ZoneClass};
use crate::assets::authoring::SERVICES;
use crate::assets::registry::AssetRegistry;
use crate::simulation::economy::definitions::{EconomyProfileRuntimeKind, RuntimeEconomyCatalog};
use std::cmp::Ordering;
use std::collections::BTreeMap;

// Explicit buildings of this service class are parking lots (the allocator's own test).
const PARKING_SERVICE_CLASS: &str = "parking";

/// What one asset is, as far as the browser groups it. Declaration order is display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum BrowserGroup<'a> {
    /// A zoned private building, ordered by zone, then low, medium and high, then other
    /// densities by name.
    Zoned {
        zone: u8,
        density_rank: u8,
        density: &'a str,
    },
    /// An explicit building with a declared service class other than parking, ordered as the
    /// editor lists service subtypes, unknown classes after them by name.
    Service { rank: u8, class: &'a str },
    /// An explicit building of service class `parking`.
    Parking,
    /// An explicit building extracting the named resource.
    Extractor(&'a str),
    /// An explicit building growing the named resource on a field.
    Field(&'a str),
    /// An explicit building with no service class, extractor or field.
    OtherBuilding,
    /// A prop, by its authored content-browser category.
    Prop(&'a str),
    /// A vehicle.
    Vehicle,
    /// A pedestrian character.
    Character,
    /// Anything the cases above do not describe (a manifest with no class section, or a zoned
    /// building without a zone; validation refuses both).
    Other,
}

impl<'a> BrowserGroup<'a> {
    /// Classifies one manifest from its fields.
    pub(crate) fn of(manifest: &'a AssetManifest) -> Self {
        if let Some(building) = &manifest.building {
            return Self::of_building(building);
        }
        if let Some(prop) = &manifest.prop {
            return Self::Prop(prop.category.trim());
        }
        if manifest.vehicle.is_some() {
            return Self::Vehicle;
        }
        if manifest.character.is_some() {
            return Self::Character;
        }
        Self::Other
    }

    fn of_building(building: &'a BuildingData) -> Self {
        let resource = |resource: &'a str| Some(resource.trim()).filter(|r| !r.is_empty());
        match building.placement_mode {
            PlacementMode::ZonedPrivate => {
                let Some(zone) = building.zone_type else {
                    return Self::Other;
                };
                Self::zoned(zone, building.density_key().unwrap_or("low"))
            }
            PlacementMode::Explicit => {
                if let Some(resource) = building
                    .extractor
                    .as_ref()
                    .and_then(|e| resource(&e.resource))
                {
                    return Self::Extractor(resource);
                }
                if let Some(resource) = building.field.as_ref().and_then(|f| resource(&f.resource))
                {
                    return Self::Field(resource);
                }
                match building.service_class.as_deref().map(str::trim) {
                    None | Some("" | "none") => Self::OtherBuilding,
                    Some(PARKING_SERVICE_CLASS) => Self::Parking,
                    Some(class) => Self::service(class),
                }
            }
        }
    }

    /// The group of zoned buildings of one zone and density.
    pub(crate) fn zoned(zone: ZoneClass, density: &'a str) -> Self {
        let density = density.trim();
        let density_rank = match density {
            "low" => 0,
            "medium" => 1,
            "high" => 2,
            _ => 3,
        };
        Self::Zoned {
            zone: zone as u8,
            density_rank,
            density,
        }
    }

    fn service(class: &'a str) -> Self {
        let rank = SERVICES.iter().position(|(id, _)| *id == class);
        Self::Service {
            rank: rank.unwrap_or(SERVICES.len()) as u8,
            class,
        }
    }

    /// The group's heading, such as `Residential · low density` or `Service · Fire`.
    pub(crate) fn label(&self) -> String {
        match *self {
            Self::Zoned { zone, density, .. } => {
                format!("{} · {density} density", zone_label(zone))
            }
            Self::Service { rank, class } => {
                let name = SERVICES.get(usize::from(rank)).map_or(class, |(_, l)| l);
                format!("Service · {name}")
            }
            Self::Parking => "Parking lots".to_owned(),
            Self::Extractor(resource) => format!("Resource extractor · {resource}"),
            Self::Field(resource) => format!("Farm · {resource}"),
            Self::OtherBuilding => "Other explicit buildings".to_owned(),
            Self::Prop("") => "Props".to_owned(),
            Self::Prop(category) => format!("Props · {category}"),
            Self::Vehicle => "Vehicles".to_owned(),
            Self::Character => "Characters".to_owned(),
            Self::Other => "Other assets".to_owned(),
        }
    }

    /// The editor's creation type (`authoring::TYPES`) a New asset in this group starts as, or
    /// `""` where the editor creates no such asset.
    pub(crate) fn creation_type(&self) -> &'static str {
        match *self {
            Self::Zoned { zone, .. } => match zone {
                z if z == ZoneClass::Residential as u8 => "residential",
                z if z == ZoneClass::Commercial as u8 => "commercial",
                z if z == ZoneClass::Industrial as u8 => "industrial",
                _ => "",
            },
            Self::Service { .. } | Self::Parking => "service",
            Self::Extractor(_) => "extractor",
            Self::Field(_) => "farm",
            Self::OtherBuilding => "explicit",
            Self::Prop(_) | Self::Vehicle | Self::Character | Self::Other => "",
        }
    }
}

fn zone_label(zone: u8) -> &'static str {
    [
        (ZoneClass::Residential, "Residential"),
        (ZoneClass::Commercial, "Commercial"),
        (ZoneClass::Industrial, "Industrial"),
        (ZoneClass::Office, "Office"),
        (ZoneClass::Mixed, "Mixed"),
    ]
    .into_iter()
    .find(|(class, _)| *class as u8 == zone)
    .map_or("Zoned", |(_, label)| label)
}

/// One level chip: a registered asset of a row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BrowserLevel<'a> {
    /// The building level; `0` for an asset that is not a building.
    pub level: u8,
    /// The qualified id `pack_id:asset_id`.
    pub id: &'a str,
}

/// One browser row: a single asset, or the members of one `asset_set` in one pack and group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BrowserRow<'a> {
    /// The lowest level's display name.
    pub label: &'a str,
    /// The upgrade family, when the row is one.
    pub asset_set: Option<&'a str>,
    /// Members by ascending level, then id; the first is what the row opens.
    pub levels: Vec<BrowserLevel<'a>>,
}

/// One group heading and its rows, ordered by display name (ASCII case folded), then id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BrowserGroupRows<'a> {
    /// What the group holds.
    pub group: BrowserGroup<'a>,
    /// Assets in the group's rows (not rows).
    pub asset_count: usize,
    /// The group's rows.
    pub rows: Vec<BrowserRow<'a>>,
}

/// One pack and its groups, packs ordered by id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BrowserPack<'a> {
    /// The owning pack id.
    pub pack_id: &'a str,
    /// Assets in the pack's groups (not rows).
    pub asset_count: usize,
    /// Groups in [`BrowserGroup`] order.
    pub groups: Vec<BrowserGroupRows<'a>>,
}

/// A parsed browser search: whitespace-separated terms, each of which an asset must match.
#[derive(Debug, Clone, Default)]
pub(crate) struct BrowserQuery {
    terms: Vec<Term>,
}

#[derive(Debug, Clone)]
enum Term {
    // A substring of the asset's searchable text.
    Text(String),
    // `L2`, `level2` or `level 2`: that level exactly, or the term's text as a substring.
    Level(u8, String),
}

impl BrowserQuery {
    /// Parses a search; case is ignored and an empty search matches every asset.
    pub(crate) fn parse(query: &str) -> Self {
        let lower = query.to_lowercase();
        let mut words = lower.split_whitespace().peekable();
        let mut terms = Vec::new();
        while let Some(word) = words.next() {
            if word == "level"
                && let Some(level) = words.peek().and_then(|next| next.parse::<u8>().ok())
            {
                let next = words.next().unwrap_or_default();
                terms.push(Term::Level(level, format!("level {next}")));
                continue;
            }
            let digits = word
                .strip_prefix("level")
                .or_else(|| word.strip_prefix('l'))
                .and_then(|digits| digits.parse::<u8>().ok());
            terms.push(match digits {
                Some(level) => Term::Level(level, word.to_owned()),
                None => Term::Text(word.to_owned()),
            });
        }
        Self { terms }
    }

    /// Whether the search has no terms, so every asset matches.
    pub(crate) fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    // Every term must match: a text term anywhere in the searchable text, a level term by the
    // building's level or as text.
    fn matches(&self, text: &str, level: u8) -> bool {
        self.terms.iter().all(|term| match term {
            Term::Text(word) => text.contains(word.as_str()),
            Term::Level(wanted, word) => {
                (level != 0 && *wanted == level) || text.contains(word.as_str())
            }
        })
    }
}

// The fields a search reads, lowercased and newline-separated so no term (which holds no
// whitespace) spans two fields: qualified id, display name, group heading, zone, density,
// service class, resource, asset set and tags.
fn searchable_text(id: &str, manifest: &AssetManifest, group: &BrowserGroup) -> String {
    let mut text = format!("{id}\n{}\n{}", manifest.display_name, group.label());
    if let Some(building) = &manifest.building {
        let fields = [
            building.zone_type.map(|zone| zone_label(zone as u8)),
            building.density_key(),
            building.service_class.as_deref(),
            building.extractor.as_ref().map(|e| e.resource.as_str()),
            building.field.as_ref().map(|f| f.resource.as_str()),
        ];
        for field in fields.into_iter().flatten() {
            text.push('\n');
            text.push_str(field);
        }
    }
    for field in manifest.asset_set.iter().chain(&manifest.tags) {
        text.push('\n');
        text.push_str(field);
    }
    text.to_lowercase()
}

// Display names compare with ASCII case folded, then exactly, so the order is total.
fn fold_cmp(a: &str, b: &str) -> Ordering {
    a.bytes()
        .map(|c| c.to_ascii_lowercase())
        .cmp(b.bytes().map(|c| c.to_ascii_lowercase()))
        .then_with(|| a.cmp(b))
}

// A row's identity inside one pack and group: its family, or its own id outside any family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum RowKey<'a> {
    Family(&'a str),
    Single(&'a str),
}

// A row's members with their display names, before they become chips.
type Members<'a> = Vec<(BrowserLevel<'a>, &'a str)>;
type Groups<'a> = BTreeMap<BrowserGroup<'a>, BTreeMap<RowKey<'a>, Members<'a>>>;

/// Builds the browser's packs, groups and rows for the assets matching `query`.
pub(crate) fn browser_packs<'a>(
    registry: &'a AssetRegistry,
    query: &BrowserQuery,
) -> Vec<BrowserPack<'a>> {
    let mut packs: BTreeMap<&str, Groups> = BTreeMap::new();
    for id in registry.qualified_ids() {
        let Some(entry) = registry.get(id) else {
            continue;
        };
        let manifest = &entry.manifest;
        let group = BrowserGroup::of(manifest);
        let level = manifest.building.as_ref().map_or(0, |b| b.level);
        if !query.is_empty() && !query.matches(&searchable_text(id, manifest, &group), level) {
            continue;
        }
        let key = match manifest.asset_set.as_deref().map(str::trim) {
            Some(set) if !set.is_empty() => RowKey::Family(set),
            _ => RowKey::Single(id),
        };
        packs
            .entry(entry.pack_id.as_str())
            .or_default()
            .entry(group)
            .or_default()
            .entry(key)
            .or_default()
            .push((BrowserLevel { level, id }, manifest.display_name.as_str()));
    }
    packs
        .into_iter()
        .map(|(pack_id, groups)| {
            let groups: Vec<_> = groups.into_iter().map(group_rows).collect();
            BrowserPack {
                pack_id,
                asset_count: groups.iter().map(|group| group.asset_count).sum(),
                groups,
            }
        })
        .collect()
}

fn group_rows<'a>(
    (group, rows): (BrowserGroup<'a>, BTreeMap<RowKey<'a>, Members<'a>>),
) -> BrowserGroupRows<'a> {
    let mut rows: Vec<_> = rows
        .into_iter()
        .map(|(key, mut members)| {
            members.sort_unstable_by(|(a, _), (b, _)| a.level.cmp(&b.level).then(a.id.cmp(b.id)));
            BrowserRow {
                // A key exists only once a member was pushed under it.
                label: members[0].1,
                asset_set: match key {
                    RowKey::Family(set) => Some(set),
                    RowKey::Single(_) => None,
                },
                levels: members.into_iter().map(|(level, _)| level).collect(),
            }
        })
        .collect();
    rows.sort_unstable_by(|a, b| {
        fold_cmp(a.label, b.label).then(a.levels[0].id.cmp(b.levels[0].id))
    });
    BrowserGroupRows {
        group,
        asset_count: rows.iter().map(|row| row.levels.len()).sum(),
        rows,
    }
}

/// The groups the game names without any asset existing, in group order and without
/// repeats: each zoning profile's zone and density (`zoned`), every service subtype the editor
/// creates, parking lots, and each resource an executable extractor or field profile produces.
/// Other explicit buildings, props, vehicles and characters have no such list.
pub(crate) fn catalogue_groups<'a>(
    zoned: impl IntoIterator<Item = (ZoneClass, &'a str)>,
    extractor_resources: impl IntoIterator<Item = &'a str>,
    field_resources: impl IntoIterator<Item = &'a str>,
) -> Vec<BrowserGroup<'a>> {
    let mut groups: Vec<_> = zoned
        .into_iter()
        .map(|(zone, density)| BrowserGroup::zoned(zone, density))
        .chain(
            SERVICES
                .iter()
                .map(|(class, _)| BrowserGroup::service(class)),
        )
        .chain([BrowserGroup::Parking])
        .chain(extractor_resources.into_iter().map(BrowserGroup::Extractor))
        .chain(field_resources.into_iter().map(BrowserGroup::Field))
        .collect();
    groups.sort_unstable();
    groups.dedup();
    groups
}

/// The resources the catalogue's executable profiles of `kind` produce: an extractor or field
/// asset authored for any other resource has no profile to run (`authoring::profile_matches`).
pub(crate) fn profile_resources(
    catalog: &RuntimeEconomyCatalog,
    kind: EconomyProfileRuntimeKind,
) -> impl Iterator<Item = &str> {
    catalog
        .all_profiles()
        .iter()
        .filter(move |p| p.kind == kind && p.runtime_supported && p.worker_capacity > 0)
        .flat_map(|profile| &profile.outputs)
        .filter_map(|port| catalog.resource_id_for_runtime_id(port.resource_runtime_id))
}

/// The `catalogue` groups no registered asset of any pack is in, in group order. `O(N log G)`
/// for `N` registered assets and `G` catalogue groups.
pub(crate) fn missing_groups<'a>(
    registry: &AssetRegistry,
    mut catalogue: Vec<BrowserGroup<'a>>,
) -> Vec<BrowserGroup<'a>> {
    catalogue.sort_unstable();
    let mut stocked = vec![false; catalogue.len()];
    for id in registry.qualified_ids() {
        let Some(entry) = registry.get(id) else {
            continue;
        };
        let group = BrowserGroup::of(&entry.manifest);
        if let Ok(index) = catalogue.binary_search_by(|probe| probe.cmp(&group)) {
            stocked[index] = true;
        }
    }
    catalogue
        .into_iter()
        .zip(stocked)
        .filter_map(|(group, stocked)| (!stocked).then_some(group))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // A valid manifest with one mesh part and the main entrance on a 2 x 2 lot; `fields` are the
    // `[building]` keys beyond the lot, `head` the top-level keys beyond id and name.
    fn building(id: &str, name: &str, head: &str, fields: &str) -> AssetManifest {
        AssetManifest::from_str(&format!(
            r#"asset_id = "{id}"
display_name = "{name}"
{head}
[building]
lot_width_cells = 2
lot_depth_cells = 2
{fields}
[[mesh_parts]]
name = "main"
[[mesh_parts.lods]]
file = "model.glb"
distance_min_m = 0.0
[[anchors]]
type = "entrance"
name = "main"
position = [0.0, 0.0, 1.0]
forward = [0.0, 0.0, 1.0]
"#
        ))
        .unwrap()
    }

    fn house(id: &str, name: &str, set: Option<&str>, density: &str, level: u8) -> AssetManifest {
        let head = set.map_or(String::new(), |set| format!("asset_set = \"{set}\""));
        let fields = format!(
            "zone_type = \"residential\"\ndensity = \"{density}\"\nlevel = {level}\nhousehold_capacity = 1"
        );
        building(id, name, &head, &fields)
    }

    fn explicit(id: &str, name: &str, fields: &str) -> AssetManifest {
        let fields = format!("placement_mode = \"explicit\"\nworker_capacity = 0\n{fields}");
        building(id, name, "", &fields)
    }

    fn registry(assets: Vec<(&str, AssetManifest)>) -> AssetRegistry {
        let mut registry = AssetRegistry::new();
        for (pack, manifest) in assets {
            registry.register(pack, manifest, String::new());
        }
        registry
    }

    // A row as its label and level chips.
    type Row = (String, Vec<u8>);

    // Each row, by group heading.
    fn rows(packs: &[BrowserPack]) -> Vec<(String, Vec<Row>)> {
        packs
            .iter()
            .flat_map(|pack| &pack.groups)
            .map(|group| {
                let rows = group.rows.iter().map(|row| {
                    (
                        row.label.to_owned(),
                        row.levels.iter().map(|l| l.level).collect(),
                    )
                });
                (group.group.label(), rows.collect())
            })
            .collect()
    }

    fn search(registry: &AssetRegistry, query: &str) -> Vec<String> {
        browser_packs(registry, &BrowserQuery::parse(query))
            .iter()
            .flat_map(|pack| &pack.groups)
            .flat_map(|group| &group.rows)
            .flat_map(|row| &row.levels)
            .map(|level| level.id.to_owned())
            .collect()
    }

    #[test]
    fn family_levels_share_one_row_and_unfamilied_namesakes_keep_their_own() {
        let registry = registry(vec![
            (
                "kuopio",
                house(
                    "house_1_level_2",
                    "House 1",
                    Some("kuopio_house_1"),
                    "low",
                    2,
                ),
            ),
            (
                "kuopio",
                house("house_1", "House 1", Some("kuopio_house_1"), "low", 1),
            ),
            ("kuopio", house("cottage_2", "Cottage", None, "low", 2)),
            ("kuopio", house("cottage", "Cottage", None, "low", 1)),
        ]);
        let packs = browser_packs(&registry, &BrowserQuery::default());
        assert_eq!(
            rows(&packs),
            [(
                "Residential · low density".to_owned(),
                vec![
                    ("Cottage".to_owned(), vec![1]),
                    ("Cottage".to_owned(), vec![2]),
                    ("House 1".to_owned(), vec![1, 2]),
                ]
            )]
        );
        let family = &packs[0].groups[0].rows[2];
        assert_eq!(family.asset_set, Some("kuopio_house_1"));
        assert_eq!(family.levels[0].id, "kuopio:house_1");
        assert_eq!(family.levels[1].id, "kuopio:house_1_level_2");
        // Counts are of assets, not rows.
        assert_eq!(
            (packs[0].asset_count, packs[0].groups[0].asset_count),
            (4, 4)
        );
    }

    #[test]
    fn groups_follow_manifest_fields_in_fixed_order() {
        let registry = registry(vec![
            (
                "base",
                explicit("b.lot", "Lot", "service_class = \"parking\""),
            ),
            (
                "base",
                explicit("b.station", "Station", "service_class = \"fire\""),
            ),
            (
                "base",
                explicit("b.plant", "Plant", "service_class = \"power\""),
            ),
            ("base", explicit("b.monument", "Monument", "")),
            (
                "base",
                explicit(
                    "b.mine",
                    "Mine",
                    "economy_profile = \"coal_mine_basic\"\n[building.extractor]\nresource = \"coal\"\narea_mode = \"player_polygon\"",
                ),
            ),
            // The id says commercial; the manifest says residential, and the manifest wins.
            (
                "base",
                house("building.commercial.flat", "Flat", None, "high", 1),
            ),
            ("base", house("b.house", "House", None, "low", 1)),
            ("base", house("b.terrace", "Terrace", None, "medium", 1)),
            ("addon", house("b.house", "Addon house", None, "low", 1)),
        ]);
        let labels: Vec<_> = browser_packs(&registry, &BrowserQuery::default())
            .iter()
            .map(|pack| {
                let groups = pack.groups.iter().map(|group| group.group.label());
                (pack.pack_id, groups.collect::<Vec<_>>())
            })
            .collect();
        assert_eq!(
            labels,
            [
                ("addon", vec!["Residential · low density".to_owned()]),
                (
                    "base",
                    [
                        "Residential · low density",
                        "Residential · medium density",
                        "Residential · high density",
                        "Service · Power",
                        "Service · Fire",
                        "Parking lots",
                        "Resource extractor · coal",
                        "Other explicit buildings",
                    ]
                    .map(str::to_owned)
                    .to_vec()
                ),
            ]
        );
        let station = BrowserGroup::of(&registry.get("base:b.station").unwrap().manifest);
        assert_eq!(station.creation_type(), "service");
        let flat = BrowserGroup::of(
            &registry
                .get("base:building.commercial.flat")
                .unwrap()
                .manifest,
        );
        assert_eq!(flat.creation_type(), "residential");
    }

    #[test]
    fn rows_sort_by_display_name_then_id() {
        let registry = registry(vec![
            ("base", house("b.z", "apartment", None, "low", 1)),
            ("base", house("b.y", "Bungalow", None, "low", 1)),
            ("base", house("b.x", "Apartment", None, "low", 1)),
            ("base", house("b.w", "Apartment", None, "low", 1)),
        ]);
        let ids = search(&registry, "");
        assert_eq!(ids, ["base:b.w", "base:b.x", "base:b.z", "base:b.y"]);
    }

    #[test]
    fn search_matches_level_zone_density_family_and_tags() {
        let mut tagged = house("b.villa", "Villa", None, "medium", 1);
        tagged.tags = vec!["Lakeside".to_owned()];
        let registry = registry(vec![
            (
                "kuopio",
                house("b.house_1", "House 1", Some("kuopio_house_1"), "low", 1),
            ),
            (
                "kuopio",
                house("b.house_1_l2", "House 1", Some("kuopio_house_1"), "low", 2),
            ),
            ("kuopio", house("b.house_12", "House 12", None, "low", 12)),
            ("kuopio", tagged),
            (
                "kuopio",
                explicit("b.station", "Station", "service_class = \"fire\""),
            ),
        ]);
        // A level term matches that level exactly, not level 12.
        for query in ["L2", "level 2", "Level2", "house l2"] {
            assert_eq!(search(&registry, query), ["kuopio:b.house_1_l2"], "{query}");
        }
        assert_eq!(
            search(&registry, "kuopio_house_1"),
            ["kuopio:b.house_1", "kuopio:b.house_1_l2"]
        );
        assert_eq!(search(&registry, "medium density"), ["kuopio:b.villa"]);
        assert_eq!(search(&registry, "lakeside"), ["kuopio:b.villa"]);
        assert_eq!(search(&registry, "residential low").len(), 3);
        assert_eq!(search(&registry, "fire"), ["kuopio:b.station"]);
        // The id and display name still match as before.
        assert_eq!(
            search(&registry, "kuopio:b.house_12"),
            ["kuopio:b.house_12"]
        );
        assert_eq!(search(&registry, "house 12"), ["kuopio:b.house_12"]);
        assert!(search(&registry, "commercial").is_empty());
        // A partial family match keeps the row, with only its matching levels.
        let packs = browser_packs(&registry, &BrowserQuery::parse("l2"));
        assert_eq!(packs[0].asset_count, 1);
        assert_eq!(packs[0].groups[0].rows[0].asset_set, Some("kuopio_house_1"));
    }

    // Every zone and density of the built-in zoning profiles, as the bridge passes them.
    fn profile_groups(
        profiles: &crate::simulation::zoning::ZoningProfileRegistry,
    ) -> Vec<BrowserGroup<'_>> {
        use crate::simulation::buildings::allocator::zone_type_to_zone_class;
        let zoned = profiles.profiles().iter().filter_map(|profile| {
            zone_type_to_zone_class(profile.zone_type).map(|zone| (zone, profile.density.as_str()))
        });
        catalogue_groups(zoned, ["coal"], ["grain"])
    }

    #[test]
    fn groups_no_pack_stocks_are_missing_until_one_does() {
        let profiles = crate::simulation::zoning::load_builtin_profile_registry().unwrap();
        let mut registry = registry(vec![("base", house("b.house", "House", None, "low", 1))]);
        let labels = |registry: &AssetRegistry| -> Vec<String> {
            missing_groups(registry, profile_groups(&profiles))
                .iter()
                .map(BrowserGroup::label)
                .collect()
        };
        let missing = labels(&registry);
        let zoned: Vec<_> = missing
            .iter()
            .filter(|label| label.contains("density"))
            .collect();
        assert_eq!(
            zoned,
            [
                "Residential · medium density",
                "Residential · high density",
                "Commercial · low density",
                "Commercial · medium density",
                "Commercial · high density",
                "Industrial · low density",
                "Industrial · medium density",
                "Industrial · high density",
            ]
        );
        assert!(missing.contains(&"Parking lots".to_owned()));
        assert!(missing.contains(&"Service · Fire".to_owned()));
        assert!(missing.contains(&"Resource extractor · coal".to_owned()));
        assert!(missing.contains(&"Farm · grain".to_owned()));

        // An asset in another pack still fills the group: emptiness is across all packs.
        for (id, density) in [("b.terrace", "medium"), ("b.tower", "high")] {
            registry.register("addon", house(id, "Home", None, density, 2), String::new());
        }
        for zone in ["commercial", "industrial"] {
            for density in ["low", "medium", "high"] {
                let fields =
                    format!("zone_type = \"{zone}\"\ndensity = \"{density}\"\nworker_capacity = 2");
                let id = format!("b.{zone}_{density}");
                registry.register("base", building(&id, "Work", "", &fields), String::new());
            }
        }
        registry.register(
            "base",
            explicit("b.lot", "Lot", "service_class = \"parking\""),
            String::new(),
        );
        let missing = labels(&registry);
        assert!(
            missing.iter().all(|label| !label.contains("density")),
            "{missing:?}"
        );
        assert!(!missing.contains(&"Parking lots".to_owned()));
        assert!(missing.contains(&"Service · Fire".to_owned()));
    }
}
