use godot::prelude::Vector3;
use super::data::RegionGraph;
use super::super::types::*;
use std::collections::HashMap;

impl RegionGraph {
    pub fn rebuild_adjacency_list(&mut self) {
        self.adjacency.clear();
        self.adjacency.resize(self.nodes.len(), Vec::new());
        for (i, e) in self.edges.iter().enumerate() {
            if e.deleted { continue; }
            self.adjacency[e.start_node as usize].push(i);
            self.adjacency[e.end_node as usize].push(i);
        }
    }

    /// Removes all edges marked as `deleted` and remaps all internal indices.
    /// Returns a mapping from [Old Edge Index] -> [New Edge Index].
    pub fn compact_edges(&mut self) -> HashMap<usize, usize> {
        let mut old_to_new = HashMap::new();
        let mut new_edges = Vec::new();

        for (old_idx, edge) in self.edges.iter().enumerate() {
            if !edge.deleted {
                let new_idx = new_edges.len();
                old_to_new.insert(old_idx, new_idx);
                new_edges.push(edge.clone());
            }
        }

        // If no edges were deleted, we're already compacted.
        if new_edges.len() == self.edges.len() {
            return HashMap::new();
        }

        self.edges = new_edges;

        // 1. Rebuild Adjacency List (Fastest way to update indices)
        self.rebuild_adjacency_list();

        // 2. Rebuild Spatial Index
        self.spatial_edge_rt = rstar::RTree::new();
        for i in 0..self.edges.len() {
            self.add_to_spatial_index(i);
        }

        // 3. Update Lane Connection rules inside each Node
        for node in &mut self.nodes {
            let mut new_lane_conns = HashMap::new();
            for (src, targets) in node.lane_connections.drain() {
                // If the source edge still exists, remap it
                if let Some(&new_src_idx) = old_to_new.get(&src.0) {
                    let mut new_targets = Vec::new();
                    for mut tgt in targets {
                        // If the target edge still exists, remap it
                        if let Some(&new_tgt_idx) = old_to_new.get(&tgt.0) {
                            tgt.0 = new_tgt_idx;
                            new_targets.push(tgt);
                        }
                    }
                    if !new_targets.is_empty() {
                        new_lane_conns.insert((new_src_idx, src.1), new_targets);
                    }
                }
            }
            node.lane_connections = new_lane_conns;
        }

        old_to_new
    }

    /// Returns the number of disconnected components (islands) in the network
    pub fn get_island_count(&self) -> usize {
        if self.nodes.is_empty() { return 0; }
        
        // Disjoint Set Union (DSU)
        let mut parent: Vec<usize> = (0..self.nodes.len()).collect();
        
        fn find(i: usize, parent: &mut Vec<usize>) -> usize {
            if parent[i] == i { return i; }
            parent[i] = find(parent[i], parent);
            parent[i]
        }
        
        fn unite(i: usize, j: usize, parent: &mut Vec<usize>) {
            let root_i = find(i, parent);
            let root_j = find(j, parent);
            if root_i != root_j {
                parent[root_i] = root_j;
            }
        }
        
        // Unite nodes connected by edges
        for edge in &self.edges {
            if edge.deleted { continue; }
            unite(edge.start_node as usize, edge.end_node as usize, &mut parent);
        }
        
        // Count unique roots (only for nodes that are part of an edge to avoid counting "floating" preview nodes)
        let mut active_nodes = std::collections::HashSet::new();
        for edge in &self.edges {
            if edge.deleted { continue; }
            active_nodes.insert(edge.start_node as usize);
            active_nodes.insert(edge.end_node as usize);
        }
        
        let mut roots = std::collections::HashSet::new();
        for &node_idx in &active_nodes {
            roots.insert(find(node_idx, &mut parent));
        }
        
        roots.len()
    }

