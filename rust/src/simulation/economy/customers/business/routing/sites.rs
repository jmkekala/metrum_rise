// SPDX-License-Identifier: GPL-2.0-only

//! Prepared replacement/project buyer doors over the existing customer bucket index.

use super::*;
use crate::simulation::buildings::allocator::DemandSiteEntranceKey;
use std::ops::Range;

/// One caller-selected buyer recipe and actual door, prepared before candidate proofs.
#[derive(Clone, Debug)]
pub(crate) struct PreparedBusinessSite {
    owner: usize,
    key: String,
    profile: u16,
    pub(super) entrance: BuildingEntrance,
    entrance_key: Option<DemandSiteEntranceKey>,
    // Door chunk and car components; valid while the network topology is unchanged.
    access: Option<Access>,
}

impl PreparedBusinessSite {
    /// Associates a candidate's stable commitment owner with its recipe and proposed door.
    pub(crate) fn new(owner: usize, key: &str, profile: u16, entrance: BuildingEntrance) -> Self {
        Self {
            owner,
            key: key.to_owned(),
            profile,
            entrance,
            entrance_key: None,
            access: None,
        }
    }

    /// Commitment owner; rows sort by owner, then [`Self::key`].
    pub(crate) fn owner(&self) -> usize {
        self.owner
    }

    /// Recipe key, unique within one owner.
    pub(crate) fn key(&self) -> &str {
        &self.key
    }

    /// Renumbers a retained row whose owner namespace shifted; geometry stays valid.
    pub(crate) fn set_owner(&mut self, owner: usize) {
        self.owner = owner;
    }

    /// Records exact geometry inputs for reuse after unrelated live-layout changes.
    pub(crate) fn with_entrance_key(mut self, key: DemandSiteEntranceKey) -> Self {
        self.entrance_key = Some(key);
        self
    }

    /// Searches the previous sorted sites without allocating or scanning other owners.
    pub(crate) fn retained_entrance<'a>(
        sites: &'a [Self],
        owner: usize,
        key: &str,
    ) -> Option<(DemandSiteEntranceKey, &'a BuildingEntrance)> {
        let index = sites
            .binary_search_by(|site| site.owner.cmp(&owner).then(site.key.as_str().cmp(key)))
            .ok()?;
        let site = &sites[index];
        Some((site.entrance_key?, &site.entrance))
    }
}

/// Cold geometry belongs to selected sites; funding and tranche changes retain it.
#[derive(Clone, Debug, Default)]
pub(super) struct PreparedSites {
    // Caller revision of `entries`; survives live forks so the caller can update changed rows.
    key: Option<Vec<u64>>,
    // Live layout revision of the derived ports and indices below.
    pub(super) layout: Option<Option<(u64, u64, u32)>>,
    // Network topology of every entry's cached access.
    access_topology: Option<u32>,
    pub(super) entries: Vec<PreparedBusinessSite>,
    pub(super) ports: Vec<Range<usize>>,
}

impl PreparedSites {
    /// Keeps prepared rows for incremental updates while live ports are renumbered.
    pub(super) fn invalidate_layout(&mut self) {
        self.layout = None;
        self.ports.clear();
    }
}

