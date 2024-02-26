use std::fmt::Debug;
use std::hash::Hash;
use std::num::NonZeroUsize;
use std::sync::Arc;

use ecow::eco_format;

use super::lines::{
    generate_line_segments, hline_stroke_at_column, vline_stroke_at_row, Line,
    LinePosition, LineSegment,
};
use super::rowspans::{subtract_end_sizes, Rowspan, UnbreakableRowGroup};
use crate::diag::{
    bail, At, Hint, HintedStrResult, HintedString, SourceResult, StrResult,
};
use crate::engine::Engine;
use crate::foundations::{
    Array, CastInfo, Content, Context, Fold, FromValue, Func, IntoValue, Reflect,
    Resolve, Smart, StyleChain, Value,
};
use crate::layout::{
    Abs, Alignment, Axes, Dir, Fr, Fragment, Frame, FrameItem, LayoutMultiple, Length,
    Point, Regions, Rel, Sides, Size, Sizing,
};
use crate::syntax::Span;
use crate::text::TextElem;
use crate::util::{MaybeReverseIter, NonZeroExt, Numeric};
use crate::visualize::{Geometry, Paint, Stroke};

/// A value that can be configured per cell.
#[derive(Debug, Clone, PartialEq, Hash)]
pub enum Celled<T> {
    /// A bare value, the same for all cells.
    Value(T),
    /// A closure mapping from cell coordinates to a value.
    Func(Func),
    /// An array of alignment values corresponding to each column.
    Array(Vec<T>),
}

impl<T: Default + Clone + FromValue> Celled<T> {
    /// Resolve the value based on the cell position.
    pub fn resolve(
        &self,
        engine: &mut Engine,
        styles: StyleChain,
        x: usize,
        y: usize,
    ) -> SourceResult<T> {
        Ok(match self {
            Self::Value(value) => value.clone(),
            Self::Func(func) => func
                .call(engine, &Context::new(None, Some(styles)), [x, y])?
                .cast()
                .at(func.span())?,
            Self::Array(array) => x
                .checked_rem(array.len())
                .and_then(|i| array.get(i))
                .cloned()
                .unwrap_or_default(),
        })
    }
}

impl<T: Default> Default for Celled<T> {
    fn default() -> Self {
        Self::Value(T::default())
    }
}

impl<T: Reflect> Reflect for Celled<T> {
    fn input() -> CastInfo {
        T::input() + Array::input() + Func::input()
    }

    fn output() -> CastInfo {
        T::output() + Array::output() + Func::output()
    }

    fn castable(value: &Value) -> bool {
        Array::castable(value) || Func::castable(value) || T::castable(value)
    }
}

impl<T: IntoValue> IntoValue for Celled<T> {
    fn into_value(self) -> Value {
        match self {
            Self::Value(value) => value.into_value(),
            Self::Func(func) => func.into_value(),
            Self::Array(arr) => arr.into_value(),
        }
    }
}

impl<T: FromValue> FromValue for Celled<T> {
    fn from_value(value: Value) -> StrResult<Self> {
        match value {
            Value::Func(v) => Ok(Self::Func(v)),
            Value::Array(array) => Ok(Self::Array(
                array.into_iter().map(T::from_value).collect::<StrResult<_>>()?,
            )),
            v if T::castable(&v) => Ok(Self::Value(T::from_value(v)?)),
            v => Err(Self::error(&v)),
        }
    }
}

impl<T: Fold> Fold for Celled<T> {
    fn fold(self, outer: Self) -> Self {
        match (self, outer) {
            (Self::Value(inner), Self::Value(outer)) => Self::Value(inner.fold(outer)),
            (self_, _) => self_,
        }
    }
}

impl<T: Resolve> Resolve for Celled<T> {
    type Output = ResolvedCelled<T>;

    fn resolve(self, styles: StyleChain) -> Self::Output {
        match self {
            Self::Value(value) => ResolvedCelled(Celled::Value(value.resolve(styles))),
            Self::Func(func) => ResolvedCelled(Celled::Func(func)),
            Self::Array(values) => ResolvedCelled(Celled::Array(
                values.into_iter().map(|value| value.resolve(styles)).collect(),
            )),
        }
    }
}

/// The result of resolving a Celled's value according to styles.
/// Holds resolved values which depend on each grid cell's position.
/// When it is a closure, however, it is only resolved when the closure is
/// called.
#[derive(Default, Clone)]
pub struct ResolvedCelled<T: Resolve>(Celled<T::Output>);

impl<T> ResolvedCelled<T>
where
    T: FromValue + Resolve,
    <T as Resolve>::Output: Default + Clone,
{
    /// Resolve the value based on the cell position.
    pub fn resolve(
        &self,
        engine: &mut Engine,
        styles: StyleChain,
        x: usize,
        y: usize,
    ) -> SourceResult<T::Output> {
        Ok(match &self.0 {
            Celled::Value(value) => value.clone(),
            Celled::Func(func) => func
                .call(engine, &Context::new(None, Some(styles)), [x, y])?
                .cast::<T>()
                .at(func.span())?
                .resolve(styles),
            Celled::Array(array) => x
                .checked_rem(array.len())
                .and_then(|i| array.get(i))
                .cloned()
                .unwrap_or_default(),
        })
    }
}

/// Represents a cell in CellGrid, to be laid out by GridLayouter.
#[derive(Clone)]
pub struct Cell {
    /// The cell's body.
    pub body: Content,
    /// The cell's fill.
    pub fill: Option<Paint>,
    /// The amount of columns spanned by the cell.
    pub colspan: NonZeroUsize,
    /// The amount of rows spanned by the cell.
    pub rowspan: NonZeroUsize,
    /// The cell's stroke.
    ///
    /// We use an Arc to avoid unnecessary space usage when all sides are the
    /// same, or when the strokes come from a common source.
    pub stroke: Sides<Option<Arc<Stroke<Abs>>>>,
    /// Which stroke sides were explicitly overridden by the cell, over the
    /// grid's global stroke setting.
    ///
    /// This is used to define whether or not this cell's stroke sides should
    /// have priority over adjacent cells' stroke sides, if those don't
    /// override their own stroke properties (and thus have less priority when
    /// defining with which stroke to draw grid lines around this cell).
    pub stroke_overridden: Sides<bool>,
    /// Whether rows spanned by this cell can be placed in different pages.
    /// When equal to [`Smart::Auto`], a cell spanning only fixed-size rows is
    /// unbreakable, while a cell spanning at least one `auto`-sized row is
    /// breakable.
    pub breakable: Smart<bool>,
}

impl From<Content> for Cell {
    /// Create a simple cell given its body.
    fn from(body: Content) -> Self {
        Self {
            body,
            fill: None,
            colspan: NonZeroUsize::ONE,
            rowspan: NonZeroUsize::ONE,
            stroke: Sides::splat(None),
            stroke_overridden: Sides::splat(false),
            breakable: Smart::Auto,
        }
    }
}

impl LayoutMultiple for Cell {
    fn layout(
        &self,
        engine: &mut Engine,
        styles: StyleChain,
        regions: Regions,
    ) -> SourceResult<Fragment> {
        self.body.layout(engine, styles, regions)
    }
}

/// A grid entry.
#[derive(Clone)]
pub(super) enum Entry {
    /// An entry which holds a cell.
    Cell(Cell),
    /// An entry which is merged with another cell.
    Merged {
        /// The index of the cell this entry is merged with.
        parent: usize,
    },
}

impl Entry {
    /// Obtains the cell inside this entry, if this is not a merged cell.
    fn as_cell(&self) -> Option<&Cell> {
        match self {
            Self::Cell(cell) => Some(cell),
            Self::Merged { .. } => None,
        }
    }
}

/// A grid item, possibly affected by automatic cell positioning. Can be either
/// a line or a cell.
pub enum GridItem<T: ResolvableCell> {
    /// A horizontal line in the grid.
    HLine {
        /// The row above which the horizontal line is drawn.
        y: Smart<usize>,
        start: usize,
        end: Option<NonZeroUsize>,
        stroke: Option<Arc<Stroke<Abs>>>,
        /// The span of the corresponding line element.
        span: Span,
        /// The line's position. "before" here means on top of row 'y', while
        /// "after" means below it.
        position: LinePosition,
    },
    /// A vertical line in the grid.
    VLine {
        /// The column before which the vertical line is drawn.
        x: Smart<usize>,
        start: usize,
        end: Option<NonZeroUsize>,
        stroke: Option<Arc<Stroke<Abs>>>,
        /// The span of the corresponding line element.
        span: Span,
        /// The line's position. "before" here means to the left of column 'x',
        /// while "after" means to its right (both considering LTR).
        position: LinePosition,
    },
    /// A cell in the grid.
    Cell(T),
}

/// Used for cell-like elements which are aware of their final properties in
/// the table, and may have property overrides.
pub trait ResolvableCell {
    /// Resolves the cell's fields, given its coordinates and default grid-wide
    /// fill, align, inset and stroke properties.
    /// Returns a final Cell.
    #[allow(clippy::too_many_arguments)]
    fn resolve_cell(
        self,
        x: usize,
        y: usize,
        fill: &Option<Paint>,
        align: Smart<Alignment>,
        inset: Sides<Option<Rel<Length>>>,
        stroke: Sides<Option<Option<Arc<Stroke<Abs>>>>>,
        styles: StyleChain,
    ) -> Cell;

