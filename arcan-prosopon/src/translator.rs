//! Pure translation layer: `aios_protocol::EventKind` → `Vec<ProsoponEvent>`.

use aios_protocol::EventKind;
use prosopon_core::ProsoponEvent;

use crate::state::TranslationState;

/// Translate a single `EventKind` into zero or more `ProsoponEvent`s.
///
/// Total over every currently-known variant and includes a `_` wildcard
/// for `#[non_exhaustive]` forward compatibility.
pub fn translate(_state: &mut TranslationState, kind: &EventKind) -> Vec<ProsoponEvent> {
    match kind {
        _ => Vec::new(),
    }
}
