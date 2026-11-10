//! Operator operations that act on a whole state directory: snapshot, restore
//! and credential rotation. Parity source of truth is the operator-operations
//! audit (`docs/parity/operator-cli-ops-2026-09-24.md:83-86`), which names
//! both gaps as MISSING.

pub mod backup;
pub mod rotate;
