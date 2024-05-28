use comemo::Tracked;

use crate::diag::{SourceDiagnostic, Tracepoint};
use crate::engine::Engine;
use crate::eval::FlowEvent;
use crate::foundations::{Context, IntoValue, Scopes, Value};
use crate::syntax::ast::{self, AstNode};
use crate::syntax::Span;
use crate::{World, WorldExt};

/// A virtual machine.
///
/// Holds the state needed to [evaluate](crate::eval::eval()) Typst sources. A
/// new virtual machine is created for each module evaluation and function call.
pub struct Vm<'a> {
    /// The underlying virtual typesetter.
    pub(crate) engine: Engine<'a>,
    /// A control flow event that is currently happening.
    pub(crate) flow: Option<FlowEvent>,
    /// The stack of scopes.
    pub(crate) scopes: Scopes<'a>,
    /// A span that is currently under inspection.
    pub(crate) inspected: Option<Span>,
    /// Warnings created by user code.
    /// They are stored in the Vm so that tracepoints can be added to them.
    /// Once we reach the top of the call stack, the warnings are pushed into
    /// the tracer.
    ///
    /// Note that this is `None` if no functions were called yet.
    /// When in a call stack, this will become `Some(...)`.
    /// After we leave the call stack, the warnings are dumped into the tracer
    /// and this field becomes `None` again.
    pub(crate) user_warns: Option<Vec<SourceDiagnostic>>,
    /// Data that is contextually made accessible to code behind the scenes.
    pub(crate) context: Tracked<'a, Context<'a>>,
}

impl<'a> Vm<'a> {
    /// Create a new virtual machine.
    pub fn new(
        engine: Engine<'a>,
        context: Tracked<'a, Context<'a>>,
        scopes: Scopes<'a>,
        target: Span,
    ) -> Self {
        let inspected = target.id().and_then(|id| engine.tracer.inspected(id));
        Self {
            engine,
            context,
            flow: None,
            scopes,
            inspected,
            user_warns: None,
        }
    }

    /// Access the underlying world.
    pub fn world(&self) -> Tracked<'a, dyn World + 'a> {
        self.engine.world
    }

    /// Define a variable in the current scope.
    pub fn define(&mut self, var: ast::Ident, value: impl IntoValue) {
        let value = value.into_value();
        if self.inspected == Some(var.span()) {
            self.trace(value.clone());
        }
        self.scopes.top.define(var.get().clone(), value);
    }

    /// Trace a value.
    #[cold]
    pub fn trace(&mut self, value: Value) {
        self.engine
            .tracer
            .value(value.clone(), self.context.styles().ok().map(|s| s.to_map()));
    }

    /// Add a tracepoint to all pending user warnings.
    /// This is used when each function call is evaluated so we can generate a
    /// stack trace for the warning.
    pub fn trace_warns<F>(&mut self, make_point: F, span: Span)
    where
        F: Fn() -> Tracepoint,
    {
        if let Some(user_warns) = &mut self.user_warns {
            let Some(trace_range) = self.engine.world.range(span) else {
                return;
            };
            for warn in user_warns {
                warn.trace(&trace_range, self.engine.world, &make_point, span);
            }
        }
    }

    /// Pushes a user warning to the list of pending user warnings.
    /// This is done so a tracepoint can be added to it later.
    /// However, if not currently inside a function call, the warning
    /// is directly added to the tracer, as no tracepoints will be added.
    pub fn push_user_warn(&mut self, warn: SourceDiagnostic) {
        if let Some(user_warns) = &mut self.user_warns {
            user_warns.push(warn);
        } else {
            self.engine.tracer.warn(warn);
        }
    }

    /// Consume and dump user warnings into the tracer.
    /// Ensures the warnings will be recognized and displayed,
    /// as we won't be adding any further stack traces.
    pub fn dump_warns(&mut self) {
        if let Some(user_warns) = self.user_warns.take() {
            for warn in user_warns {
                self.engine.tracer.warn(warn);
            }
        }
    }
}
