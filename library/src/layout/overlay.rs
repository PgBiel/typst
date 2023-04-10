use super::*;

/// # Overlay
/// Overlays some content over another. The first content specified is the base body,
/// and all following content are drawn over the first one, in a box with the same
/// width and height as that base content.
///
/// ```example
/// #overlay(rect(width: 100pt, height: 100pt, fill: yellow), align(center + horizon)[Hey!])
/// ```
///
/// Display: Overlay
/// Category: layout
#[element(Layout)]
pub struct OverlayElem {
    /// The body, upon which children will be overlaid.
    #[required]
    pub body: Content,

    /// The children to overlay over the body.
    #[variadic]
    pub children: Vec<Content>,
}

impl Layout for OverlayElem {
    fn layout(
        &self,
        vt: &mut Vt,
        styles: StyleChain,
        regions: Regions,
    ) -> SourceResult<Fragment> {
        // Render the body freely first, to get its size.
        let pod = Regions::one(regions.base(), Axes::splat(false));
        let mut frame = self.body().layout(vt, styles, pod)?.into_frame();
        let size = frame.size();

        // Now we restrict the children to that size, and
        // layout each child on top of the body, in order.
        let pod = Regions::one(size, Axes::splat(true));
        for child in self.children() {
            let child_frame = child.layout(vt, styles, pod)?.into_frame();
            frame.push_frame(Point::zero(), child_frame);
        }

        // Finally, apply metadata.
        frame.meta(styles, false);

        Ok(Fragment::frame(frame))
    }
}