    /// Returns this cell's column override.
    fn x(&self, styles: StyleChain) -> Smart<usize>;

    /// Returns this cell's row override.
    fn y(&self, styles: StyleChain) -> Smart<usize>;

    /// The amount of columns spanned by this cell.
    fn colspan(&self, styles: StyleChain) -> NonZeroUsize;

    /// The amount of rows spanned by this cell.
    fn rowspan(&self, styles: StyleChain) -> NonZeroUsize;

    /// The cell's span, for errors.
    fn span(&self) -> Span;
}

/// A grid of cells, including the columns, rows, and cell data.
pub struct CellGrid {
    /// The grid cells.
    pub(super) entries: Vec<Entry>,
    /// The column tracks including gutter tracks.
    pub(super) cols: Vec<Sizing>,
    /// The row tracks including gutter tracks.
    pub(super) rows: Vec<Sizing>,
    /// The vertical lines before each column, or on the end border.
    /// Gutter columns are not included.
    /// Contains up to 'cols_without_gutter.len() + 1' vectors of lines.
    pub(super) vlines: Vec<Vec<Line>>,
    /// The horizontal lines on top of each row, or on the bottom border.
    /// Gutter rows are not included.
    /// Contains up to 'rows_without_gutter.len() + 1' vectors of lines.
    pub(super) hlines: Vec<Vec<Line>>,
    /// Whether this grid has gutters.
    pub(super) has_gutter: bool,
}

impl CellGrid {
    /// Generates the cell grid, given the tracks and cells.
    pub fn new(
        tracks: Axes<&[Sizing]>,
        gutter: Axes<&[Sizing]>,
        cells: impl IntoIterator<Item = Cell>,
    ) -> Self {
        let entries = cells.into_iter().map(Entry::Cell).collect();
        Self::new_internal(tracks, gutter, vec![], vec![], entries)
    }

    /// Resolves and positions all cells in the grid before creating it.
    /// Allows them to keep track of their final properties and positions
    /// and adjust their fields accordingly.
    /// Cells must implement Clone as they will be owned. Additionally, they
    /// must implement Default in order to fill positions in the grid which
    /// weren't explicitly specified by the user with empty cells.
    #[allow(clippy::too_many_arguments)]
    pub fn resolve<T, I>(
        tracks: Axes<&[Sizing]>,
        gutter: Axes<&[Sizing]>,
        items: I,
        fill: &Celled<Option<Paint>>,
        align: &Celled<Smart<Alignment>>,
        inset: &Celled<Sides<Option<Rel<Length>>>>,
        stroke: &ResolvedCelled<Sides<Option<Option<Arc<Stroke>>>>>,
        engine: &mut Engine,
        styles: StyleChain,
        span: Span,
    ) -> SourceResult<Self>
    where
        T: ResolvableCell + Default,
        I: IntoIterator<Item = GridItem<T>>,
        I::IntoIter: ExactSizeIterator,
    {
        // Number of content columns: Always at least one.
        let c = tracks.x.len().max(1);

        // Lists of lines.
        // Horizontal lines are only pushed later to be able to check for row
        // validity, since the amount of rows isn't known until all items were
        // analyzed in the for loop below.
        // We keep their spans so we can report errors later.
        let mut pending_hlines: Vec<(Span, Line)> = vec![];

        // For consistency, only push vertical lines later as well.
        let mut pending_vlines: Vec<(Span, Line)> = vec![];
        let has_gutter = gutter.any(|tracks| !tracks.is_empty());

        // We can't just use the cell's index in the 'cells' vector to
        // determine its automatic position, since cells could have arbitrary
        // positions, so the position of a cell in 'cells' can differ from its
        // final position in 'resolved_cells' (see below).
        // Therefore, we use a counter, 'auto_index', to determine the position
        // of the next cell with (x: auto, y: auto). It is only stepped when
        // a cell with (x: auto, y: auto), usually the vast majority, is found.
        let mut auto_index: usize = 0;

        // We have to rebuild the grid to account for arbitrary positions.
        // Create at least 'items.len()' positions, since there could be at
        // least 'items.len()' cells (if no explicit lines were specified),
        // even though some of them might be placed in arbitrary positions and
        // thus cause the grid to expand.
        // Additionally, make sure we allocate up to the next multiple of 'c',
        // since each row will have 'c' cells, even if the last few cells
        // weren't explicitly specified by the user.
        // We apply '% c' twice so that the amount of cells potentially missing
        // is zero when 'items.len()' is already a multiple of 'c' (thus
        // 'items.len() % c' would be zero).
        let items = items.into_iter();
        let Some(item_count) = items.len().checked_add((c - items.len() % c) % c) else {
            bail!(span, "too many cells or lines were given")
        };
        let mut resolved_cells: Vec<Option<Entry>> = Vec::with_capacity(item_count);
        for item in items {
            let cell = match item {
                GridItem::HLine { y, start, end, stroke, span, position } => {
                    let y = y.unwrap_or_else(|| {
                        // When no 'y' is specified for the hline, we place it
                        // under the latest automatically positioned cell.
                        // The current value of the auto index is always the
                        // index of the latest automatically positioned cell
                        // placed plus one (that's what we do in
                        // 'resolve_cell_position'), so we subtract 1 to get
                        // that cell's index, and place the hline below its
                        // row. The exception is when the auto_index is 0,
                        // meaning no automatically positioned cell was placed
                        // yet. In that case, we place the hline at the top of
                        // the table.
                        auto_index
                            .checked_sub(1)
                            .map_or(0, |last_auto_index| last_auto_index / c + 1)
                    });
                    if end.is_some_and(|end| end.get() < start) {
                        bail!(span, "line cannot end before it starts");
                    }
                    let line = Line { index: y, start, end, stroke, position };

                    // Since the amount of rows is dynamic, delay placing
                    // hlines until after all cells were placed so we can
                    // properly verify if they are valid. Note that we can't
                    // place hlines even if we already know they would be in a
                    // valid row, since it's possible that we pushed pending
                    // hlines in the same row as this one in previous
                    // iterations, and we need to ensure that hlines from
                    // previous iterations are pushed to the final vector of
                    // hlines first - the order of hlines must be kept, as this
                    // matters when determining which one "wins" in case of
                    // conflict. Pushing the current hline before we push
                    // pending hlines later would change their order!
                    pending_hlines.push((span, line));
                    continue;
                }
                GridItem::VLine { x, start, end, stroke, span, position } => {
                    let x = x.unwrap_or_else(|| {
                        // When no 'x' is specified for the vline, we place it
                        // after the latest automatically positioned cell.
                        // The current value of the auto index is always the
                        // index of the latest automatically positioned cell
                        // placed plus one (that's what we do in
                        // 'resolve_cell_position'), so we subtract 1 to get
                        // that cell's index, and place the vline after its
                        // column. The exception is when the auto_index is 0,
                        // meaning no automatically positioned cell was placed
                        // yet. In that case, we place the vline to the left of
                        // the table.
                        auto_index
                            .checked_sub(1)
                            .map_or(0, |last_auto_index| last_auto_index % c + 1)
                    });
                    if end.is_some_and(|end| end.get() < start) {
                        bail!(span, "line cannot end before it starts");
                    }
                    let line = Line { index: x, start, end, stroke, position };

                    // For consistency with hlines, we only push vlines to the
                    // final vector of vlines after processing every cell.
                    pending_vlines.push((span, line));
                    continue;
                }
                GridItem::Cell(cell) => cell,
            };
            let cell_span = cell.span();
            // Let's calculate the cell's final position based on its
            // requested position.
            let resolved_index = {
                let cell_x = cell.x(styles);
                let cell_y = cell.y(styles);
                resolve_cell_position(cell_x, cell_y, &resolved_cells, &mut auto_index, c)
                    .at(cell_span)?
            };
            let x = resolved_index % c;
            let y = resolved_index / c;
            let colspan = cell.colspan(styles).get();
            let rowspan = cell.rowspan(styles).get();

            if colspan > c - x {
                bail!(
                    cell_span,
                    "cell's colspan would cause it to exceed the available column(s)";
                    hint: "try placing the cell in another position or reducing its colspan"
                )
            }

            let Some(largest_index) = c
                .checked_mul(rowspan - 1)
                .and_then(|full_rowspan_offset| {
                    resolved_index.checked_add(full_rowspan_offset)
                })
                .and_then(|last_row_pos| last_row_pos.checked_add(colspan - 1))
            else {
                bail!(
                    cell_span,
                    "cell would span an exceedingly large position";
                    hint: "try reducing the cell's rowspan or colspan"
                )
            };

            // Let's resolve the cell so it can determine its own fields
            // based on its final position.
            let cell = cell.resolve_cell(
                x,
                y,
                &fill.resolve(engine, styles, x, y)?,
                align.resolve(engine, styles, x, y)?,
                inset.resolve(engine, styles, x, y)?,
                stroke.resolve(engine, styles, x, y)?,
                styles,
            );

            if largest_index >= resolved_cells.len() {
                // Ensure the length of the vector of resolved cells is always
                // a multiple of 'c' by pushing full rows every time. Here, we
                // add enough absent positions (later converted to empty cells)
                // to ensure the last row in the new vector length is
                // completely filled. This is necessary so that those
                // positions, even if not explicitly used at the end, are
                // eventually susceptible to show rules and receive grid
                // styling, as they will be resolved as empty cells in a second
                // loop below.
                let Some(new_len) = largest_index
                    .checked_add(1)
                    .and_then(|new_len| new_len.checked_add((c - new_len % c) % c))
                else {
                    bail!(cell_span, "cell position too large")
                };

                // Here, the cell needs to be placed in a position which
                // doesn't exist yet in the grid (out of bounds). We will add
                // enough absent positions for this to be possible. They must
                // be absent as no cells actually occupy them (they can be
                // overridden later); however, if no cells occupy them as we
                // finish building the grid, then such positions will be
                // replaced by empty cells.
                resolved_cells.resize(new_len, None);
            }

            // The vector is large enough to contain the cell, so we can just
            // index it directly to access the position it will be placed in.
            // However, we still need to ensure we won't try to place a cell
            // where there already is one.
            let slot = &mut resolved_cells[resolved_index];
            if slot.is_some() {
                bail!(
                    cell_span,
                    "attempted to place a second cell at column {x}, row {y}";
                    hint: "try specifying your cells in a different order"
                );
            }

            *slot = Some(Entry::Cell(cell));

            // Now, if the cell spans more than one row or column, we fill the
            // spanned positions in the grid with Entry::Merged pointing to the
            // original cell as its parent.
            for rowspan_offset in 0..rowspan {
                let spanned_y = y + rowspan_offset;
                let first_row_index = resolved_index + c * rowspan_offset;
                for (colspan_offset, slot) in
                    resolved_cells[first_row_index..][..colspan].iter_mut().enumerate()
                {
                    let spanned_x = x + colspan_offset;
                    if spanned_x == x && spanned_y == y {
                        // This is the parent cell.
                        continue;
                    }
                    if slot.is_some() {
                        bail!(
                            cell_span,
                            "cell would span a previously placed cell at column {spanned_x}, row {spanned_y}";
                            hint: "try specifying your cells in a different order or reducing the cell's rowspan or colspan"
                        )
                    }
                    *slot = Some(Entry::Merged { parent: resolved_index });
                }
            }
        }

        // Replace absent entries by resolved empty cells, and produce a vector
        // of 'Entry' from 'Option<Entry>' (final step).
        let resolved_cells = resolved_cells
            .into_iter()
            .enumerate()
            .map(|(i, cell)| {
                if let Some(cell) = cell {
                    Ok(cell)
                } else {
                    let x = i % c;
                    let y = i / c;

                    // Ensure all absent entries are affected by show rules and
                    // grid styling by turning them into resolved empty cells.
                    let new_cell = T::default().resolve_cell(
                        x,
                        y,
                        &fill.resolve(engine, styles, x, y)?,
                        align.resolve(engine, styles, x, y)?,
                        inset.resolve(engine, styles, x, y)?,
                        stroke.resolve(engine, styles, x, y)?,
                        styles,
                    );
                    Ok(Entry::Cell(new_cell))
                }
            })
            .collect::<SourceResult<Vec<Entry>>>()?;

        // Populate the final lists of lines.
        // For each line type (horizontal or vertical), we keep a vector for
        // every group of lines with the same index.
        let mut vlines: Vec<Vec<Line>> = vec![];
        let mut hlines: Vec<Vec<Line>> = vec![];
        let row_amount = resolved_cells.len().div_ceil(c);

        for (line_span, line) in pending_hlines {
            let y = line.index;
            if y > row_amount {
                bail!(line_span, "cannot place horizontal line at invalid row {y}");
            }
            if y == row_amount && line.position == LinePosition::After {
                bail!(
                    line_span,
                    "cannot place horizontal line at the 'bottom' position of the bottom border (y = {y})";
                    hint: "set the line's position to 'top' or place it at a smaller 'y' index"
                );
            }
            let line = if line.position == LinePosition::After
                && (!has_gutter || y + 1 == row_amount)
            {
                // Just place the line on top of the next row if
                // there's no gutter and the line should be placed
                // after the one with given index.
                //
                // Note that placing after the last row is also the same as
                // just placing on the grid's bottom border, even with
                // gutter.
                Line {
                    index: y + 1,
                    position: LinePosition::Before,
                    ..line
                }
            } else {
                line
            };
            let y = line.index;

            if hlines.len() <= y {
                hlines.resize_with(y + 1, Vec::new);
            }
            hlines[y].push(line);
        }

        for (line_span, line) in pending_vlines {
            let x = line.index;
            if x > c {
                bail!(line_span, "cannot place vertical line at invalid column {x}");
            }
            if x == c && line.position == LinePosition::After {
                bail!(
                    line_span,
                    "cannot place vertical line at the 'end' position of the end border (x = {c})";
                    hint: "set the line's position to 'start' or place it at a smaller 'x' index"
                );
            }
            let line =
                if line.position == LinePosition::After && (!has_gutter || x + 1 == c) {
                    // Just place the line before the next column if
                    // there's no gutter and the line should be placed
                    // after the one with given index.
                    //
                    // Note that placing after the last column is also the
                    // same as just placing on the grid's end border, even
                    // with gutter.
                    Line {
                        index: x + 1,
                        position: LinePosition::Before,
                        ..line
                    }
                } else {
                    line
                };
            let x = line.index;

            if vlines.len() <= x {
                vlines.resize_with(x + 1, Vec::new);
            }
            vlines[x].push(line);
        }

        Ok(Self::new_internal(tracks, gutter, vlines, hlines, resolved_cells))
    }

