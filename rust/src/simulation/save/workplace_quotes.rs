// SPDX-License-Identifier: GPL-2.0-only

//! Persistence for complete workplace commute quotes.

use super::{SaveLoadResult, SnapshotMaps};
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::buildings::allocator::BuildingEntranceKey;
use crate::simulation::economy::households::HouseholdSystem;
use crate::simulation::network::TransitNetwork;
use rusqlite::{Connection, Transaction, params};
use std::cmp::Ordering;

const MAGIC: [u8; 4] = *b"WRC2";
// Magic, entry count (u64) and the workplace pass counter (u32).
const HEADER_BYTES: usize = 16;
// Little-endian home/work IDs (u64 each), car flag, presence flag, u16 seconds, then the u32
// pass that added the quote (`SIM-02-G`).
const ENTRY_BYTES: usize = 24;
type QuoteEntry = ((usize, usize, bool), Option<u16>, u32);

/// Writes the complete workplace quotes that were built against the current topology.
pub(super) fn save(
    tx: &Transaction<'_>,
    households: &HouseholdSystem,
    allocator: &BuildingAllocator,
    network: &TransitNetwork,
    maps: &SnapshotMaps,
) -> SaveLoadResult<()> {
    let (saved_building_revision, saved_entrance_revision, saved_cch_generation) =
        households.workplace_route_cache_revisions();
    let topology_matches = saved_building_revision == allocator.building_ref_revision()
        && saved_entrance_revision == allocator.entrance_ref_revision()
        && saved_cch_generation == network.cch_graph.build_generation;
    let mut entries: Vec<_> = if topology_matches {
        households.workplace_route_cache_entries().collect()
    } else {
        Vec::new()
    };
    if entries
        .iter()
        .any(|((home, work, _), _, _)| *home >= maps.building_count || *work >= maps.building_count)
    {
        return Err(super::SaveLoadError::custom(
            "workplace quote references missing building",
        ));
    }
    entries.sort_unstable_by(|left, right| compare_key(left.0, right.0));
    let blob = encode(&entries, households.workplace_route_cache_pass())?;
    let entrance_keys = serde_json::to_vec(
        &allocator
            .entrances
            .iter()
            .map(BuildingEntranceKey::new)
            .collect::<Vec<_>>(),
    )
    .map_err(|err| super::SaveLoadError::custom(err.to_string()))?;
    tx.execute(
        "INSERT INTO workplace_route_cache(singleton, quote_blob, entrance_keys_json) VALUES (1, ?1, ?2)",
        params![blob, entrance_keys],
    )?;
    Ok(())
}

/// Loads complete workplace quotes after the graph and entrance caches are rebuilt.
pub(super) fn load(
    conn: &Connection,
    households: &mut HouseholdSystem,
    allocator: &BuildingAllocator,
    network: &TransitNetwork,
    preserve: bool,
) -> SaveLoadResult<()> {
    if !preserve {
        return Ok(());
    }
    let row = conn.query_row(
        "SELECT quote_blob, entrance_keys_json FROM workplace_route_cache WHERE singleton = 1",
        [],
        |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?)),
    )?;
    let current_entrance_keys = serde_json::to_vec(
        &allocator
            .entrances
            .iter()
            .map(BuildingEntranceKey::new)
            .collect::<Vec<_>>(),
    )
    .map_err(|err| super::SaveLoadError::custom(err.to_string()))?;
    if row.1 != current_entrance_keys {
        return Ok(());
    }
    let (entries, pass) = decode(&row.0, allocator.buildings.len())?;
    households.restore_workplace_route_cache(
        allocator.building_ref_revision(),
        allocator.entrance_ref_revision(),
        network.cch_graph.build_generation,
        entries,
        pass,
    );
    Ok(())
}

fn compare_key(left: (usize, usize, bool), right: (usize, usize, bool)) -> Ordering {
    left.0
        .cmp(&right.0)
        .then_with(|| left.1.cmp(&right.1))
        .then_with(|| left.2.cmp(&right.2))
}

