use super::*;

/// Overlays some content over some piece of math.
///
/// ```example
/// #let cancel(x) = $ overlay(#x, #line(stroke: 0.5pt, start: (0pt, 100%), end: (100%, 0pt))) $
///
/// $ a + cancel(b) - cancel(b) $
/// ```
///
/// Display: Math Overlay
/// Category: math
#[element(LayoutMath)]
pub struct OverlayElem {
    /// The content over which content should be placed.
    #[required]
    pub body: Content,

    /// The children to overlay over the body.
    #[variadic]
    pub children: Vec<Content>,
}

impl LayoutMath for OverlayElem {
    fn layout_math(&self, ctx: &mut MathContext) -> SourceResult<()> {
        layer_in_math(ctx, self.body(), self.children(), /*is_overlay:*/ true)
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
    #[required]
    pub body: Content,

    /// The children to display under the body.
    #[variadic]
    pub children: Vec<Content>,
}

impl LayoutMath for UnderlayElem {
    fn layout_math(&self, ctx: &mut MathContext) -> SourceResult<()> {
        layer_in_math(ctx, self.body(), self.children(), /*is_overlay:*/ false)
    }
}

/// Performs layering (places certain content above others) in math.
fn layer_in_math(
    ctx: &mut MathContext,
    body: Content,
    children: Vec<Content>,
    is_overlay: bool,
) -> SourceResult<()> {
    let mut body = ctx.layout_frame(&body)?;

    let size = body.size();
    let styles = ctx.styles().to_map();
    let styles = StyleChain::new(&styles);

    let pod = Regions::one(size, Axes::splat(true));

    for child in children {
        let layer = child.layout(&mut ctx.vt, styles, pod)?.into_frame();

        if is_overlay {
            body.push_frame(Point::zero(), layer);
        } else {
            body.prepend_frame(Point::zero(), layer);
        }
    }

    ctx.push(FrameFragment::new(ctx, body));

    Ok(())
}
