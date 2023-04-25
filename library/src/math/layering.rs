use super::*;

/// Overlays some content over some piece of math.
///
/// ```example
/// #let custom-cancel(x) = {
///     $ overlay(#x, #line(stroke: blue + 0.5pt, start: (0pt, 100%), end: (100%, 0pt))) $
/// }
///
/// $ a + custom-cancel(xyz) - custom-cancel(xyz) $
/// ```
///
/// Display: Math Overlay
/// Category: math
#[element(LayoutMath)]
pub struct OverlayElem {
    /// The content over which content should be placed.
    #[positional]
    pub body: Content,

    /// The children to lay over the body, in order (rightmost is topmost).
    #[variadic]
    pub children: Vec<Content>,
}

impl LayoutMath for OverlayElem {
    fn layout_math(&self, ctx: &mut MathContext) -> SourceResult<()> {
        math_layer(ctx, self.body(ctx.styles()), self.children(), true)
    }
}

/// Layers some content below some piece of math.
///
/// ```example
/// #let cool-effect(x) = $ underlay(#x, #align(center + horizon, rect(width: 100% + 2em, height: 100%, fill: red))) $
///
/// $ a + cool-effect(b) - cool-effect(b) $
/// ```
///
/// Display: Math Overlay
/// Category: math
#[element(LayoutMath)]
pub struct UnderlayElem {
    /// The content under which content should be placed.
    #[positional]
    pub body: Content,

    /// The children to lay under the body, in order (leftmost is topmost).
    #[variadic]
    pub children: Vec<Content>,
}

impl LayoutMath for UnderlayElem {
    fn layout_math(&self, ctx: &mut MathContext) -> SourceResult<()> {
        math_layer(ctx, self.body(ctx.styles()), self.children(), false)
    }
}

/// Performs layering (places certain content above others) in math.
fn math_layer(
    ctx: &mut MathContext,
    body: Content,
    children: Vec<Content>,
    is_overlay: bool,
) -> SourceResult<()> {
    let mut body = ctx.layout_frame(&body)?;
    let size = body.size();

    // avoid conflicting with 'vt' borrow by creating a new StyleChain
    let styles = ctx.styles().to_map();
    let styles = StyleChain::new(&styles);

    let pod = Regions::one(size, Axes::splat(true));

    for child in children {
        let layer = child.layout(ctx.vt, styles, pod)?.into_frame();

        if is_overlay {
            body.push_frame(Point::zero(), layer);
        } else {
            body.prepend_frame(Point::zero(), layer);
        }
    }

    ctx.push(FrameFragment::new(ctx, body));

    Ok(())
}