    /// Generates the cell grid, given the tracks and resolved entries.
    pub(super) fn new_internal(
        tracks: Axes<&[Sizing]>,
        gutter: Axes<&[Sizing]>,
        vlines: Vec<Vec<Line>>,
        hlines: Vec<Vec<Line>>,
        entries: Vec<Entry>,
    ) -> Self {
        let mut cols = vec![];
        let mut rows = vec![];

        // Number of content columns: Always at least one.
        let c = tracks.x.len().max(1);

        // Number of content rows: At least as many as given, but also at least
        // as many as needed to place each item.
        let r = {
            let len = entries.len();
            let given = tracks.y.len();
            let needed = len / c + (len % c).clamp(0, 1);
            given.max(needed)
        };

        let has_gutter = gutter.any(|tracks| !tracks.is_empty());
        let auto = Sizing::Auto;
        let zero = Sizing::Rel(Rel::zero());
        let get_or = |tracks: &[_], idx, default| {
            tracks.get(idx).or(tracks.last()).copied().unwrap_or(default)
        };

        // Collect content and gutter columns.
        for x in 0..c {
            cols.push(get_or(tracks.x, x, auto));
            if has_gutter {
                cols.push(get_or(gutter.x, x, zero));
            }
        }

        // Collect content and gutter rows.
        for y in 0..r {
            rows.push(get_or(tracks.y, y, auto));
            if has_gutter {
                rows.push(get_or(gutter.y, y, zero));
            }
        }

        // Remove superfluous gutter tracks.
        if has_gutter {
            cols.pop();
            rows.pop();
        }

        Self { cols, rows, entries, vlines, hlines, has_gutter }
    }

    /// Get the grid entry in column `x` and row `y`.
    ///
    /// Returns `None` if it's a gutter cell.
    #[track_caller]
    pub(super) fn entry(&self, x: usize, y: usize) -> Option<&Entry> {
        assert!(x < self.cols.len());
        assert!(y < self.rows.len());

        if self.has_gutter {
            // Even columns and rows are children, odd ones are gutter.
            if x % 2 == 0 && y % 2 == 0 {
                let c = 1 + self.cols.len() / 2;
                self.entries.get((y / 2) * c + x / 2)
            } else {
                None
            }
        } else {
            let c = self.cols.len();
            self.entries.get(y * c + x)
        }
    }

    /// Get the content of the cell in column `x` and row `y`.
    ///
    /// Returns `None` if it's a gutter cell or merged position.
    #[track_caller]
    pub(super) fn cell(&self, x: usize, y: usize) -> Option<&Cell> {
        self.entry(x, y).and_then(Entry::as_cell)
    }

    /// Returns the position of the parent cell of the grid entry at the given
    /// position. It is guaranteed to have a non-gutter, non-merged cell at
    /// the returned position, due to how the grid is built.
    /// - If the entry at the given position is a cell, returns the given
    /// position.
    /// - If it is a merged cell, returns the parent cell's position.
    /// - If it is a gutter cell, returns None.
    #[track_caller]
    pub(super) fn parent_cell_position(&self, x: usize, y: usize) -> Option<Axes<usize>> {
        self.entry(x, y).map(|entry| match entry {
            Entry::Cell(_) => Axes::new(x, y),
            Entry::Merged { parent } => {
                let c = if self.has_gutter {
                    1 + self.cols.len() / 2
                } else {
                    self.cols.len()
                };
                let factor = if self.has_gutter { 2 } else { 1 };
                Axes::new(factor * (*parent % c), factor * (*parent / c))
            }
        })
    }

    /// Checks if the track with the given index is gutter.
    /// Does not check if the index is a valid track.
    #[inline]
    pub(super) fn is_gutter_track(&self, index: usize) -> bool {
        self.has_gutter && index % 2 == 1
    }

