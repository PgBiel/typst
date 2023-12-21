use crate::diag::SourceResult;
use crate::engine::Engine;
use crate::foundations::{
    cast, elem, scope, Content, NativeElement, Show, Smart, StyleChain,
};
use crate::layout::{
    Abs, Align, AlignElem, Axes, Cell, CellGrid, Celled, Fragment, GridLayouter, Layout,
    Length, Regions, Rel, ResolvableCell, Sides, TrackSizings,
};
use crate::model::Figurable;
use crate::text::{Lang, LocalName, Region};
use crate::visualize::{Paint, Stroke};

/// A table of items.
///
/// Tables are used to arrange content in cells. Cells can contain arbitrary
/// content, including multiple paragraphs and are specified in row-major order.
/// Because tables are just grids with configurable cell properties, refer to
/// the [grid documentation]($grid) for more information on how to size the
/// table tracks.
///
/// To give a table a caption and make it [referenceable]($ref), put it into a
/// [figure]($figure).
///
/// # Example
/// ```example
/// #table(
///   columns: (1fr, auto, auto),
///   inset: 10pt,
///   align: horizon,
///   [], [*Area*], [*Parameters*],
///   image("cylinder.svg"),
///   $ pi h (D^2 - d^2) / 4 $,
///   [
///     $h$: height \
///     $D$: outer radius \
///     $d$: inner radius
///   ],
///   image("tetrahedron.svg"),
///   $ sqrt(2) / 12 a^3 $,
///   [$a$: edge length]
/// )
/// ```
#[elem(scope, Layout, LocalName, Figurable)]
pub struct TableElem {
    /// The column sizes. See the [grid documentation]($grid) for more
    /// information on track sizing.
    #[borrowed]
    pub columns: TrackSizings,

    /// The row sizes. See the [grid documentation]($grid) for more information
    /// on track sizing.
    #[borrowed]
    pub rows: TrackSizings,

    /// The gaps between rows & columns. See the [grid documentation]($grid) for
    /// more information on gutters.
    #[external]
    pub gutter: TrackSizings,

    /// The gaps between columns. Takes precedence over `gutter`. See the
    /// [grid documentation]($grid) for more information on gutters.
    #[borrowed]
    #[parse(
        let gutter = args.named("gutter")?;
        args.named("column-gutter")?.or_else(|| gutter.clone())
    )]
    pub column_gutter: TrackSizings,

    /// The gaps between rows. Takes precedence over `gutter`. See the
    /// [grid documentation]($grid) for more information on gutters.
    #[parse(args.named("row-gutter")?.or_else(|| gutter.clone()))]
    #[borrowed]
    pub row_gutter: TrackSizings,

    /// How to fill the cells.
    ///
    /// This can be a color or a function that returns a color. The function is
    /// passed the cells' column and row index, starting at zero. This can be
    /// used to implement striped tables.
    ///
    /// ```example
    /// #table(
    ///   fill: (col, _) => if calc.odd(col) { luma(240) } else { white },
    ///   align: (col, row) =>
    ///     if row == 0 { center }
    ///     else if col == 0 { left }
    ///     else { right },
    ///   columns: 4,
    ///   [], [*Q1*], [*Q2*], [*Q3*],
    ///   [Revenue:], [1000 €], [2000 €], [3000 €],
    ///   [Expenses:], [500 €], [1000 €], [1500 €],
    ///   [Profit:], [500 €], [1000 €], [1500 €],
    /// )
    /// ```
    #[borrowed]
    pub fill: Celled<Option<Paint>>,

    /// How to align the cells' content.
    ///
    /// This can either be a single alignment, an array of alignments
    /// (corresponding to each column) or a function that returns an alignment.
    /// The function is passed the cells' column and row index, starting at zero.
    /// If set to `{auto}`, the outer alignment is used.
    ///
    /// ```example
    /// #table(
    ///   columns: 3,
    ///   align: (x, y) => (left, center, right).at(x),
    ///   [Hello], [Hello], [Hello],
    ///   [A], [B], [C],
    /// )
    /// ```
    #[borrowed]
    pub align: Celled<Smart<Align>>,

    /// How to [stroke]($stroke) the cells.
    ///
    /// Strokes can be disabled by setting this to `{none}`.
    ///
    /// _Note:_ Richer stroke customization for individual cells is not yet
    /// implemented, but will be in the future. In the meantime, you can use the
    /// third-party [tablex library](https://github.com/PgBiel/typst-tablex/).
    #[resolve]
    #[fold]
    #[default(Some(Stroke::default()))]
    pub stroke: Option<Stroke>,

    /// How much to pad the cells' content.
    ///
    /// ```example
    /// #table(
    ///   inset: 10pt,
    ///   [Hello],
    ///   [World],
    /// )
    ///
    /// #table(
    ///   columns: 2,
    ///   inset: (
    ///     x: 20pt,
    ///     y: 10pt,
    ///   ),
    ///   [Hello],
    ///   [World],
    /// )
    /// ```
    #[fold]
    #[default(Sides::splat(Abs::pt(5.0).into()))]
    pub inset: Sides<Option<Rel<Length>>>,

    /// The contents of the table cells.
    #[variadic]
    pub children: Vec<TableCell>,
}

