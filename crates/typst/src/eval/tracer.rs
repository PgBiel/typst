use comemo::Tracked;
use std::collections::HashSet;

use ecow::{eco_vec, EcoVec};
use typst_syntax::ast;

use crate::diag::{Severity, SourceDiagnostic, Tracepoint};
use crate::foundations::{Styles, Value};
use crate::syntax::{FileId, Span};
use crate::utils::hash128;
use crate::{World, WorldExt};

/// Traces warnings and which values existed for an expression at a span.
#[derive(Default, Clone)]
pub struct Tracer {
    inspected: Option<Span>,
    pending_warnings: Option<EcoVec<SourceDiagnostic>>,
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

    /// Pushes a warning to the list of pending warnings.
    /// This is done so tracepoints can be added to it later.
    /// However, if not currently inside a function call, the warning
    /// is directly added to the tracer, as no tracepoints will be added.
    pub fn warn_with_trace(&mut self, warning: SourceDiagnostic) {
        if let Some(pending_warnings) = &mut self.pending_warnings {
            pending_warnings.push(warning);
        } else {
            self.warn(warning);
        }
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

    /// Starts collecting pending warnings.
    /// This is done at the topmost function call, so that we can add
    /// tracepoints to upcoming warnings.
    pub fn init_pending_warnings(&mut self) -> bool {
        if self.pending_warnings.is_none() {
            self.pending_warnings = Some(eco_vec![]);
            true
        } else {
            false
        }
    }

    /// Add a tracepoint to all pending warnings.
    /// This is used when each function call is evaluated so we can generate a
    /// stack trace for the warning.
    pub fn trace_warnings(&mut self, world: Tracked<dyn World + '_>, span: Span) {
        let make_point = todo!();
        if let Some(pending_warnings) = &mut self.pending_warnings {
            let Some(trace_range) = world.range(span) else {
                return;
            };
            for warn in pending_warnings.make_mut() {
                warn.trace(&trace_range, world, &make_point, span);
            }
        }
    }

    /// Consume any pending warnings.
    /// Ensures the warnings will be recognized and displayed,
    /// as we won't be adding any further stack traces.
    pub fn flush_pending_warnings(&mut self) {
        if let Some(pending_warnings) = self.pending_warnings.take() {
            for warn in pending_warnings {
                self.warn(warn);
            }
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