    /// Returns the effective colspan of a cell, considering the gutters it
    /// might span if the grid has gutters.
    #[inline]
    pub(super) fn effective_colspan_of_cell(&self, cell: &Cell) -> usize {
        if self.has_gutter {
            2 * cell.colspan.get() - 1
        } else {
            cell.colspan.get()
        }
    }

    /// Returns the effective rowspan of a cell, considering the gutters it
    /// might span if the grid has gutters.
    #[inline]
    pub(super) fn effective_rowspan_of_cell(&self, cell: &Cell) -> usize {
        if self.has_gutter {
            2 * cell.rowspan.get() - 1
        } else {
            cell.rowspan.get()
        }
    }
}

/// Given a cell's requested x and y, the vector with the resolved cell
/// positions, the `auto_index` counter (determines the position of the next
/// `(auto, auto)` cell) and the amount of columns in the grid, returns the
/// final index of this cell in the vector of resolved cells.
fn resolve_cell_position(
    cell_x: Smart<usize>,
    cell_y: Smart<usize>,
    resolved_cells: &[Option<Entry>],
    auto_index: &mut usize,
    columns: usize,
) -> HintedStrResult<usize> {
    // Translates a (x, y) position to the equivalent index in the final cell vector.
    // Errors if the position would be too large.
    let cell_index = |x, y: usize| {
        y.checked_mul(columns)
            .and_then(|row_index| row_index.checked_add(x))
            .ok_or_else(|| HintedString::from(eco_format!("cell position too large")))
    };
    match (cell_x, cell_y) {
        // Fully automatic cell positioning. The cell did not
        // request a coordinate.
        (Smart::Auto, Smart::Auto) => {
            // Let's find the first available position starting from the
            // automatic position counter, searching in row-major order.
            let mut resolved_index = *auto_index;
            while let Some(Some(_)) = resolved_cells.get(resolved_index) {
                // Skip any non-absent cell positions (`Some(None)`) to
                // determine where this cell will be placed. An out of bounds
                // position (thus `None`) is also a valid new position (only
                // requires expanding the vector).
                resolved_index += 1;
            }

            // Ensure the next cell with automatic position will be
            // placed after this one (maybe not immediately after).
            *auto_index = resolved_index + 1;

            Ok(resolved_index)
        }
        // Cell has chosen at least its column.
        (Smart::Custom(cell_x), cell_y) => {
            if cell_x >= columns {
                return Err(HintedString::from(eco_format!(
                    "cell could not be placed at invalid column {cell_x}"
                )));
            }
            if let Smart::Custom(cell_y) = cell_y {
                // Cell has chosen its exact position.
                cell_index(cell_x, cell_y)
            } else {
                // Cell has only chosen its column.
                // Let's find the first row which has that column available.
                let mut resolved_y = 0;
                while let Some(Some(_)) =
                    resolved_cells.get(cell_index(cell_x, resolved_y)?)
                {
                    // Try each row until either we reach an absent position
                    // (`Some(None)`) or an out of bounds position (`None`),
                    // in which case we'd create a new row to place this cell in.
                    resolved_y += 1;
                }
                cell_index(cell_x, resolved_y)
            }
        }
        // Cell has only chosen its row, not its column.
        (Smart::Auto, Smart::Custom(cell_y)) => {
            // Let's find the first column which has that row available.
            let first_row_pos = cell_index(0, cell_y)?;
            let last_row_pos = first_row_pos
                .checked_add(columns)
                .ok_or_else(|| eco_format!("cell position too large"))?;

            (first_row_pos..last_row_pos)
                .find(|possible_index| {
                    // Much like in the previous cases, we skip any occupied
                    // positions until we either reach an absent position
                    // (`Some(None)`) or an out of bounds position (`None`),
                    // in which case we can just expand the vector enough to
                    // place this cell. In either case, we found an available
                    // position.
                    !matches!(resolved_cells.get(*possible_index), Some(Some(_)))
                })
                .ok_or_else(|| {
                    eco_format!(
                        "cell could not be placed in row {cell_y} because it was full"
                    )
                })
                .hint("try specifying your cells in a different order")
        }
    }
}

/// Performs grid layout.
pub struct GridLayouter<'a> {
    /// The grid of cells.
    pub(super) grid: &'a CellGrid,
    /// The regions to layout children into.
    pub(super) regions: Regions<'a>,
    /// The inherited styles.
    pub(super) styles: StyleChain<'a>,
    /// Resolved column sizes.
    pub(super) rcols: Vec<Abs>,
    /// The sum of `rcols`.
    pub(super) width: Abs,
    /// Resolve row sizes, by region.
    pub(super) rrows: Vec<Vec<RowPiece>>,
    /// Rows in the current region.
    pub(super) lrows: Vec<Row>,
    /// The amount of unbreakable rows remaining to be added in the unbreakable
    /// row group. When this is 0, no unbreakable row group is being built, and
    /// the grid layout process continues normally. But when this is > 0, any
    /// future rows must be added to 'unbreakable_row_group' first before being
    /// pushed to 'lrows'.
    pub(super) unbreakable_rows_left: usize,
    /// Rowspans to layout after all regions were finished.
    pub(super) rowspans: Vec<Rowspan>,
    /// The initial size of the current region before we started subtracting.
    pub(super) initial: Size,
    /// Frames for finished regions.
    pub(super) finished: Vec<Frame>,
    /// Whether this is an RTL grid.
    pub(super) is_rtl: bool,
    /// The span of the grid element.
    pub(super) span: Span,
}

/// Details about a resulting row piece.
#[derive(Debug)]
pub struct RowPiece {
    /// The height of the segment.
    pub height: Abs,
    /// The index of the row.
    pub y: usize,
}

/// Produced by initial row layout, auto and relative rows are already finished,
/// fractional rows not yet.
/// Includes new rowspans found in this row.
pub(super) enum Row {
    /// Finished row frame of auto or relative row with y index.
    Frame(Frame, usize),
    /// Fractional row with y index.
    Fr(Fr, usize),
}

impl<'a> GridLayouter<'a> {
    /// Create a new grid layouter.
    ///
    /// This prepares grid layout by unifying content and gutter tracks.
    pub fn new(
        grid: &'a CellGrid,
        regions: Regions<'a>,
        styles: StyleChain<'a>,
        span: Span,
    ) -> Self {
        // We use these regions for auto row measurement. Since at that moment,
        // columns are already sized, we can enable horizontal expansion.
        let mut regions = regions;
        regions.expand = Axes::new(true, false);

        Self {
            grid,
            regions,
            styles,
            rcols: vec![Abs::zero(); grid.cols.len()],
            width: Abs::zero(),
            rrows: vec![],
            lrows: vec![],
            unbreakable_rows_left: 0,
            rowspans: vec![],
            initial: regions.size,
            finished: vec![],
            is_rtl: TextElem::dir_in(styles) == Dir::RTL,
            span,
        }
    }

    /// Determines the columns sizes and then layouts the grid row-by-row.
    pub fn layout(mut self, engine: &mut Engine) -> SourceResult<Fragment> {
        self.measure_columns(engine)?;

        for y in 0..self.grid.rows.len() {
            // Skip to next region if current one is full, but only for content
            // rows, not for gutter rows, and only if we aren't laying out an
            // unbreakable group of rows.
            let is_content_row = !self.grid.is_gutter_track(y);
            if self.unbreakable_rows_left == 0 && self.regions.is_full() && is_content_row
            {
                self.finish_region(engine)?;
            }

            // Don't layout gutter rows at the top of a region.
            if is_content_row || !self.lrows.is_empty() {
                self.check_for_rowspans(y);
                self.check_for_unbreakable_rows(y, engine)?;

                match self.grid.rows[y] {
                    Sizing::Auto => self.layout_auto_row(engine, y)?,
                    Sizing::Rel(v) => self.layout_relative_row(engine, v, y)?,
                    Sizing::Fr(v) => {
                        self.lrows.push(Row::Fr(v, y));
                    }
                }
            }

            self.unbreakable_rows_left = self.unbreakable_rows_left.saturating_sub(1);
        }

        self.finish_region(engine)?;
        self.layout_rowspans(engine)?;
        self.render_fills_strokes()
    }