fn encode(entries: &[QuoteEntry], pass: u32) -> SaveLoadResult<Vec<u8>> {
    let entry_count = u64::try_from(entries.len())
        .map_err(|_| super::SaveLoadError::custom("workplace quote count overflows u64"))?;
    let capacity = HEADER_BYTES
        .checked_add(
            entries
                .len()
                .checked_mul(ENTRY_BYTES)
                .ok_or_else(|| super::SaveLoadError::custom("workplace quote blob too large"))?,
        )
        .ok_or_else(|| super::SaveLoadError::custom("workplace quote blob too large"))?;
    let mut blob = Vec::with_capacity(capacity);
    blob.extend_from_slice(&MAGIC);
    blob.extend_from_slice(&entry_count.to_le_bytes());
    blob.extend_from_slice(&pass.to_le_bytes());
    for &((home, work, has_car), value, stamp) in entries {
        blob.extend_from_slice(
            &u64::try_from(home)
                .map_err(|_| super::SaveLoadError::custom("home building overflows u64"))?
                .to_le_bytes(),
        );
        blob.extend_from_slice(
            &u64::try_from(work)
                .map_err(|_| super::SaveLoadError::custom("work building overflows u64"))?
                .to_le_bytes(),
        );
        blob.push(u8::from(has_car));
        match value {
            Some(seconds) => {
                blob.push(1);
                blob.extend_from_slice(&seconds.to_le_bytes());
            }
            None => {
                blob.push(0);
                blob.extend_from_slice(&0_u16.to_le_bytes());
            }
        }
        blob.extend_from_slice(&stamp.to_le_bytes());
    }
    Ok(blob)
}

