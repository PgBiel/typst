use comemo::{Tracked, TrackedMut};
use std::collections::HashSet;

use ecow::EcoVec;
use typst_syntax::ast;

use crate::diag::{Severity, SourceDiagnostic, Tracepoint};
use crate::foundations::{Styles, Value};
use crate::syntax::{FileId, Span};
use crate::utils::hash128;
use crate::World;

/// Traces warnings and which values existed for an expression at a span.
#[derive(Default, Clone)]
pub struct Tracer {
    inspected: Option<Span>,
    warnings: EcoVec<SourceDiagnostic>,
    warnings_set: HashSet<u128>,
    delayed: EcoVec<SourceDiagnostic>,
    values: EcoVec<(Value, Option<Styles>)>,
}

impl Tracer {
    /// The maximum number of inspected values.
    pub const MAX_VALUES: usize = 10;

    /// Create a new tracer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Extend another tracer with this tracer's stored data.
    /// The destination tracer's `inspected` field is not changed.
    ///
    /// This is used whenever a temporary tracer is needed in order to be able
    /// to apply operations not tracked by comemo to the tracer. Then, this
    /// function is called to merge the two tracers together, thus comemo
    /// will only track (and replay, if needed) the `extend` operation,
    /// which should be more viable to cache.
    pub fn dump(self, mut destination: TrackedMut<Self>) {
        for warning in self.warnings {
            // Use 'warn()' to ensure we don't push duplicate warnings.
            // This should also update 'warnings_set' accordingly.
            destination.warn(warning);
        }

        destination.delay(self.delayed);

        for (value, styles) in self.values {
            destination.value(value, styles)
        }
    }

    /// Get the stored delayed errors.
    pub fn delayed(&mut self) -> EcoVec<SourceDiagnostic> {
        std::mem::take(&mut self.delayed)
    }

    /// Get the stored warnings.
    pub fn warnings(self) -> EcoVec<SourceDiagnostic> {
        self.warnings
    }

    /// Mark a span as inspected. All values observed for this span can be
    /// retrieved via `values` later.
    pub fn inspect(&mut self, span: Span) {
        self.inspected = Some(span);
    }

    /// Get the values for the inspected span.
    pub fn values(self) -> EcoVec<(Value, Option<Styles>)> {
        self.values
    }

    /// Add a tracepoint to all traced warnings that lie outside the given
    /// span.
    pub fn trace_warnings<F>(
        &mut self,
        world: Tracked<dyn World + '_>,
        make_point: F,
        span: Span,
    ) where
        F: Fn() -> Tracepoint,
    {
        crate::diag::trace_diagnostics(&mut self.warnings, world, make_point, span);
    }

    /// Remove any suppressed warnings.
    pub fn suppress_warns(&mut self, world: Tracked<dyn World + '_>) {
        suppress_warns(world, &mut self.warnings);
    }
}

#[comemo::track]
impl Tracer {
    /// Push delayed errors.
    pub fn delay(&mut self, errors: EcoVec<SourceDiagnostic>) {
        self.delayed.extend(errors);
    }

    /// Add a warning.
    pub fn warn(&mut self, warning: SourceDiagnostic) {
        // Check if warning is a duplicate.
        let hash = hash128(&(&warning.span, &warning.message));
        if self.warnings_set.insert(hash) {
            self.warnings.push(warning);
        }
    }

    /// The inspected span if it is part of the given source file.
    pub fn inspected(&self, id: FileId) -> Option<Span> {
        if self.inspected.and_then(Span::id) == Some(id) {
            self.inspected
        } else {
            None
        }
    }

    /// Trace a value for the span.
    pub fn value(&mut self, value: Value, styles: Option<Styles>) {
        if self.values.len() < Self::MAX_VALUES {
            self.values.push((value, styles));
        }
    }
}

fn suppress_warns(world: Tracked<dyn World + '_>, diags: &mut EcoVec<SourceDiagnostic>) {
    diags.retain(|diag| {
        // Only retain warnings which weren't locally suppressed where they
        // were emitted or at any of their tracepoints.
        diag.severity != Severity::Warning
            || (!check_warning_suppressed(diag.span, world, &diag.message)
                && !diag.trace.iter().any(|tracepoint| {
                    check_warning_suppressed(tracepoint.span, world, &diag.message)
                }))
    });
}

/// Checks if a given warning is suppressed given one span it has a tracepoint
/// in. If there is one parent `allow("warning")` node containing this span
/// in the same file, the warning is considered suppressed.
fn check_warning_suppressed(
    span: Span,
    world: Tracked<dyn World + '_>,
    message: &ecow::EcoString,
) -> bool {
    let Some(file) = span.id() else {
        // Don't suppress detached warnings.
        return false;
    };

    // The source must exist if a warning occurred in the file,
    // or has a tracepoint in the file.
    let source = world.source(file).unwrap();
    // The span must point to this source file, so we unwrap.
    let mut node = &source.find(span).unwrap();

    // Walk the parent nodes to check for a warning suppression.
    while let Some(parent) = node.parent() {
        if let Some(allow_warning) = parent.cast::<ast::AllowWarning>() {
            if allow_warning.warning() == *message {
                // Suppress this warning.
                return true;
            }
        }
        node = parent;
    }

    false
}
