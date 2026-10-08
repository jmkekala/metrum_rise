// SPDX-License-Identifier: GPL-2.0-only

//! Deterministic directed components shared by route proofs and authored recipe ordering.

/// Labels strongly connected groups in topological order using iterative Kosaraju walks.
/// O(vertices + edges) time and scratch; adjacency and reverse adjacency must describe
/// the same directed graph. Disconnected vertices receive their own group.
pub(crate) fn strong_components(
    outgoing: &[Vec<usize>],
    incoming: &[Vec<usize>],
) -> (Vec<usize>, usize) {
    let mut visited = vec![false; outgoing.len()];
    let mut finished = Vec::with_capacity(outgoing.len());
    let mut walk = Vec::with_capacity(outgoing.len());
    for root in 0..outgoing.len() {
        if visited[root] {
            continue;
        }
        visited[root] = true;
        walk.push((root, 0));
        while let Some((node, next)) = walk.last_mut() {
            if let Some(&target) = outgoing[*node].get(*next) {
                *next += 1;
                if !visited[target] {
                    visited[target] = true;
                    walk.push((target, 0));
                }
            } else {
                finished.push(*node);
                walk.pop();
            }
        }
    }
    // Kosaraju's reverse walk yields strongly connected groups in topological order.
    let mut groups = vec![usize::MAX; outgoing.len()];
    let mut count = 0;
    let mut stack = Vec::with_capacity(outgoing.len());
    for &root in finished.iter().rev() {
        if groups[root] != usize::MAX {
            continue;
        }
        groups[root] = count;
        stack.push(root);
        while let Some(node) = stack.pop() {
            for &source in &incoming[node] {
                if groups[source] == usize::MAX {
                    groups[source] = count;
                    stack.push(source);
                }
            }
        }
        count += 1;
    }
    (groups, count)
}