impl BusinessMarket {
    /// Prepares caller-selected recipes/doors before owner and account storage. A changed
    /// caller revision passes the previous rows and their revision to `factory`, which may
    /// regenerate only changed owners; rows must come back sorted by owner and key.
    /// A changed live layout re-derives ports and indices from the retained rows.
    /// Layout work is O(prepared ports log ports + live ports) plus access for new rows;
    /// unchanged revisions are O(1). Finance never rebuilds buckets inside a proposal.
    pub(crate) fn prepare_sites(
        &mut self,
        caller_revision: &[u64],
        catalog: &RuntimeEconomyCatalog,
        components: &ModeComponentIndex,
        graph: &RegionGraph,
        factory: impl FnOnce(Vec<PreparedBusinessSite>, Option<&[u64]>) -> Vec<PreparedBusinessSite>,
    ) {
        debug_assert!(!self.budgets.revalidation.is_active() && self.budgets.staged.is_empty());
        let entries_current = self.sites.key.as_deref() == Some(caller_revision);
        if entries_current && self.sites.layout == Some(self.revision) {
            return;
        }
        let timing = crate::debug::Channel::EconomyPhases
            .enabled()
            .then(std::time::Instant::now);
        if !entries_current {
            let previous = std::mem::take(&mut self.sites.entries);
            self.sites.entries = factory(previous, self.sites.key.as_deref());
            self.sites.key = Some(caller_revision.to_vec());
            crate::debug::work::add(crate::debug::work::Work::BuyerSiteRebuilds, 1);
            // Linear for the already-ordered rows the planner returns.
            self.sites
                .entries
                .sort_unstable_by(|a, b| a.owner.cmp(&b.owner).then(a.key.cmp(&b.key)));
            assert!(
                self.sites
                    .entries
                    .windows(2)
                    .all(|pair| (pair[0].owner, &pair[0].key) != (pair[1].owner, &pair[1].key)),
                "prepared buyer keys must be unique"
            );
        }
        let factory_ms = timing.map_or(0.0, |start| start.elapsed().as_secs_f64() * 1000.0);
        // Components follow the network topology; door chunks follow the retained entrance.
        let topology = self.revision.map(|own| own.2);
        if self.sites.access_topology != topology {
            self.sites
                .entries
                .iter_mut()
                .for_each(|site| site.access = None);
            self.sites.access_topology = topology;
        }
        self.sites
            .entries
            .par_iter_mut()
            .filter(|site| site.access.is_none())
            .for_each(|site| {
                site.access = Some(Access {
                    chunk: chunk_for_point(site.entrance.door_pos.x, site.entrance.door_pos.y),
                    components: components.entrance_components(
                        &site.entrance,
                        graph,
                        TransitFlags::CAR,
                    ),
                });
            });
        let live_ports = self.buyer_ports.last().map_or(0, |ports| ports.end);
        self.identities.truncate(live_ports);
        self.port_access.truncate(live_ports);
        self.port_sites.truncate(live_ports);
        self.sites.ports.clear();
        self.sites.ports.reserve(self.sites.entries.len());
        // Prefix ranges depend on preceding sites; preparation fixes their quote order.
        for (index, site) in self.sites.entries.iter().enumerate() {
            let first = self.identities.len();
            if let Some(profile) = catalog.profile_by_runtime_id(site.profile) {
                self.identities.extend(
                    profile
                        .inputs
                        .iter()
                        .map(|input| (site.owner, input.resource_runtime_id)),
                );
                self.port_access.extend(std::iter::repeat_n(
                    site.access.unwrap(),
                    profile.inputs.len(),
                ));
                self.port_sites
                    .extend(std::iter::repeat_n(Some(index), profile.inputs.len()));
            }
            self.sites.ports.push(first..self.identities.len());
        }
        self.rebuild_indices(catalog.resource_count() + 1, true);
        self.reset_site_ports(live_ports);
        self.sites.layout = Some(self.revision);
        if let Some(start) = timing {
            crate::debug_log!(
                "economy_phases",
                "buyer_sites_detail entries={} rows_changed={} factory_ms={:.3} layout_ms={:.3}",
                self.sites.entries.len(),
                !entries_current,
                factory_ms,
                start.elapsed().as_secs_f64() * 1000.0 - factory_ms
            );
        }
    }

    // No account maps to a proposed-site port between proposals: the live pass has restored
    // its books and an execution fork has just reset every account. Live ports keep their
    // availability and finance masks; site ports start empty. O(site ports).
    fn reset_site_ports(&mut self, live_ports: usize) {
        let ports = self.identities.len();
        self.port_available.truncate(live_ports);
        self.port_masks.truncate(live_ports);
        self.port_available.resize(ports, 0);
        self.port_masks.resize(ports, 0);
        self.visited.resize(ports, 0);
        debug_assert!(
            self.account_ports
                .iter()
                .all(|&port| port < live_ports || port == usize::MAX)
        );
        self.indices.sites.par_iter_mut().for_each(|index| {
            index.reset_available(|_| false);
            index.reset_query_masks(|_| 0);
        });
    }

    /// Finds a prepared owner/key pair in O(log sites), without allocation or a city scan.
    pub(crate) fn prepared_site(&self, owner: usize, key: &str) -> Option<usize> {
        self.sites
            .entries
            .binary_search_by(|site| site.owner.cmp(&owner).then(site.key.as_str().cmp(key)))
            .ok()
    }

