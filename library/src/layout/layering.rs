use super::*;

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
        let body = self.body(styles).unwrap_or_default();

        let children = self.children();

        layer_in_text(vt, styles, regions, body, children, /*is_overlay:*/ true)
    }
}

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
        let body = self.body(styles).unwrap_or_default();

        let children = self.children();

        layer_in_text(vt, styles, regions, body, children, false)
    }
}

/// Layer in non-math mode.
fn layer_in_text(
    vt: &mut Vt,
    styles: StyleChain,
    regions: Regions,
    body: Content,
    children: Vec<Content>,
    is_overlay: bool,
) -> SourceResult<Fragment> {
    // Render the body freely first, to get its size.
    let pod = Regions::one(regions.base(), Axes::splat(false));
    let mut frame = body.layout(vt, styles, pod)?.into_frame();
    let size = frame.size();

    // Now we restrict the children to that size, and
    // layout each child above or below the body, in order.
    let pod = Regions::one(size, Axes::splat(true));
    for child in children {
        let layer = child.layout(vt, styles, pod)?.into_frame();

        if is_overlay {
            frame.push_frame(Point::zero(), layer);
        } else {
            frame.prepend_frame(Point::zero(), layer);
        }
    }

    // Finally, apply metadata.
    frame.meta(styles, false);

    Ok(Fragment::frame(frame))
}
