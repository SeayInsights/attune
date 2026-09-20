//! Measure, derive, apply, verify.
//!
//! The loop this crate closes:
//!
//! 1. [`attune_analysis`] records the mic and measures it.
//! 2. [`derive`] turns that measurement plus the current chain into settings,
//!    against a named [`targets::Target`].
//! 3. [`apply`] writes them through [`attune_control`], which verifies each one
//!    landed.
//! 4. Measure again to confirm, because the device only exposes the mic after
//!    its own DSP, so the effect of a change is observable but not predictable
//!    in closed form.
//!
//! Nothing here is specific to one microphone or one voice. Every number is
//! derived from a measured delta against a target, which is what lets someone
//! clone this and have it work on hardware the author has never seen.

pub mod apply;
pub mod derive;
pub mod targets;

pub use derive::{Change, Recommendation, derive};
pub use targets::Target;