    /// Add lines and backgrounds.
    fn render_fills_strokes(mut self) -> SourceResult<Fragment> {
        let mut finished = std::mem::take(&mut self.finished);
        for (frame, rows) in finished.iter_mut().zip(&self.rrows) {
            if self.rcols.is_empty() || rows.is_empty() {
                continue;
            }

            // Render grid lines.
            // We collect lines into a vector before rendering so we can sort
            // them based on thickness, such that the lines with largest
            // thickness are drawn on top; and also so we can prepend all of
            // them at once in the frame, as calling prepend() for each line,
            // and thus pushing all frame items forward each time, would result
            // in quadratic complexity.
            let mut lines = vec![];

            // Render vertical lines.
            // Render them first so horizontal lines have priority later.
            for (x, dx) in points(self.rcols.iter().copied()).enumerate() {
                let dx = if self.is_rtl { self.width - dx } else { dx };
                let is_end_border = x == self.grid.cols.len();
                let vlines_at_column = self
                    .grid
                    .vlines
                    .get(if !self.grid.has_gutter {
                        x
                    } else if is_end_border {
                        // The end border has its own vector of lines, but
                        // dividing it by 2 and flooring would give us the
                        // vector of lines with the index of the last column.
                        // Add 1 so we get the border's lines.
                        x / 2 + 1
                    } else {
                        // If x is a gutter column, this will round down to the
                        // index of the previous content column, which is
                        // intentional - the only lines which can appear before
                        // a gutter column are lines for the previous column
                        // marked with "LinePosition::After". Therefore, we get
                        // the previous column's lines. Worry not, as
                        // 'generate_line_segments' will correctly filter lines
                        // based on their LinePosition for us.
                        //
                        // If x is a content column, this will correctly return
                        // its index before applying gutters, so nothing
                        // special here (lines with "LinePosition::After" would
                        // then be ignored for this column, as we are drawing
                        // lines before it, not after).
                        x / 2
                    })
                    .map(|vlines| &**vlines)
                    .unwrap_or(&[]);
                let tracks = rows.iter().map(|row| (row.y, row.height));

                // Determine all different line segments we have to draw in
                // this column, and convert them to points and shapes.
                //
                // Even a single, uniform line might generate more than one
                // segment, if it happens to cross a colspan (over which it
                // must not be drawn).
                let segments = generate_line_segments(
                    self.grid,
                    tracks,
                    x,
                    vlines_at_column,
                    is_end_border,
                    vline_stroke_at_row,
                )
                .map(|segment| {
                    let LineSegment { stroke, offset: dy, length, priority } = segment;
                    let stroke = (*stroke).clone().unwrap_or_default();
                    let thickness = stroke.thickness;
                    let half = thickness / 2.0;
                    let target = Point::with_y(length + thickness);
                    let vline = Geometry::Line(target).stroked(stroke);
                    (
                        thickness,
                        priority,
                        Point::new(dx, dy - half),
                        FrameItem::Shape(vline, self.span),
                    )
                });

                lines.extend(segments);
            }

            // Render horizontal lines.
            // They are rendered second as they default to appearing on top.
            // First, calculate their offsets from the top of the frame.
            let hline_offsets = points(rows.iter().map(|piece| piece.height));

            // Additionally, determine their indices (the indices of the
            // rows they are drawn on top of). In principle, this will
            // correspond to the rows' indices directly, except for the
            // first and last hlines, which must be 0 and (amount of rows)
            // respectively, as they are always drawn (due to being part of
            // the table's border).
            let hline_indices = std::iter::once(0)
                .chain(rows.iter().map(|piece| piece.y).skip(1))
                .chain(std::iter::once(self.grid.rows.len()));

            for (y, dy) in hline_indices.zip(hline_offsets) {
                let is_bottom_border = y == self.grid.rows.len();
                let hlines_at_row = self
                    .grid
                    .hlines
                    .get(if !self.grid.has_gutter {
                        y
                    } else if is_bottom_border {
                        y / 2 + 1
                    } else {
                        // Check the vlines loop for an explanation regarding
                        // these index operations.
                        y / 2
                    })
                    .map(|hlines| &**hlines)
                    .unwrap_or(&[]);
                let tracks = self.rcols.iter().copied().enumerate();

                // Determine all different line segments we have to draw in
                // this row, and convert them to points and shapes.
                let segments = generate_line_segments(
                    self.grid,
                    tracks,
                    y,
                    hlines_at_row,
                    is_bottom_border,
                    |grid, y, x, stroke| hline_stroke_at_column(grid, rows, y, x, stroke),
                )
                .map(|segment| {
                    let LineSegment { stroke, offset: dx, length, priority } = segment;
                    let stroke = (*stroke).clone().unwrap_or_default();
                    let thickness = stroke.thickness;
                    let half = thickness / 2.0;
                    let dx = if self.is_rtl { self.width - dx - length } else { dx };
                    let target = Point::with_x(length + thickness);
                    let hline = Geometry::Line(target).stroked(stroke);
                    (
                        thickness,
                        priority,
                        Point::new(dx - half, dy),
                        FrameItem::Shape(hline, self.span),
                    )
                });

                // Draw later (after we sort all lines below.)
                lines.extend(segments);
            }

            // Sort by increasing thickness, so that we draw larger strokes
            // on top. When the thickness is the same, sort by priority.
            //
            // Sorting by thickness avoids layering problems where a smaller
            // hline appears "inside" a larger vline. When both have the same
            // size, hlines are drawn on top (since the sort is stable, and
            // they are pushed later).
            lines.sort_by_key(|(thickness, priority, ..)| (*thickness, *priority));

            // Render cell backgrounds.
            // We collect them into a vector so they can all be prepended at
            // once to the frame, together with lines.
            let mut fills = vec![];

            // Reverse with RTL so that later columns start first.
            let mut dx = Abs::zero();
            for (x, &col) in self.rcols.iter().enumerate().rev_if(self.is_rtl) {
                let mut dy = Abs::zero();
                for row in rows {
                    // We want to only draw the fill starting at the parent
                    // positions of cells. However, sometimes the parent
                    // position is absent from the current region, either
                    // because the first few rows of a rowspan were empty auto
                    // rows and thus removed from layout, or because the parent
                    // cell was in a previous region (in which case we'd want
                    // to draw its fill again, in the current region).
                    // Therefore, we first analyze the parent position to see
                    // if the current row would be the first row spanned by the
                    // parent cell in this region. If so, this means we have to
                    // start drawing the cell's fill here. If not, we ignore
                    // the position `(x, row.y)`, as its fill will already have
                    // been rendered before.
                    // NOTE: In the case of gutter rows, we have to check the
                    // row below before discarding them fully, because a
                    // gutter row might be the first row spanned by a rowspan
                    // in this region (e.g. if the first row was empty and
                    // therefore removed), so its fill could start in that
                    // gutter row.
                    let Some(Axes { x: parent_x, y: parent_y }) =
                        self.grid.parent_cell_position(
                            x,
                            row.y + if self.grid.has_gutter { row.y % 2 } else { 0 },
                        )
                    else {
                        // Ignore gutter columns.
                        dy += row.height;
                        continue;
                    };

                    if parent_x != x {
                        // This isn't the first column spanned by the cell, so
                        // we already rendered its fill in a previous
                        // iteration of the outer loop. Skip.
                        dy += row.height;
                        continue;
                    }

                    // The "effective parent Y" is the first row spanned by the
                    // parent cell in this region.
                    // Even if the parent cell's fill was already drawn in a
                    // previous region, we must render it again in later
                    // regions.
                    let is_effective_parent_y = {
                        parent_y == row.y
                            || parent_y < row.y
                                && rows.iter().find(|row| row.y >= parent_y).is_some_and(
                                    |first_spanned_row| first_spanned_row.y == row.y,
                                )
                    };

                    // Ensure we are at the very first row spanned by the cell
                    // in this region before rendering fill. Note that this
                    // will always be true if the current cell has rowspan == 1
                    // and we're not currently handling a gutter row.
                    if is_effective_parent_y {
                        let cell = self.grid.cell(parent_x, parent_y).unwrap();
                        let fill = cell.fill.clone();
                        if let Some(fill) = fill {
                            let rowspan = self.grid.effective_rowspan_of_cell(cell);
                            let height = if rowspan == 1 {
                                row.height
                            } else {
                                rows.iter()
                                    .filter(|row| {
                                        (parent_y..parent_y + rowspan).contains(&row.y)
                                    })
                                    .map(|row| row.height)
                                    .sum()
                            };
                            let width = self.cell_spanned_width(cell, x);
                            // In the grid, cell colspans expand to the right,
                            // so we're at the leftmost (lowest 'x') column
                            // spanned by the cell. However, in RTL, cells
                            // expand to the left. Therefore, without the
                            // offset below, cell fills would start at the
                            // rightmost visual position of a cell and extend
                            // over to unrelated columns to the right in RTL.
                            // We avoid this by ensuring the fill starts at the
                            // very left of the cell, even with colspan > 1.
                            let offset =
                                if self.is_rtl { -width + col } else { Abs::zero() };
                            let pos = Point::new(dx + offset, dy);
                            let size = Size::new(width, height);
                            let rect = Geometry::Rect(size).filled(fill);
                            fills.push((pos, FrameItem::Shape(rect, self.span)));
                        }
                    }
                    dy += row.height;
                }
                dx += col;
            }

            // Now we render each fill and stroke by prepending to the frame,
            // such that both appear below cell contents. Fills come first so
            // that they appear below lines.
            frame.prepend_multiple(
                fills
                    .into_iter()
                    .chain(lines.into_iter().map(|(_, _, point, shape)| (point, shape))),
            );
        }

        Ok(Fragment::frames(finished))
    }

