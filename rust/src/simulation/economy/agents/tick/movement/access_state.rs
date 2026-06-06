//! Access-egress and access-ingress movement state handling.

mod egress;
mod ingress;

pub(super) use egress::handle_access_egress;
pub(super) use ingress::handle_access_ingress;
