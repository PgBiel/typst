use super::*;

/// # Math Overlay
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
pub struct MathOverlayElem {
    /// The content over which content should be placed.
    #[required]
    pub body: Content,

    /// The content to place.
    #[required]
    pub overlay: Content,
}

impl LayoutMath for MathOverlayElem {
    fn layout_math(&self, ctx: &mut MathContext) -> SourceResult<()> {
        let body = ctx.layout_frame(&self.body())?;

        let size = body.size();

        let overlay = self.overlay();
        let pod = Regions::one(size, Axes::splat(true));
        let styles = ctx.styles().to_map();
        let overlay = overlay
            .layout(&mut ctx.vt, StyleChain::new(&styles), pod)?
            .into_frame();

        let mut frame = Frame::new(size);
        frame.set_baseline(body.baseline());
        frame.push_frame(Point::zero(), body);
        frame.push_frame(Point::zero(), overlay);

        ctx.push(FrameFragment::new(ctx, frame));

        Ok(())
    }
}