    /// Determine all column sizes.
    fn measure_columns(&mut self, engine: &mut Engine) -> SourceResult<()> {
        // Sum of sizes of resolved relative tracks.
        let mut rel = Abs::zero();

        // Sum of fractions of all fractional tracks.
        let mut fr = Fr::zero();

        // Resolve the size of all relative columns and compute the sum of all
        // fractional tracks.
        for (&col, rcol) in self.grid.cols.iter().zip(&mut self.rcols) {
            match col {
                Sizing::Auto => {}
                Sizing::Rel(v) => {
                    let resolved =
                        v.resolve(self.styles).relative_to(self.regions.base().x);
                    *rcol = resolved;
                    rel += resolved;
                }
                Sizing::Fr(v) => fr += v,
            }
        }

        // Size that is not used by fixed-size columns.
        let available = self.regions.size.x - rel;
        if available >= Abs::zero() {
            // Determine size of auto columns.
            let (auto, count) = self.measure_auto_columns(engine, available)?;

            // If there is remaining space, distribute it to fractional columns,
            // otherwise shrink auto columns.
            let remaining = available - auto;
            if remaining >= Abs::zero() {
                self.grow_fractional_columns(remaining, fr);
            } else {
                self.shrink_auto_columns(available, count);
            }
        }

        // Sum up the resolved column sizes once here.
        self.width = self.rcols.iter().sum();

        Ok(())
    }

    /// Total width spanned by the cell (among resolved columns).
    /// Includes spanned gutter columns.
    pub(super) fn cell_spanned_width(&self, cell: &Cell, x: usize) -> Abs {
        let colspan = self.grid.effective_colspan_of_cell(cell);
        self.rcols.iter().skip(x).take(colspan).sum()
    }

    /// Measure the size that is available to auto columns.
    fn measure_auto_columns(
        &mut self,
        engine: &mut Engine,
        available: Abs,
    ) -> SourceResult<(Abs, usize)> {
        let mut auto = Abs::zero();
        let mut count = 0;
        let all_frac_cols = self
            .grid
            .cols
            .iter()
            .enumerate()
            .filter(|(_, col)| col.is_fractional())
            .map(|(x, _)| x)
            .collect::<Vec<_>>();

        // Determine size of auto columns by laying out all cells in those
        // columns, measuring them and finding the largest one.
        for (x, &col) in self.grid.cols.iter().enumerate() {
            if col != Sizing::Auto {
                continue;
            }

            let mut resolved = Abs::zero();
            for y in 0..self.grid.rows.len() {
                // We get the parent cell in case this is a merged position.
                let Some(Axes { x: parent_x, y: parent_y }) =
                    self.grid.parent_cell_position(x, y)
                else {
                    continue;
                };
                if parent_y != y {
                    // Don't check the width of rowspans more than once.
                    continue;
                }
                let cell = self.grid.cell(parent_x, parent_y).unwrap();
                let colspan = self.grid.effective_colspan_of_cell(cell);
                if colspan > 1 {
                    let last_spanned_auto_col = self
                        .grid
                        .cols
                        .iter()
                        .enumerate()
                        .skip(parent_x)
                        .take(colspan)
                        .rev()
                        .find(|(_, col)| **col == Sizing::Auto)
                        .map(|(x, _)| x);

                    if last_spanned_auto_col != Some(x) {
                        // A colspan only affects the size of the last spanned
                        // auto column.
                        continue;
                    }
                }

                if colspan > 1
                    && self.regions.size.x.is_finite()
                    && !all_frac_cols.is_empty()
                    && all_frac_cols
                        .iter()
                        .all(|x| (parent_x..parent_x + colspan).contains(x))
                {
                    // Additionally, as a heuristic, a colspan won't affect the
                    // size of auto columns if it already spans all fractional
                    // columns, since those would already expand to provide all
                    // remaining available after auto column sizing to that
                    // cell. However, this heuristic is only valid in finite
                    // regions (pages without 'auto' width), since otherwise
                    // the fractional columns don't expand at all.
                    continue;
                }

                // Sum the heights of spanned rows to find the expected
                // available height for the cell, unless it spans a fractional
                // or auto column.
                let rowspan = self.grid.effective_rowspan_of_cell(cell);
                let height = self
                    .grid
                    .rows
                    .iter()
                    .skip(y)
                    .take(rowspan)
                    .try_fold(Abs::zero(), |acc, col| {
                        // For relative rows, we can already resolve the correct
                        // base and for auto and fr we could only guess anyway.
                        match col {
                            Sizing::Rel(v) => Some(
                                acc + v
                                    .resolve(self.styles)
                                    .relative_to(self.regions.base().y),
                            ),
                            _ => None,
                        }
                    })
                    .unwrap_or_else(|| self.regions.base().y);
                // Don't expand this auto column more than the cell actually
                // needs. To do this, we check how much the other, previously
                // resolved columns provide to the cell in terms of width
                // (if it is a colspan), and subtract this from its expected
                // width when comparing with other cells in this column. Note
                // that, since this is the last auto column spanned by this
                // cell, all other auto columns will already have been resolved
                // and will be considered.
                // Only fractional columns will be excluded from this
                // calculation, which can lead to auto columns being expanded
                // unnecessarily when cells span both a fractional column and
                // an auto column. One mitigation for this is the heuristic
                // used above to not expand the last auto column spanned by a
                // cell if it spans all fractional columns in a finite region.
                let already_covered_width = self.cell_spanned_width(cell, parent_x);

                let size = Size::new(available, height);
                let pod = Regions::one(size, Axes::splat(false));
                let frame = cell.measure(engine, self.styles, pod)?.into_frame();
                resolved.set_max(frame.width() - already_covered_width);
            }

            self.rcols[x] = resolved;
            auto += resolved;
            count += 1;
        }

        Ok((auto, count))
    }

    /// Distribute remaining space to fractional columns.
    fn grow_fractional_columns(&mut self, remaining: Abs, fr: Fr) {
        if fr.is_zero() {
            return;
        }

        for (&col, rcol) in self.grid.cols.iter().zip(&mut self.rcols) {
            if let Sizing::Fr(v) = col {
                *rcol = v.share(fr, remaining);
            }
        }
    }

    /// Redistribute space to auto columns so that each gets a fair share.
    fn shrink_auto_columns(&mut self, available: Abs, count: usize) {
        let mut last;
        let mut fair = -Abs::inf();
        let mut redistribute = available;
        let mut overlarge = count;
        let mut changed = true;

        // Iteratively remove columns that don't need to be shrunk.
        while changed && overlarge > 0 {
            changed = false;
            last = fair;
            fair = redistribute / (overlarge as f64);

            for (&col, &rcol) in self.grid.cols.iter().zip(&self.rcols) {
                // Remove an auto column if it is not overlarge (rcol <= fair),
                // but also hasn't already been removed (rcol > last).
                if col == Sizing::Auto && rcol <= fair && rcol > last {
                    redistribute -= rcol;
                    overlarge -= 1;
                    changed = true;
                }
            }
        }

        // Redistribute space fairly among overlarge columns.
        for (&col, rcol) in self.grid.cols.iter().zip(&mut self.rcols) {
            if col == Sizing::Auto && *rcol > fair {
                *rcol = fair;
            }
        }
    }

    /// Layout a row with automatic height. Such a row may break across multiple
    /// regions.
    fn layout_auto_row(&mut self, engine: &mut Engine, y: usize) -> SourceResult<()> {
        let unbreakable = self.unbreakable_rows_left > 0;

        // Determine the size for each region of the row. If the first region
        // ends up empty for some column, skip the region and remeasure.
        let mut resolved = match self.measure_auto_row(
            engine,
            y,
            true,
            unbreakable,
            self.unbreakable_rows_left,
            &UnbreakableRowGroup::default(),
        )? {
            Some(resolved) => resolved,
            None => {
                self.finish_region(engine)?;
                self.measure_auto_row(
                    engine,
                    y,
                    false,
                    unbreakable,
                    self.unbreakable_rows_left,
                    &UnbreakableRowGroup::default(),
                )?
                .unwrap()
            }
        };

        // Nothing to layout.
        if resolved.is_empty() {
            return Ok(());
        }

        // Layout into a single region.
        if let &[first] = resolved.as_slice() {
            let frame = self.layout_single_row(engine, first, y)?;
            self.push_row(frame, y);
            return Ok(());
        }

        // Expand all but the last region.
        // Skip the first region if the space is eaten up by an fr row.
        let len = resolved.len();
        for (region, target) in self
            .regions
            .iter()
            .zip(&mut resolved[..len - 1])
            .skip(self.lrows.iter().any(|row| matches!(row, Row::Fr(..))) as usize)
        {
            target.set_max(region.y);
        }

        // Layout into multiple regions.
        let fragment = self.layout_multi_row(engine, &resolved, y)?;
        let len = fragment.len();
        for (i, frame) in fragment.into_iter().enumerate() {
            self.push_row(frame, y);
            if i + 1 < len {
                self.finish_region(engine)?;
            }
        }

        Ok(())
    }