fn decode(blob: &[u8], building_count: usize) -> SaveLoadResult<(Vec<QuoteEntry>, u32)> {
    if blob.len() < HEADER_BYTES || blob[..4] != MAGIC {
        return Err(super::SaveLoadError::custom(
            "invalid workplace quote blob header",
        ));
    }
    let count = usize::try_from(u64::from_le_bytes(
        blob[4..12]
            .try_into()
            .expect("workplace quote header has a fixed length"),
    ))
    .map_err(|_| super::SaveLoadError::custom("workplace quote count overflows usize"))?;
    let pass = u32::from_le_bytes(
        blob[12..16]
            .try_into()
            .expect("workplace quote header has a fixed length"),
    );
    let expected_len = HEADER_BYTES
        .checked_add(
            count
                .checked_mul(ENTRY_BYTES)
                .ok_or_else(|| super::SaveLoadError::custom("workplace quote blob too large"))?,
        )
        .ok_or_else(|| super::SaveLoadError::custom("workplace quote blob too large"))?;
    if blob.len() != expected_len {
        return Err(super::SaveLoadError::custom(
            "workplace quote blob length does not match entry count",
        ));
    }
    let max_entries = building_count
        .checked_mul(building_count)
        .and_then(|pairs| pairs.checked_mul(2))
        .ok_or_else(|| super::SaveLoadError::custom("building count overflows quote bound"))?;
    if count > max_entries {
        return Err(super::SaveLoadError::custom(
            "workplace quote count exceeds building pair bound",
        ));
    }
    let mut entries = Vec::with_capacity(count);
    let mut offset = HEADER_BYTES;
    let mut previous = None;
    for _ in 0..count {
        let home = usize::try_from(u64::from_le_bytes(
            blob[offset..offset + 8].try_into().unwrap(),
        ))
        .map_err(|_| super::SaveLoadError::custom("home building overflows usize"))?;
        let work = usize::try_from(u64::from_le_bytes(
            blob[offset + 8..offset + 16].try_into().unwrap(),
        ))
        .map_err(|_| super::SaveLoadError::custom("work building overflows usize"))?;
        if home >= building_count || work >= building_count {
            return Err(super::SaveLoadError::custom(
                "workplace quote references missing building",
            ));
        }
        let mode = blob[offset + 16];
        if mode > 1 {
            return Err(super::SaveLoadError::custom("invalid workplace quote mode"));
        }
        let key = (home, work, mode != 0);
        if previous.is_some_and(|previous| compare_key(previous, key) != Ordering::Less) {
            return Err(super::SaveLoadError::custom(
                "workplace quote keys are not strictly ordered",
            ));
        }
        let value = match blob[offset + 17] {
            0 => None,
            1 => Some(u16::from_le_bytes(
                blob[offset + 18..offset + 20].try_into().unwrap(),
            )),
            _ => {
                return Err(super::SaveLoadError::custom(
                    "invalid workplace quote value",
                ));
            }
        };
        let stamp = u32::from_le_bytes(blob[offset + 20..offset + 24].try_into().unwrap());
        if pass.wrapping_sub(stamp)
            >= crate::simulation::economy::households::WORKPLACE_QUOTE_PASSES
        {
            return Err(super::SaveLoadError::custom("expired workplace quote"));
        }
        previous = Some(key);
        entries.push((key, value, stamp));
        offset += ENTRY_BYTES;
    }
    Ok((entries, pass))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::buildings::allocator::{BuildingAllocator, BuildingEntrance};
    use crate::simulation::economy::households::HouseholdSystem;
    use crate::simulation::network::TransitNetwork;
    use rusqlite::Connection;

    #[test]
    fn quote_blob_roundtrips_sorted_entries() {
        let entries = vec![
            ((2, 4, false), None, 7),
            ((2, 4, true), Some(65535), 9),
            ((8, 1, false), Some(37), 8),
        ];
        let blob = encode(&entries, 9).unwrap();
        assert_eq!(decode(&blob, 9).unwrap(), (entries, 9));
    }

    #[test]
    fn quote_blob_rejects_bad_order_and_length() {
        let entries = vec![((1, 2, false), Some(4), 3), ((1, 2, true), None, 3)];
        let mut blob = encode(&entries, 3).unwrap();
        blob.pop();
        assert!(decode(&blob, 3).is_err());

        let mut blob = encode(&entries, 3).unwrap();
        blob[HEADER_BYTES + ENTRY_BYTES + 16] = 0;
        assert!(decode(&blob, 3).is_err());

        let mut blob = encode(&entries, 3).unwrap();
        blob[HEADER_BYTES + 16] = 2;
        assert!(decode(&blob, 3).is_err());

        let mut blob = encode(&entries, 3).unwrap();
        blob[HEADER_BYTES + 17] = 2;
        assert!(decode(&blob, 3).is_err());

        // A quote a day of passes old has expired.
        let blob = encode(&entries, 27).unwrap();
        assert!(decode(&blob, 3).is_err());
    }

    #[test]
    fn sqlite_roundtrip_restores_current_runtime_stamps() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE workplace_route_cache(
                singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
                quote_blob BLOB NOT NULL,
                entrance_keys_json BLOB NOT NULL
            )",
        )
        .unwrap();
        let mut households = HouseholdSystem::new();
        let allocator = test_allocator(2);
        let network = TransitNetwork::new();
        let entries = vec![
            ((0, 1, false), None, 4),
            ((0, 1, true), Some(65535), 6),
            ((1, 0, false), Some(37), 5),
        ];
        households.restore_workplace_route_cache(
            allocator.building_ref_revision(),
            allocator.entrance_ref_revision(),
            network.cch_graph.build_generation,
            entries.clone(),
            6,
        );
        let maps = SnapshotMaps {
            node_old_to_new: Default::default(),
            edge_old_to_new: Default::default(),
            building_count: 2,
        };
        let tx = conn.transaction().unwrap();
        save(&tx, &households, &allocator, &network, &maps).unwrap();
        tx.commit().unwrap();

        let mut restored_allocator = test_allocator(2);
        restored_allocator.bump_building_ref_revision();
        restored_allocator.bump_entrance_ref_revision();
        let mut restored_network = TransitNetwork::new();
        restored_network.cch_graph = crate::simulation::pathing::cch::CchGraph::build(
            &crate::simulation::network::graph::RegionGraph::new(),
        );
        let mut restored = HouseholdSystem::new();
        load(
            &conn,
            &mut restored,
            &restored_allocator,
            &restored_network,
            true,
        )
        .unwrap();
        assert_eq!(
            restored.workplace_route_cache_revisions(),
            (
                restored_allocator.building_ref_revision(),
                restored_allocator.entrance_ref_revision(),
                restored_network.cch_graph.build_generation,
            )
        );
        let mut restored_entries: Vec<_> = restored.workplace_route_cache_entries().collect();
        restored_entries.sort_unstable_by_key(|entry| entry.0);
        assert_eq!(restored_entries, entries);
    }

    #[test]
    fn changed_entrance_drops_quotes_but_unrelated_building_change_does_not() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE workplace_route_cache(
                singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
                quote_blob BLOB NOT NULL,
                entrance_keys_json BLOB NOT NULL
            )",
        )
        .unwrap();
        let mut households = HouseholdSystem::new();
        let allocator = test_allocator(2);
        let network = TransitNetwork::new();
        let entries = vec![((0, 1, false), Some(37), 0)];
        households.restore_workplace_route_cache(
            allocator.building_ref_revision(),
            allocator.entrance_ref_revision(),
            network.cch_graph.build_generation,
            entries.clone(),
            0,
        );
        let maps = SnapshotMaps {
            node_old_to_new: Default::default(),
            edge_old_to_new: Default::default(),
            building_count: 2,
        };
        let tx = conn.transaction().unwrap();
        save(&tx, &households, &allocator, &network, &maps).unwrap();
        tx.commit().unwrap();

        let mut unrelated_change = test_allocator(2);
        unrelated_change.buildings[0].daily_household_sales_value = 99.0;
        let unrelated_network = TransitNetwork::new();
        let mut unrelated_households = HouseholdSystem::new();
        load(
            &conn,
            &mut unrelated_households,
            &unrelated_change,
            &unrelated_network,
            true,
        )
        .unwrap();
        assert_eq!(
            unrelated_households
                .workplace_route_cache_entries()
                .collect::<Vec<_>>(),
            entries
        );

        let mut changed_entrance = test_allocator(2);
        changed_entrance.entrances[0].door_pos.x = 1.0;
        let changed_network = TransitNetwork::new();
        let mut changed_households = HouseholdSystem::new();
        load(
            &conn,
            &mut changed_households,
            &changed_entrance,
            &changed_network,
            true,
        )
        .unwrap();
        assert_eq!(
            changed_households.workplace_route_cache_entries().count(),
            0
        );
    }

    #[test]
    fn stale_runtime_stamps_drop_quotes_at_save() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE workplace_route_cache(
                singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
                quote_blob BLOB NOT NULL,
                entrance_keys_json BLOB NOT NULL
            )",
        )
        .unwrap();
        let mut households = HouseholdSystem::new();
        households.restore_workplace_route_cache(
            u64::MAX,
            u64::MAX,
            u32::MAX,
            vec![((0, 0, false), Some(12), 0)],
            0,
        );
        let allocator = BuildingAllocator::new();
        let network = TransitNetwork::new();
        let maps = SnapshotMaps {
            node_old_to_new: Default::default(),
            edge_old_to_new: Default::default(),
            building_count: 1,
        };
        let tx = conn.transaction().unwrap();
        save(&tx, &households, &allocator, &network, &maps).unwrap();
        tx.commit().unwrap();
        let blob: Vec<u8> = conn
            .query_row(
                "SELECT quote_blob FROM workplace_route_cache WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(decode(&blob, 1).unwrap().0.is_empty());
    }

    #[test]
    fn current_save_without_quote_table_is_rejected() {
        let conn = Connection::open_in_memory().unwrap();
        let mut households = HouseholdSystem::new();
        let allocator = BuildingAllocator::new();
        let network = TransitNetwork::new();
        assert!(load(&conn, &mut households, &allocator, &network, true,).is_err());
    }

    #[test]
    fn valid_cache_rejects_invalid_building_reference_at_save() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE workplace_route_cache(
                singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
                quote_blob BLOB NOT NULL,
                entrance_keys_json BLOB NOT NULL
            )",
        )
        .unwrap();
        let mut households = HouseholdSystem::new();
        let allocator = test_allocator(2);
        let network = TransitNetwork::new();
        households.restore_workplace_route_cache(
            allocator.building_ref_revision(),
            allocator.entrance_ref_revision(),
            network.cch_graph.build_generation,
            vec![((2, 0, false), Some(12), 0)],
            0,
        );
        let maps = SnapshotMaps {
            node_old_to_new: Default::default(),
            edge_old_to_new: Default::default(),
            building_count: 2,
        };
        let tx = conn.transaction().unwrap();
        assert!(save(&tx, &households, &allocator, &network, &maps).is_err());
    }

    fn test_allocator(count: usize) -> BuildingAllocator {
        let mut allocator = BuildingAllocator::new();
        for index in 0..count {
            allocator.buildings.push(
                crate::simulation::buildings::allocator::indexed_test_building(
                    format!("test:building_{index}"),
                    crate::simulation::zoning::ZoneType::Residential,
                    index as i32,
                ),
            );
        }
        allocator.entrances = vec![BuildingEntrance::default(); count];
        allocator
    }
}