#[scope]
impl TableElem {
    #[elem]
    type TableCell;
}

impl Layout for TableElem {
    #[typst_macros::time(name = "table", span = self.span())]
    fn layout(
        &self,
        engine: &mut Engine,
        styles: StyleChain,
        regions: Regions,
    ) -> SourceResult<Fragment> {
        let inset = self.inset(styles);
        let align = self.align(styles);
        let columns = self.columns(styles);
        let rows = self.rows(styles);
        let column_gutter = self.column_gutter(styles);
        let row_gutter = self.row_gutter(styles);
        let fill = self.fill(styles);
        let stroke = self.stroke(styles).map(Stroke::unwrap_or_default);

        let tracks = Axes::new(columns.0.as_slice(), rows.0.as_slice());
        let gutter = Axes::new(column_gutter.0.as_slice(), row_gutter.0.as_slice());
        let grid = CellGrid::new(tracks, gutter, self.children().clone(), styles)
            .resolve_cells(engine, fill, align, inset, styles)?;

        // Prepare grid layout by unifying content and gutter tracks.
        let layouter = GridLayouter::new(&grid, &stroke, regions, styles, self.span());

        Ok(layouter.layout(engine)?.fragment)
    }
}

impl LocalName for TableElem {
    fn local_name(lang: Lang, _: Option<Region>) -> &'static str {
        match lang {
            Lang::ALBANIAN => "Tabel",
            Lang::ARABIC => "جدول",
            Lang::BOKMÅL => "Tabell",
            Lang::CHINESE => "表",
            Lang::CZECH => "Tabulka",
            Lang::DANISH => "Tabel",
            Lang::DUTCH => "Tabel",
            Lang::ESTONIAN => "Tabel",
            Lang::FILIPINO => "Talaan",
            Lang::FINNISH => "Taulukko",
            Lang::FRENCH => "Tableau",
            Lang::GERMAN => "Tabelle",
            Lang::GREEK => "Πίνακας",
            Lang::HUNGARIAN => "Táblázat",
            Lang::ITALIAN => "Tabella",
            Lang::NYNORSK => "Tabell",
            Lang::POLISH => "Tabela",
            Lang::PORTUGUESE => "Tabela",
            Lang::ROMANIAN => "Tabelul",
            Lang::RUSSIAN => "Таблица",
            Lang::SERBIAN => "Табела",
            Lang::SLOVENIAN => "Tabela",
            Lang::SPANISH => "Tabla",
            Lang::SWEDISH => "Tabell",
            Lang::TURKISH => "Tablo",
            Lang::UKRAINIAN => "Таблиця",
            Lang::VIETNAMESE => "Bảng",
            Lang::JAPANESE => "表",
            Lang::ENGLISH | _ => "Table",
        }
    }
}

impl Figurable for TableElem {}

/// A cell in the table.
#[elem(name = "cell", title = "Table Cell", Show)]
pub struct TableCell {
    /// The cell's body.
    #[required]
    body: Content,

    /// The cell's fill override.
    fill: Smart<Option<Paint>>,

    /// The cell's alignment override.
    align: Smart<Align>,

    /// The cell's inset override.
    inset: Smart<Sides<Option<Rel<Length>>>>,
}

cast! {
    TableCell,
    v: Content => v.into(),
}

impl Cell for TableCell {
    fn fill(&self, styles: StyleChain) -> Option<Paint> {
        // The fill must have been resolved by the point it's requested.
        self.fill(styles).unwrap_or(None)
    }
}

impl ResolvableCell for TableCell {
    fn resolve_cell(
        &mut self,
        _x: usize,
        _y: usize,
        fill: &Option<Paint>,
        align: Smart<Align>,
        inset: Sides<Rel<Length>>,
        styles: StyleChain,
    ) {
        self.push_fill(Smart::Custom(self.fill(styles).unwrap_or_else(|| fill.clone())));
        self.push_align(self.align(styles).or(align));
        self.push_inset(Smart::Custom(
            self.inset(styles).unwrap_or_else(|| inset.map(Some)),
        ));
    }
}

impl Show for TableCell {
    fn show(&self, _engine: &mut Engine, styles: StyleChain) -> SourceResult<Content> {
        let inset = self.inset(styles).unwrap_or_default().map(Option::unwrap_or_default);

        let mut body = self.body().clone();

        if inset != Sides::default() {
            // Only pad if some inset is not 0pt.
            // Avoids a bug where using .padded() in any way inside Show causes
            // alignment in align(...) to break.
            body = body.padded(inset);
        }

        if let Smart::Custom(alignment) = self.align(styles) {
            body = body.styled(AlignElem::set_alignment(alignment));
        }

        Ok(body)
    }
}

impl Layout for TableCell {
    fn layout(
        &self,
        engine: &mut Engine,
        styles: StyleChain,
        regions: Regions,
    ) -> SourceResult<Fragment> {
        self.clone().pack().layout(engine, styles, regions)
    }
}

impl From<Content> for TableCell {
    fn from(value: Content) -> Self {
        value
            .to::<Self>()
            .cloned()
            .unwrap_or_else(|| Self::new(value.clone()))
    }
}