    /// Measure the regions sizes of an auto row. The option is always `Some(_)`
    /// if `can_skip` is false.
    /// If `unbreakable` is true, this function shall only return a single
    /// frame. Useful when an unbreakable rowspan crosses this auto row.
    /// The `row_group_data` option is used within the unbreakable row group
    /// simulator to predict the height of the auto row if previous rows in the
    /// group were placed in the same region.
    pub(super) fn measure_auto_row(
        &self,
        engine: &mut Engine,
        y: usize,
        can_skip: bool,
        unbreakable: bool,
        unbreakable_rows_left: usize,
        row_group_data: &UnbreakableRowGroup,
    ) -> SourceResult<Option<Vec<Abs>>> {
        let mut resolved: Vec<Abs> = vec![];
        let mut pending_rowspans: Vec<(usize, usize, Vec<Abs>)> = vec![];

        for x in 0..self.rcols.len() {
            // Get the parent cell in case this is a merged position.
            let Some(Axes { x: parent_x, y: parent_y }) =
                self.grid.parent_cell_position(x, y)
            else {
                // Skip gutter columns.
                continue;
            };
            if parent_x != x {
                // Only check the height of a colspan once.
                continue;
            }
            // The parent cell is never a gutter or merged position.
            let cell = self.grid.cell(parent_x, parent_y).unwrap();
            let rowspan = self.grid.effective_rowspan_of_cell(cell);

            // This variable is used to construct a custom backlog if the cell
            // is a rowspan. When measuring, we join the heights from previous
            // regions to the current backlog to form the rowspan's expected
            // backlog.
            let rowspan_backlog: Vec<Abs>;

            // Each declaration, from left to right:
            // 1. The height available to the cell in the first region.
            // Usually, this will just be the size remaining in the current
            // region.
            // 2. The backlog of upcoming region heights to specify as
            // available to the cell.
            // 3. The total height of the cell covered by previously spanned
            // rows in this region. This is used by rowspans to be able to tell
            // how much the auto row needs to expand.
            // 4. The amount of frames laid out by this cell in previous
            // regions. When the cell isn't a rowspan, this is always zero.
            // These frames are skipped after measuring.
            let (height, backlog, height_in_this_region, frames_in_previous_regions);
            if rowspan == 1 {
                // Not a rowspan, so the cell only occupies this row. Therefore:
                // 1. When we measure the cell below, use the available height
                // remaining in the region as the height it has available.
                // Ensure we subtract the height of previous rows in the
                // unbreakable row group, if we're currently simulating it.
                // 2. Also use the region's backlog when measuring.
                // 3. No height occupied by this cell in this region so far.
                // 4. Yes, this cell started in this region.
                height = self.regions.size.y - row_group_data.height;
                backlog = self.regions.backlog;
                height_in_this_region = Abs::zero();
                frames_in_previous_regions = 0;
            } else {
                let last_spanned_auto_row = self
                    .grid
                    .rows
                    .iter()
                    .enumerate()
                    .skip(parent_y)
                    .take(rowspan)
                    .rev()
                    .find(|(_, &row)| row == Sizing::Auto)
                    .map(|(y, _)| y);

                if last_spanned_auto_row != Some(y) {
                    // A rowspan should only affect the height of its last
                    // spanned auto row.
                    continue;
                }

                // Height of the rowspan covered by spanned rows in the current
                // region.
                let laid_out_height: Abs = self
                    .lrows
                    .iter()
                    .filter_map(|row| match row {
                        Row::Frame(frame, y)
                            if (parent_y..parent_y + rowspan).contains(y) =>
                        {
                            Some(frame.height())
                        }
                        // Either we have a row outside of the rowspan, or a
                        // fractional row, whose size we can't really guess.
                        _ => None,
                    })
                    .sum();

                // If we're currently simulating an unbreakable row group, also
                // consider the height of previously spanned rows which are in
                // the row group but not yet laid out.
                let unbreakable_height: Abs = row_group_data
                    .rows
                    .iter()
                    .filter(|(y, _)| (parent_y..parent_y + rowspan).contains(y))
                    .map(|(_, height)| height)
                    .sum();

                height_in_this_region = laid_out_height + unbreakable_height;

                // Ensure we will measure the rowspan with the correct heights.
                // For that, we will gather the total height spanned by this
                // rowspan in previous regions.
                if let Some([rowspan_height, rowspan_other_heights @ ..]) = self
                    .rowspans
                    .iter()
                    .find(|data| data.x == parent_x && data.y == parent_y)
                    .map(|data| &*data.heights)
                {
                    // The rowspan started in a previous region (as it already
                    // has at least one region height).
                    // Therefore, its initial height will be the height in its
                    // first spanned region, and the backlog will be the
                    // remaining heights, plus the current region's size, plus
                    // the current backlog.
                    frames_in_previous_regions = rowspan_other_heights.len() + 1;
                    rowspan_backlog = if unbreakable {
                        // No extra backlog if this is an unbreakable auto row.
                        // Ensure, when measuring, that the rowspan can be laid
                        // out through all spanned rows which were already laid
                        // out so far, but don't go further than this region.
                        rowspan_other_heights
                            .iter()
                            .copied()
                            .chain(std::iter::once(self.initial.y))
                            .collect::<Vec<_>>()
                    } else {
                        // This auto row is breakable. Therefore, join the
                        // rowspan's already laid out heights with the current
                        // region's height and current backlog to ensure a good
                        // level of accuracy in the measurements.
                        rowspan_other_heights
                            .iter()
                            .copied()
                            .chain(std::iter::once(self.initial.y))
                            .chain(self.regions.backlog.iter().copied())
                            .collect::<Vec<_>>()
                    };

                    (height, backlog) = (*rowspan_height, &rowspan_backlog);
                } else {
                    // The rowspan started in the current region, as its vector
                    // of heights in regions is currently empty.
                    // Therefore, the initial height it has available will be
                    // the current available size, plus the size spanned in
                    // previous rows in this region (and/or unbreakable row
                    // group, if it's being simulated).
                    // The backlog will be the same as the current one.
                    frames_in_previous_regions = 0;
                    height = height_in_this_region + self.regions.size.y;
                    backlog = &self.regions.backlog;
                }
            }

            let width = self.cell_spanned_width(cell, x);

            let frames = if unbreakable {
                // Force cell to fit into a single region when the row is unbreakable.
                let mut pod = Regions::one(Axes::new(width, height), Axes::splat(true));
                pod.full = self.regions.full;
                if frames_in_previous_regions == 0 {
                    // Cells which started at this region will only have a
                    // single frame for measuring purposes - even if they're
                    // breakable rowspans, as a best effort.
                    vec![cell.measure(engine, self.styles, pod)?.into_frame()]
                } else {
                    // Best effort to conciliate a breakable rowspan which
                    // started at a previous region going through an
                    // unbreakable auto row. Ensure it goes through previously
                    // laid out regions, but stops at this one when measuring.
                    pod.backlog = backlog;
                    cell.measure(engine, self.styles, pod)?.into_frames()
                }
            } else {
                // This row is breakable, so measure the cell normally, with
                // the initial height and backlog determined previously.
                let mut pod = self.regions;
                pod.size = Axes::new(width, height);
                pod.backlog = backlog;
                cell.measure(engine, self.styles, pod)?.into_frames()
            };

            // Skip the first region if one cell in it is empty. Then,
            // remeasure.
            if let [first, rest @ ..] = frames.as_slice() {
                if can_skip
                    && !unbreakable
                    && first.is_empty()
                    && rest.iter().any(|frame| !frame.is_empty())
                {
                    return Ok(None);
                }
            }

            // Skip frames from previous regions if applicable.
            let mut sizes = frames
                .iter()
                .skip(frames_in_previous_regions)
                .map(|frame| frame.height())
                .collect::<Vec<_>>();

            // Don't expand this row more than the cell needs.
            // To figure out how much height the cell needs, we must first
            // subtract, from the cell's expected height, the already resolved
            // heights of its spanned rows. Note that this is the last spanned
            // auto row, so all previous auto rows were already resolved, as
            // well as fractional rows in previous regions.
            // Additionally, we subtract the heights of fixed-size rows which
            // weren't laid out yet, since those heights won't change in
            // principle.
            // Upcoming fractional rows are ignored.
            // Upcoming gutter rows might be removed, so we need to simulate
            // them.
            if rowspan > 1 {
                // Subtract already covered height from previous rows in the
                // current region, if applicable.
                let mut already_covered_height = height_in_this_region;

                // Remove frames which were already covered by previous rows
                // spanned by this cell.
                while already_covered_height > Abs::zero()
                    && sizes.first().is_some_and(|&size| size <= already_covered_height)
                {
                    already_covered_height -= sizes.remove(0);
                }
                // Subtract remaining height from the first frame.
                if let Some(first_frame_size) = sizes.first_mut() {
                    *first_frame_size -= already_covered_height;
                }

                let last_spanned_row = parent_y + rowspan - 1;

                // When the rowspan is unbreakable, or all of its upcoming
                // spanned rows are in the same unbreakable row group, its
                // spanned gutter will certainly be in the same region as all
                // of its other spanned rows, thus gutters won't be removed,
                // and we can safely reduce how much the auto row expands by
                // without using simulation.
                let is_effectively_unbreakable_rowspan = self
                    .is_unbreakable_cell(cell, y)
                    || y + unbreakable_rows_left > last_spanned_row;

                // If the rowspan doesn't end at this row and the grid has
                // gutter, we will need to run a simulation to find out how
                // much to expand this row by later. This is because gutters
                // spanned by this rowspan might be removed if they appear
                // around a pagebreak, so the auto row might have to expand a
                // bit more to compensate for the missing gutter height.
                // However, unbreakable rowspans aren't affected by that
                // problem.
                if y != last_spanned_row
                    && !sizes.is_empty()
                    && self.grid.has_gutter
                    && !is_effectively_unbreakable_rowspan
                {
                    pending_rowspans.push((parent_y, rowspan, sizes));
                    continue;
                }

                // We can only predict the resolved size of upcoming fixed-size
                // rows, but not fractional rows. In the future, we might be
                // able to simulate and circumvent the problem with fractional
                // rows. Relative rows are currently always measured relative
                // to the first region as well.
                // We can ignore auto rows since this is the last spanned auto
                // row.
                let will_be_covered_height: Abs = self
                    .grid
                    .rows
                    .iter()
                    .skip(y + 1)
                    .take(last_spanned_row - y)
                    .map(|row| match row {
                        Sizing::Rel(v) => {
                            v.resolve(self.styles).relative_to(self.regions.base().y)
                        }
                        _ => Abs::zero(),
                    })
                    .sum();

                // Remove or reduce the sizes of the rowspan at future regions
                // where it will already be covered by further rows spanned by
                // it.
                subtract_end_sizes(&mut sizes, will_be_covered_height);
            }

            let mut sizes = sizes.into_iter();

            for (target, size) in resolved.iter_mut().zip(&mut sizes) {
                target.set_max(size);
            }

            // New heights are maximal by virtue of being new. Note that
            // this extend only uses the rest of the sizes iterator.
            resolved.extend(sizes);
        }

        // Simulate the upcoming regions in order to predict how much we need
        // to expand this auto row for rowspans which span gutter.
        if !pending_rowspans.is_empty() {
            self.simulate_and_measure_rowspans_in_auto_row(
                y,
                &mut resolved,
                &pending_rowspans,
                unbreakable_rows_left,
                row_group_data,
                engine,
            )?;
        }

        debug_assert!(!unbreakable || resolved.len() < 2);

        Ok(Some(resolved))
    }