    pub fn sync_to_terrain(&mut self, terrain: &crate::simulation::terrain::TerrainSystem) {
        let hw = (terrain.width as f32 - 1.0) * 0.5;
        let hh = (terrain.height as f32 - 1.0) * 0.5;
        
        // 0. Pre-calculate which nodes are snappable (Standard only)
        let mut node_snappable = vec![true; self.nodes.len()];
        for edge in &self.edges {
            if edge.deleted { continue; }
            if edge.class != EdgeClass::Standard {
                node_snappable[edge.start_node as usize] = false;
                node_snappable[edge.end_node as usize] = false;
            }
        }

        // 1. Sync Nodes Only if snappable
        for (i, node) in self.nodes.iter_mut().enumerate() {
            if !node_snappable[i] { continue; }
            let gx = node.pos.x + hw;
            let gz = node.pos.z + hh;
            node.pos.y = terrain.get_height_interpolated(gx, gz) * crate::config::HEIGHT_SCALE;
        }
        
        // 2. Re-interpolate Edge Geometry (Smooth Grades)
        for edge in &mut self.edges {
            if edge.deleted { continue; }
            if edge.class != EdgeClass::Standard { continue; }
            
            let count = edge.geometry.len();
            if count < 2 { continue; }
            
            // Snap endpoints to nodes
            edge.geometry[0] = self.nodes[edge.start_node as usize].pos;
            edge.geometry[count - 1] = self.nodes[edge.end_node as usize].pos;
            
            // HARMONIC CONFORMANCE (Laplacian Smoothing)
            // 1. Re-sample raw terrain for all intermediate points so road follows new hills
            for j in 1..count-1 {
                let gx = edge.geometry[j].x + hw;
                let gz = edge.geometry[j].z + hh;
                edge.geometry[j].y = terrain.get_height_interpolated(gx, gz) * crate::config::HEIGHT_SCALE;
            }

            // 2. Taubin Smoothing to iron out bumps without volume shrinkage
            let iters = 50;
            if count > 2 {
                let mut temp_h = vec![0.0; count];
                let lambda = 0.5;
                let mu = -0.53;
                for _ in 0..iters {
                    // Positive Pass (Shrink/Smooth)
                    for j in 1..count-1 {
                        let laplacian = 0.5 * (edge.geometry[j-1].y + edge.geometry[j+1].y) - edge.geometry[j].y;
                        temp_h[j] = edge.geometry[j].y + lambda * laplacian;
                    }
                    for j in 1..count-1 {
                        edge.geometry[j].y = temp_h[j];
                    }
                    // Negative Pass (Inflate/Restore Volume)
                    for j in 1..count-1 {
                        let laplacian = 0.5 * (edge.geometry[j-1].y + edge.geometry[j+1].y) - edge.geometry[j].y;
                        temp_h[j] = edge.geometry[j].y + mu * laplacian;
                    }
                    for j in 1..count-1 {
                        edge.geometry[j].y = temp_h[j];
                    }
                }
            }
        }
        self.rebuild_intersection_clips();
    }

