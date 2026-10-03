// SPDX-License-Identifier: GPL-2.0-only

//! Lane successor lists that keep the common single successor inline.

use std::ops::Deref;

/// Lanes reachable from the end of a lane, in connection order.
///
/// Every junction connector has exactly one successor, so a single id is stored inline and only
/// longer lists allocate, at their exact length. Reads go through the `[usize]` slice.
#[derive(Clone)]
pub struct LaneSuccessors(Storage);

#[derive(Clone)]
enum Storage {
    One(usize),
    // Empty or at least two ids; an empty boxed slice does not allocate.
    Many(Box<[usize]>),
}

impl LaneSuccessors {
    /// A list holding only `lane_id`, as every junction connector has.
    pub fn one(lane_id: usize) -> Self {
        Self(Storage::One(lane_id))
    }

    /// Appends `lane_id`. Longer lists are copied into a new exact-length allocation; they are
    /// only built during lane rebuilds, a few ids per junction arm.
    pub fn push(&mut self, lane_id: usize) {
        self.0 = match std::mem::take(self).0 {
            Storage::Many(ids) if ids.is_empty() => Storage::One(lane_id),
            Storage::One(first) => Storage::Many(Box::new([first, lane_id])),
            Storage::Many(ids) => {
                let mut grown = Vec::with_capacity(ids.len() + 1);
                grown.extend_from_slice(&ids);
                grown.push(lane_id);
                Storage::Many(grown.into_boxed_slice())
            }
        };
    }

    /// Removes every successor and frees the list.
    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

impl Default for LaneSuccessors {
    fn default() -> Self {
        Self(Storage::Many(Box::default()))
    }
}

impl Deref for LaneSuccessors {
    type Target = [usize];

    fn deref(&self) -> &[usize] {
        match &self.0 {
            Storage::One(id) => std::slice::from_ref(id),
            Storage::Many(ids) => ids,
        }
    }
}

impl<'a> IntoIterator for &'a LaneSuccessors {
    type Item = &'a usize;
    type IntoIter = std::slice::Iter<'a, usize>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl From<Vec<usize>> for LaneSuccessors {
    fn from(ids: Vec<usize>) -> Self {
        match ids.as_slice() {
            &[id] => Self(Storage::One(id)),
            _ => Self(Storage::Many(ids.into_boxed_slice())),
        }
    }
}

impl std::fmt::Debug for LaneSuccessors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl PartialEq for LaneSuccessors {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}

impl Eq for LaneSuccessors {}

impl PartialEq<Vec<usize>> for LaneSuccessors {
    fn eq(&self, other: &Vec<usize>) -> bool {
        **self == **other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn successors_keep_one_id_inline_and_longer_lists_exact() {
        assert_eq!(size_of::<LaneSuccessors>(), 16);

        let mut successors = LaneSuccessors::default();
        assert!(successors.is_empty());
        successors.push(7);
        assert!(matches!(successors.0, Storage::One(7)));
        successors.push(3);
        successors.push(9);
        assert_eq!(successors, vec![7, 3, 9]);
        assert!(matches!(&successors.0, Storage::Many(ids) if ids.len() == 3));
        assert_eq!(format!("{successors:?}"), format!("{:?}", vec![7, 3, 9]));

        successors.clear();
        assert!(successors.is_empty());
        successors.push(4);
        assert!(matches!(successors.0, Storage::One(4)));
        assert_eq!(LaneSuccessors::from(vec![4]), successors);
        assert!(matches!(LaneSuccessors::from(vec![4]).0, Storage::One(4)));
        assert!(LaneSuccessors::from(Vec::new()).is_empty());
    }
}
