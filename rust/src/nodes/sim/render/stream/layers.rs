// SPDX-License-Identifier: GPL-2.0-only

//! The streaming core's layers: per renderer, the state that decides what it draws. The graded
//! tiles ([`graded`]) are the first.

pub(crate) mod graded;
