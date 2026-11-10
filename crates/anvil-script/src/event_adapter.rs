// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Types the platform walker invokes. `anvil-dom::walk_event` owns capture,
//! target, and bubble. This module does not walk and does not import
//! `anvil-browser`.
//!
//! Map [`EventPhase`] onto `anvil_dom::WalkPhase` of the same three names.
//! [`DomEvent::type_name`] and [`DomEvent::current_target`] are the walk
//! step. [`ScriptError::Thrown`] continues the walk. [`ScriptError::BudgetExceeded`]
//! stops it and suppresses the default action.
//!
//! A script submit is [`anvil_dom::HostSubmission`], not a body tuple.
//! `requestSubmit` is [`anvil_dom::SubmitDispatch::Interactive`].
//! `submit` is [`anvil_dom::SubmitDispatch::Direct`]. Native activation is
//! the platform's. File bytes are [`anvil_dom::ChosenFormFile`], keyed by
//! document identity and control.

pub use crate::{DomEvent, EventPhase, ListenerId, ListenerSpec, ScriptError, ScriptRuntime};
pub use anvil_dom::{
    ChosenFormFile, FileSelectionRef, HostSubmission, ReadyState, SelectionLookup, SnapshotEnctype,
    SnapshotEntry, SnapshotError, SnapshotMethod, SubmissionSnapshot, SubmissionUrls,
    SubmitDispatch,
};
