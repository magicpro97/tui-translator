//! Local MT routing table and provider-selection helpers (LF-04, issue #372).
//!
//! This module contains the language-pair routing table, routing decision
//! types, and the resolve helper that maps a routing decision to a concrete
//! provider action.
//!
//! See [`routing`] for the full API.

pub mod router;
pub mod routing;
