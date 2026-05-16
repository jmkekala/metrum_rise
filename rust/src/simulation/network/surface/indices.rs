//! Small index helpers shared by surface compilation stages.

pub(super) fn normalized_vertex_edge(a: usize, b: usize) -> [usize; 2] {
    if a < b { [a, b] } else { [b, a] }
}
