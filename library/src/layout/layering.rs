use super::*;

/// # Overlay
/// Overlays some content over another. The first content specified is the base body,
/// and all following content are drawn, in order, over the first one (and all previous ones),
/// in a box with the same width and height as that base content.
///
/// ```example
/// #overlay(
///     rect(width: 100pt, height: 100pt, fill: yellow),
///     rotate(45deg, rect(width: 100pt, height: 100pt, fill: red)),
///     align(center + horizon)[Hey!]
/// )
/// ```
///
/// Display: Overlay
/// Category: layout
#[element(Layout)]
pub struct OverlayElem {
    /// The body, upon which children will be overlaid.
    #[positional]
    pub body: Option<Content>,

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
        let mut frame = self
            .body(styles)
            .unwrap_or_default()
            .layout(vt, styles, pod)?
            .into_frame();
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

/// # Underlay
/// Places some content visually below another. The first content specified is the base body,
/// and all following content are drawn, in order, under the first one (and all previous ones),
/// in a box with the same width and height as that base content.
///
/// ```example
/// #underlay(
///     rect(width: 100pt, height: 100pt, fill: yellow),
///     move(dx: 50%, dy: 50%, rect(width: 100%, height: 100%, fill: red)),
///     move(dx: 100%, dy: 100%, rect(width: 100%, height: 100%, fill: blue))
/// )
/// ```
///
/// Display: Underlay
/// Category: layout
#[element(Layout)]
pub struct UnderlayElem {
    /// The body, under which children will be laid out.
    #[positional]
    pub body: Option<Content>,

    /// The children to place under the body.
    #[variadic]
    pub children: Vec<Content>,
}

impl Layout for UnderlayElem {
    fn layout(
        &self,
        vt: &mut Vt,
        styles: StyleChain,
        regions: Regions,
    ) -> SourceResult<Fragment> {
        // Render the body freely first, to get its size.
        let pod = Regions::one(regions.base(), Axes::splat(false));
        let mut frame = self
            .body(styles)
            .unwrap_or_default()
            .layout(vt, styles, pod)?
            .into_frame();
        let size = frame.size();

        // Now we restrict the children to that size, and
        // layout each child below the body, in order.
        let pod = Regions::one(size, Axes::splat(true));
        for child in self.children() {
            let child_frame = child.layout(vt, styles, pod)?.into_frame();
            frame.prepend_frame(Point::zero(), child_frame);
        }

        // Finally, apply metadata.
        frame.meta(styles, false);

        Ok(Fragment::frame(frame))
    }
}
