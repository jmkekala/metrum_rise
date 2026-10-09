// SPDX-License-Identifier: GPL-2.0-only

//! Prepared replacement/project buyer doors over the existing customer bucket index.

use super::*;
use std::ops::Range;

/// One caller-selected buyer recipe and actual door, prepared before candidate proofs.
#[derive(Clone, Debug)]
pub(crate) struct PreparedBusinessSite {
    owner: usize,
    key: String,
    profile: u16,
    pub(super) entrance: BuildingEntrance,
}

impl PreparedBusinessSite {
    /// Associates a candidate's stable commitment owner with its recipe and proposed door.
    pub(crate) fn new(owner: usize, key: &str, profile: u16, entrance: BuildingEntrance) -> Self {
        Self { owner, key: key.to_owned(), profile, entrance }
    }
}

/// Cold geometry belongs to selected sites; funding and tranche changes retain it.
#[derive(Clone, Debug, Default)]
pub(super) struct PreparedSites {
    revision: Option<(Option<(u64, u64, u32)>, [u64; 3])>,
    pub(super) entries: Vec<PreparedBusinessSite>,
    pub(super) ports: Vec<Range<usize>>,
}

impl BusinessMarket {
    /// Prepares caller-selected recipes/doors before owner and account storage. The caller
    /// revision covers changes in its eligible sites; live geometry has its own revision.
    /// Cold work is O(prepared ports log ports + live ports); unchanged geometry is O(1).
    /// Finance never creates spatial membership or rebuilds buckets inside a proposal.
    pub(crate) fn prepare_sites(&mut self, caller_revision: [u64; 3],
        catalog: &RuntimeEconomyCatalog, components: &ModeComponentIndex, graph: &RegionGraph,
        factory: impl FnOnce() -> Vec<PreparedBusinessSite>) {
        debug_assert!(!self.budgets.revalidation.is_active() && self.budgets.staged.is_empty());
        let revision = (self.revision, caller_revision);
        if self.sites.revision == Some(revision) { return; }
        let live_ports = self.buyer_ports.last().map_or(0, |ports| ports.end);
        self.identities.truncate(live_ports);
        self.port_access.truncate(live_ports);
        self.port_sites.truncate(live_ports);
        self.sites.entries = factory();
        self.sites.entries.sort_unstable_by(|a, b| a.owner.cmp(&b.owner).then(a.key.cmp(&b.key)));
        assert!(self.sites.entries.windows(2).all(|pair|
            pair[0].owner != pair[1].owner || pair[0].key != pair[1].key), "prepared buyer keys must be unique");
        self.sites.ports.clear();
        self.sites.ports.reserve(self.sites.entries.len());
        // Prefix ranges depend on preceding sites; preparation fixes their quote order.
        for (index, site) in self.sites.entries.iter().enumerate() {
            let first = self.identities.len();
            if let Some(profile) = catalog.profile_by_runtime_id(site.profile) {
                let access = Access {
                    chunk: chunk_for_point(site.entrance.door_pos.x, site.entrance.door_pos.y),
                    components: components.entrance_components(&site.entrance, graph, TransitFlags::CAR),
                };
                self.identities.extend(profile.inputs.iter().map(|input| (site.owner, input.resource_runtime_id)));
                self.port_access.extend(std::iter::repeat_n(access, profile.inputs.len()));
                self.port_sites.extend(std::iter::repeat_n(Some(index), profile.inputs.len()));
            }
            self.sites.ports.push(first..self.identities.len());
        }
        self.rebuild_indices(catalog.resource_count() + 1);
        self.reset_accounts();
        self.sites.revision = Some(revision);
    }

    /// Finds a prepared owner/key pair in O(log sites), without allocation or a city scan.
    pub(crate) fn prepared_site(&self, owner: usize, key: &str) -> Option<usize> {
        self.sites.entries.binary_search_by(|site| site.owner.cmp(&owner).then(site.key.as_str().cmp(key))).ok()
    }

    /// Resolves a queued recipe at its retained actual door without copying an asset name.
    /// O(log sites + this owner's authored alternatives); unrelated parcels are not visited.
    pub(crate) fn prepared_site_at(&self, owner: usize, profile: u16,
        entrance: &BuildingEntrance) -> Option<usize> {
        let first = self.sites.entries.partition_point(|site| site.owner < owner);
        self.sites.entries[first..].iter().take_while(|site| site.owner == owner)
            .position(|site| site.profile == profile && site.entrance == *entrance)
            .map(|offset| first + offset)
    }

    /// Replaces financed demand at the prepared candidate's actual door and recipe.
    /// The enclosing journal restores previous account-to-port mappings on rejection.
    /// Test harness: runtime sites reseed through `reprice_buyer_inputs`/`seed_new_buyer_inputs`.
    #[cfg(test)]
    pub(crate) fn replace_buyer_at_site(&mut self, site: usize, headroom: [f64; 2],
        input_tranches: usize, seed: impl FnOnce(&mut BusinessBudgets),
        affected: impl FnMut(usize, u16, f64, f64)) -> Result<(), usize> {
        let owner = self.sites.entries[site].owner;
        self.replace_inputs_at(owner, Some(site), headroom, input_tranches, seed, affected)
    }

    /// Rebuilds the same resource/component/chunk indices only when site geometry changes.
    pub(super) fn rebuild_indices(&mut self, resources: usize) {
        #[cfg(test)]
        { self.index_builds += 1; }
        let mut entries: Vec<Vec<ReachableBucketEntry>> = (0..resources).map(|_| Vec::new()).collect();
        for (port, &(_, resource)) in self.identities.iter().enumerate() {
            let access = self.port_access[port];
            access.components.append_bucket_entries(&mut entries[resource as usize], access.chunk, port);
        }
        self.indices.resize_with(resources, Default::default);
        self.indices.par_iter_mut().zip(entries).for_each(|(index, entries)| {
            *index = ReachableBucketIndex::from_entries(entries);
            index.enable_reservations();
        });
    }

    /// Snapshots only overwritten mappings; appended rows disappear with account rollback.
    pub(super) fn set_account_port(&mut self, slot: usize, port: usize) {
        if self.account_ports[slot] == port { return; }
        if self.port_checkpoint.is_some_and(|len| slot < len)
            && self.port_changed[slot] != self.port_generation {
            self.port_changed[slot] = self.port_generation;
            self.previous_ports[slot] = self.account_ports[slot];
        }
        self.account_ports[slot] = port;
    }
}