    /// Layout a row with relative height. Such a row cannot break across
    /// multiple regions, but it may force a region break.
    fn layout_relative_row(
        &mut self,
        engine: &mut Engine,
        v: Rel<Length>,
        y: usize,
    ) -> SourceResult<()> {
        let resolved = v.resolve(self.styles).relative_to(self.regions.base().y);
        let frame = self.layout_single_row(engine, resolved, y)?;

        // Skip to fitting region, but only if we aren't part of an unbreakable
        // row group.
        let height = frame.height();
        while self.unbreakable_rows_left == 0
            && !self.regions.size.y.fits(height)
            && !self.regions.in_last()
        {
            self.finish_region(engine)?;

            // Don't skip multiple regions for gutter and don't push a row.
            if self.grid.is_gutter_track(y) {
                return Ok(());
            }
        }

        self.push_row(frame, y);

        Ok(())
    }

    /// Layout a row with fixed height and return its frame.
    fn layout_single_row(
        &mut self,
        engine: &mut Engine,
        height: Abs,
        y: usize,
    ) -> SourceResult<Frame> {
        if !height.is_finite() {
            bail!(self.span, "cannot create grid with infinite height");
        }

        let mut output = Frame::soft(Size::new(self.width, height));
        let mut pos = Point::zero();

        // Reverse the column order when using RTL.
        for (x, &rcol) in self.rcols.iter().enumerate().rev_if(self.is_rtl) {
            if let Some(cell) = self.grid.cell(x, y) {
                // Rowspans have a separate layout step
                if cell.rowspan.get() == 1 {
                    let width = self.cell_spanned_width(cell, x);
                    let size = Size::new(width, height);
                    let mut pod = Regions::one(size, Axes::splat(true));
                    if self.grid.rows[y] == Sizing::Auto {
                        pod.full = self.regions.full;
                    }
                    let mut frame = cell.layout(engine, self.styles, pod)?.into_frame();
                    if self.is_rtl {
                        // In the grid, cell colspans expand to the right,
                        // so we're at the leftmost (lowest 'x') column
                        // spanned by the cell. However, in RTL, cells
                        // expand to the left. Therefore, without the
                        // offset below, the cell's contents would be laid out
                        // starting at its rightmost visual position and extend
                        // over to unrelated cells to its right in RTL.
                        // We avoid this by ensuring the rendered cell starts at
                        // the very left of the cell, even with colspan > 1.
                        let offset = Point::with_x(-width + rcol);
                        frame.translate(offset);
                    }
                    output.push_frame(pos, frame);
                }
            }

            pos.x += rcol;
        }

        Ok(output)
    }

    /// Layout a row spanning multiple regions.
    fn layout_multi_row(
        &mut self,
        engine: &mut Engine,
        heights: &[Abs],
        y: usize,
    ) -> SourceResult<Fragment> {
        // Prepare frames.
        let mut outputs: Vec<_> = heights
            .iter()
            .map(|&h| Frame::soft(Size::new(self.width, h)))
            .collect();

        // Prepare regions.
        let size = Size::new(self.width, heights[0]);
        let mut pod = Regions::one(size, Axes::splat(true));
        pod.full = self.regions.full;
        pod.backlog = &heights[1..];

        // Layout the row.
        let mut pos = Point::zero();
        for (x, &rcol) in self.rcols.iter().enumerate().rev_if(self.is_rtl) {
            if let Some(cell) = self.grid.cell(x, y) {
                // Rowspans have a separate layout step
                if cell.rowspan.get() == 1 {
                    let width = self.cell_spanned_width(cell, x);
                    pod.size.x = width;

                    // Push the layouted frames into the individual output frames.
                    let fragment = cell.layout(engine, self.styles, pod)?;
                    for (output, mut frame) in outputs.iter_mut().zip(fragment) {
                        if self.is_rtl {
                            let offset = Point::with_x(-width + rcol);
                            frame.translate(offset);
                        }
                        output.push_frame(pos, frame);
                    }
                }
            }

            pos.x += rcol;
        }

        Ok(Fragment::frames(outputs))
    }

    /// Push a row frame into the current region.
    fn push_row(&mut self, frame: Frame, y: usize) {
        self.regions.size.y -= frame.height();
        self.lrows.push(Row::Frame(frame, y));
    }

    /// Finish rows for one region.
    pub(super) fn finish_region(&mut self, engine: &mut Engine) -> SourceResult<()> {
        if self.lrows.last().is_some_and(|row| {
            let (Row::Frame(_, y) | Row::Fr(_, y)) = row;
            self.grid.is_gutter_track(*y)
        }) {
            // Remove the last row in the region if it is a gutter row.
            self.lrows.pop().unwrap();
        }
        // Determine the height of existing rows in the region.
        let mut used = Abs::zero();
        let mut fr = Fr::zero();
        for row in &self.lrows {
            match row {
                Row::Frame(frame, _) => used += frame.height(),
                Row::Fr(v, _) => fr += *v,
            }
        }

        // Determine the size of the grid in this region, expanding fully if
        // there are fr rows.
        let mut size = Size::new(self.width, used).min(self.initial);
        if fr.get() > 0.0 && self.initial.y.is_finite() {
            size.y = self.initial.y;
        }

        // The frame for the region.
        let mut output = Frame::soft(size);
        let mut pos = Point::zero();
        let mut rrows = vec![];
        let current_region = self.finished.len();

        // Place finished rows and layout fractional rows.
        for row in std::mem::take(&mut self.lrows) {
            let (frame, y) = match row {
                Row::Frame(frame, y) => (frame, y),
                Row::Fr(v, y) => {
                    let remaining = self.regions.full - used;
                    let height = v.share(fr, remaining);
                    (self.layout_single_row(engine, height, y)?, y)
                }
            };

            let height = frame.height();

            // Ensure rowspans which span this row will have enough space to
            // be laid out over it later.
            for rowspan in self
                .rowspans
                .iter_mut()
                .filter(|rowspan| (rowspan.y..rowspan.y + rowspan.rowspan).contains(&y))
            {
                // If the first region wasn't defined yet, it will have the the
                // initial value of usize::MAX, so we can set it to the current
                // region's index.
                if rowspan.first_region > current_region {
                    rowspan.first_region = current_region;
                    // The rowspan starts at this region, precisely at this
                    // row. In other regions, it will start at dy = 0.
                    rowspan.dy = pos.y;
                    // When we layout the rowspan later, the full size of the
                    // pod must be equal to the full size of the first region
                    // it appears in.
                    rowspan.region_full = self.regions.full;
                }
                let amount_missing_heights = (current_region + 1)
                    .saturating_sub(rowspan.heights.len() + rowspan.first_region);
                // Ensure the vector of heights is long enough such that the
                // last height is the one for the current region.
                rowspan
                    .heights
                    .extend(std::iter::repeat(Abs::zero()).take(amount_missing_heights));
                // Ensure that, in this region, the rowspan will span at least
                // this row.
                *rowspan.heights.last_mut().unwrap() += height;
            }

            output.push_frame(pos, frame);
            rrows.push(RowPiece { height, y });
            pos.y += height;
        }

        self.finished.push(output);
        self.rrows.push(rrows);
        self.regions.next();
        self.initial = self.regions.size;

        Ok(())
    }
}

/// Turn an iterator of extents into an iterator of offsets before, in between,
/// and after the extents, e.g. [10mm, 5mm] -> [0mm, 10mm, 15mm].
fn points(extents: impl IntoIterator<Item = Abs>) -> impl Iterator<Item = Abs> {
    let mut offset = Abs::zero();
    std::iter::once(Abs::zero()).chain(extents).map(move |extent| {
        offset += extent;
        offset
    })
}