    pub fn rebuild_intersection_clips(&mut self) {
        let mut connection_counts = HashMap::new();
        for edge in &self.edges {
            if edge.deleted || edge.primary_type != TransitType::Road { continue; }
            *connection_counts.entry(self.get_valid_node(edge.start_node)).or_insert(0) += 1;
            *connection_counts.entry(self.get_valid_node(edge.end_node)).or_insert(0) += 1;
        }

        // Calculate maximum road width and width difference for junction detection
        let mut node_max_width = HashMap::new();
        let mut node_min_width = HashMap::new();
        for edge in &self.edges {
            if edge.deleted || edge.primary_type != TransitType::Road { continue; }
            let w = edge.width;
            
            let s_node = self.get_valid_node(edge.start_node);
            let s_max = node_max_width.entry(s_node).or_insert(0.0);
            if w > *s_max { *s_max = w; }
            let s_min = node_min_width.entry(s_node).or_insert(f32::MAX);
            if w < *s_min { *s_min = w; }

            let e_node = self.get_valid_node(edge.end_node);
            let e_max = node_max_width.entry(e_node).or_insert(0.0);
            if w > *e_max { *e_max = w; }
            let e_min = node_min_width.entry(e_node).or_insert(f32::MAX);
            if w < *e_min { *e_min = w; }
        }


        let valid_node_ids: Vec<u32> = (0..self.nodes.len()).map(|i| self.get_valid_node(i as u32)).collect();

        self.junction_polygons.clear(); // Removing procedural intersection meshes

        for (_edge_id, edge) in self.edges.iter_mut().enumerate() {
            if edge.deleted || edge.primary_type != TransitType::Road { continue; }

            // Dynamic clipping: Ensure the junction covers at least the width of the road
            let s_valid = valid_node_ids[edge.start_node as usize];
            let e_valid = valid_node_ids[edge.end_node as usize];

            let s_conn = *connection_counts.get(&s_valid).unwrap_or(&0);
            let e_conn = *connection_counts.get(&e_valid).unwrap_or(&0);

            let s_max_w = *node_max_width.get(&s_valid).unwrap_or(&0.0);
            let s_min_w = *node_min_width.get(&s_valid).unwrap_or(&0.0);
            let s_different = (s_max_w - s_min_w).abs() > 0.1;

            let e_max_w = *node_max_width.get(&e_valid).unwrap_or(&0.0);
            let e_min_w = *node_min_width.get(&e_valid).unwrap_or(&0.0);
            let e_different = (e_max_w - e_min_w).abs() > 0.1;

            let s_clip = (s_max_w * 0.5 + crate::config::SIDEWALK_WIDTH) * 1.2;
            let e_clip = (e_max_w * 0.5 + crate::config::SIDEWALK_WIDTH) * 1.2;

            let s_node_type = self.nodes[s_valid as usize].node_type;
            let e_node_type = self.nodes[e_valid as usize].node_type;

            // Merge clip fixup is done in a second pass after merge detection below.
            edge.start_clip = if (s_conn >= 3 || s_different) && s_node_type == NodeType::Junction {
                s_clip
            } else {
                0.0
            };

            edge.end_clip = if (e_conn >= 3 || e_different) && e_node_type == NodeType::Junction {
                e_clip
            } else {
                0.0
            };

            let count = edge.geometry.len();
            if count >= 2 {
                let mut total_length = 0.0;
                for i in 0..count - 1 {
                    let p0 = edge.geometry[i];
                    let p1 = edge.geometry[i + 1];
                    total_length += (p1 - p0).length();
                }
                
                let num_segments = f32::max(1.0, f32::ceil(total_length / 2.0)) as usize;
                let mut resampled = Vec::new();
                
                for i in 0..=num_segments {
                    let dist = (i as f32 / num_segments as f32) * total_length;
                    let mut curr = 0.0;
                    let mut found = false;
                    for j in 0..count - 1 {
                        let p0 = edge.geometry[j];
                        let p1 = edge.geometry[j + 1];
                        let d = (p1 - p0).length();
                        if curr + d >= dist || (i == num_segments && j == count - 2) {
                            let t = if d > 1e-5 { (dist - curr) / d } else { 0.0 };
                            resampled.push(p0.lerp(p1, t.clamp(0.0, 1.0)));
                            found = true;
                            break;
                        }
                        curr += d;
                    }
                    if !found && !edge.geometry.is_empty() {
                         resampled.push(edge.geometry[count - 1]);
                    }
                }
                if !resampled.is_empty() {
                    let start_node_y = self.nodes[edge.start_node as usize].pos.y;
                    let end_node_y = self.nodes[edge.end_node as usize].pos.y;
                    resampled[0].y = start_node_y;
                    let last_idx = resampled.len() - 1;
                    resampled[last_idx].y = end_node_y;
                }
                edge.physical_geometry = resampled;
                edge.physical_length = total_length;
            } else {
                edge.physical_geometry = edge.geometry.clone();
                edge.physical_length = 0.0;
            }
        }

        // Post-resampling: detect Merge nodes and mark ramp edges using accurate physical_geometry.
        // Running this after resampling ensures curved ramps are detected correctly.
        {
            let mut tangent_at: HashMap<(usize, u32), Vector3> = HashMap::new();
            for (eid, edge) in self.edges.iter().enumerate() {
                if edge.deleted || edge.primary_type != TransitType::Road { continue; }
                let geom = &edge.physical_geometry;
                if geom.len() < 2 { continue; }
                let sn = valid_node_ids[edge.start_node as usize];
                let en = valid_node_ids[edge.end_node as usize];
                let t_start = (geom[1] - geom[0]).normalized();
                let t_end   = (geom[geom.len()-2] - geom[geom.len()-1]).normalized();
                tangent_at.insert((eid, sn), t_start);
                tangent_at.insert((eid, en), t_end);
            }

            let mut node_edges: HashMap<u32, Vec<usize>> = HashMap::new();
            for (eid, edge) in self.edges.iter().enumerate() {
                if edge.deleted || edge.primary_type != TransitType::Road { continue; }
                let sn = valid_node_ids[edge.start_node as usize];
                let en = valid_node_ids[edge.end_node as usize];
                node_edges.entry(sn).or_default().push(eid);
                if en != sn {
                    node_edges.entry(en).or_default().push(eid);
                }
            }

            let mut ramp_edges: std::collections::HashSet<usize> = std::collections::HashSet::new();
            let mut merge_nodes: std::collections::HashSet<u32> = std::collections::HashSet::new();

            for (&nid, eids) in &node_edges {
                if eids.len() < 3 { continue; }
                let n = eids.len();
                // Find the one anti-parallel pair that forms the mainline.
                // A Merge node must have exactly one such pair; if there are two or more
                // (e.g. a normal 4-way cross) we do not classify it as a Merge.
                let mut mainline_pair: Option<(usize, usize)> = None;
                let mut pair_count = 0;
                'find: for i in 0..n {
                    for j in (i+1)..n {
                        let ta = tangent_at.get(&(eids[i], nid));
                        let tb = tangent_at.get(&(eids[j], nid));
                        if let (Some(&ta), Some(&tb)) = (ta, tb) {
                            if ta.dot(tb) < -0.7 {
                                pair_count += 1;
                                if pair_count > 1 { mainline_pair = None; break 'find; }
                                mainline_pair = Some((i, j));
                            }
                        }
                    }
                }
                let (mi, mj) = match mainline_pair { Some(p) => p, None => continue };
                let ta = match tangent_at.get(&(eids[mi], nid)) { Some(&t) => t, None => continue };
                let tb = match tangent_at.get(&(eids[mj], nid)) { Some(&t) => t, None => continue };
                // All non-mainline edges must point in the general direction of one mainline leg.
                let mut all_valid = true;
                let mut found_ramp = false;
                for k in 0..n {
                    if k == mi || k == mj { continue; }
                    let tr = match tangent_at.get(&(eids[k], nid)) { Some(&t) => t, None => { all_valid = false; break; } };
                    // Ramp must share the general direction with at least one mainline leg (acute angle).
                    if tr.dot(ta) > 0.2 || tr.dot(tb) > 0.2 {
                        found_ramp = true;
                    } else {
                        all_valid = false;
                        break;
                    }
                }
                if all_valid && found_ramp {
                    for k in 0..n {
                        if k != mi && k != mj {
                            ramp_edges.insert(eids[k]);
                        }
                    }
                    merge_nodes.insert(nid);
                }
            }

            for nid in &merge_nodes {
                let n = &mut self.nodes[*nid as usize];
                if n.node_type == NodeType::Junction {
                    n.node_type = NodeType::Merge;
                }
            }
            for eid in &ramp_edges {
                let e = &mut self.edges[*eid];
                if e.class == EdgeClass::Standard {
                    e.class = EdgeClass::Ramp;
                }
            }

            // Second pass: zero out clips on mainline edges at Merge nodes.
            // The first clip pass above treated these nodes as Junction (Merge wasn't set yet).
            for (eid, edge) in self.edges.iter_mut().enumerate() {
                if edge.deleted || edge.class == EdgeClass::Ramp { continue; }
                let sn = valid_node_ids[edge.start_node as usize];
                let en = valid_node_ids[edge.end_node as usize];
                if merge_nodes.contains(&sn) {
                    edge.start_clip = 0.0;
                }
                if merge_nodes.contains(&en) {
                    edge.end_clip = 0.0;
                }
            }
        }

        // Re-index all roads after a massive batch clip rebuild (e.g. after terrain sync)
        self.spatial_edge_rt = rstar::RTree::new();
        for i in 0..self.edges.len() {
            self.add_to_spatial_index(i);
        }
        self.rebuild_adjacency_list();
    }
}