    /// Resolves a queued recipe at its retained actual door without copying an asset name.
    /// O(log sites + this owner's authored alternatives); unrelated parcels are not visited.
    pub(crate) fn prepared_site_at(
        &self,
        owner: usize,
        profile: u16,
        entrance: &BuildingEntrance,
    ) -> Option<usize> {
        let first = self
            .sites
            .entries
            .partition_point(|site| site.owner < owner);
        self.sites.entries[first..]
            .iter()
            .take_while(|site| site.owner == owner)
            .position(|site| site.profile == profile && site.entrance == *entrance)
            .map(|offset| first + offset)
    }

    /// Replaces financed demand at the prepared candidate's actual door and recipe.
    /// The enclosing journal restores previous account-to-port mappings on rejection.
    /// Test harness: runtime sites reseed through `reprice_buyer_inputs`/`seed_new_buyer_inputs`.
    #[cfg(test)]
    pub(crate) fn replace_buyer_at_site(
        &mut self,
        site: usize,
        headroom: [f64; 2],
        input_tranches: usize,
        seed: impl FnOnce(&mut BusinessBudgets),
        affected: impl FnMut(usize, u16, f64, f64),
    ) -> Result<(), usize> {
        let owner = self.sites.entries[site].owner;
        self.replace_inputs_at(owner, Some(site), headroom, input_tranches, seed, affected)
    }

    /// Rebuilds the live-port or proposed-site resource/component/chunk indices, each only
    /// when its own geometry changes. O(ports log ports) over the selected port range.
    pub(super) fn rebuild_indices(&mut self, resources: usize, sites: bool) {
        self.collect_index_entries(resources, sites);
        self.build_index_resources(0..resources, sites);
    }

    /// [`Self::rebuild_indices`]'s first phase (`SIM-02-K`): each resource's bucket entries of
    /// the selected ports, into storage kept between rebuilds. O(ports), serial.
    pub(super) fn collect_index_entries(&mut self, resources: usize, sites: bool) {
        #[cfg(test)]
        {
            self.index_builds += 1;
        }
        let ports = if sites {
            self.indices.live_ports..self.identities.len()
        } else {
            0..self.indices.live_ports
        };
        self.index_entries.resize_with(resources, Vec::new);
        self.index_entries.iter_mut().for_each(Vec::clear);
        for port in ports {
            let access = self.port_access[port];
            access.components.append_bucket_entries(
                &mut self.index_entries[self.identities[port].1 as usize],
                access.chunk,
                port,
            );
        }
        let indices = if sites {
            &mut self.indices.sites
        } else {
            &mut self.indices.live
        };
        indices.resize_with(resources, Default::default);
    }

    /// Builds the indices of resources `range` from the collected entries, in parallel
    /// (`SIM-02-K`). O(entries log entries) for those resources.
    pub(super) fn build_index_resources(&mut self, range: std::ops::Range<usize>, sites: bool) {
        let indices = if sites {
            &mut self.indices.sites
        } else {
            &mut self.indices.live
        };
        indices[range.clone()]
            .par_iter_mut()
            .zip(&mut self.index_entries[range])
            .for_each(|(index, entries)| {
                *index = ReachableBucketIndex::from_entries(std::mem::take(entries));
                index.enable_reservations();
            });
    }

    /// Snapshots only overwritten mappings; appended rows disappear with account rollback.
    pub(super) fn set_account_port(&mut self, slot: usize, port: usize) {
        if self.account_ports[slot] == port {
            return;
        }
        if self.port_checkpoint.is_some_and(|len| slot < len)
            && self.port_changed[slot] != self.port_generation
        {
            self.port_changed[slot] = self.port_generation;
            self.previous_ports[slot] = self.account_ports[slot];
        }
        let pass = &mut self.pass_ports;
        if pass.checkpoint.is_some_and(|len| slot < len) && pass.seen[slot] != pass.generation {
            pass.seen[slot] = pass.generation;
            pass.saved.push((slot, self.account_ports[slot]));
        }
        self.account_ports[slot] = port;
    }
}
