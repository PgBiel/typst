use crate::eval::Value;
use crate::model::{Content, Guard, Location, Styles};
use crate::syntax::Span;
use comemo::Prehashed;
use ecow::{EcoString, EcoVec};
use std::iter::Filter;

/// Attributes that can be attached to content.
#[derive(Debug, Clone, PartialEq, Hash)]
pub(super) enum Attr {
    Header(ContentHeader),
    Span(Span),
    Value(Prehashed<Value>),
    Child(Prehashed<Content>),
    Styles(Styles),
    Guard(Guard),
    Location(Location),
    Field(EcoString),
}

#[derive(Debug, Default, PartialEq, Clone, Hash)]
pub(super) struct ContentHeader {
    span_index: Option<usize>,
    styles_index: Option<usize>,
    location_index: Option<usize>,
    prepared: bool,
}

/// Organizes the Content's attributes
/// using a vector.
#[derive(Debug, Default, Clone, Hash)]
pub(super) struct ContentAttrs {
    attrs: EcoVec<Attr>,
}

impl ContentAttrs {
    pub(super) fn new() -> Self {
        Self { attrs: EcoVec::new() }
    }

    fn init_header(&mut self) {
        // Append header only when there are no elements.
        if self.attrs.is_empty() {
            self.attrs.push(Attr::Header(ContentHeader::default()));
        }
    }

    fn header(&self) -> Option<&ContentHeader> {
        self.attrs.get(0).and_then(Attr::header)
    }

    fn header_mut(&mut self) -> Option<&mut ContentHeader> {
        self.attrs.make_mut().get_mut(0).and_then(Attr::header_mut)
    }

    /// Push an attribute to the attribute list,
    /// or replaces them if needed.
    fn push_attr(&mut self, attr: Attr) {
        self.init_header();

        match &attr {
            Attr::Span(_) => {
                // We have to fetch length here to avoid borrowing issues.
                let next_index = self.attrs.len();
                let mut_attrs = self.attrs.make_mut();
                let header = mut_attrs[0].header_mut().unwrap();
                if let Some(span_index) = header.span_index {
                    mut_attrs[span_index] = attr;
                    return;
                } else {
                    // Push at the end; keep track of the index.
                    header.span_index = Some(next_index);
                }
            }
            Attr::Styles(_) => {
                // We have to fetch length here to avoid borrowing issues.
                let next_index = self.attrs.len();
                let mut_attrs = self.attrs.make_mut();
                let header = mut_attrs[0].header_mut().unwrap();
                if let Some(styles_index) = header.styles_index {
                    mut_attrs[styles_index] = attr;
                    return;
                } else {
                    // Push at the end; keep track of the index.
                    header.styles_index = Some(next_index);
                }
            }
            Attr::Location(_) => {
                // We have to fetch length here to avoid borrowing issues.
                let next_index = self.attrs.len();
                let mut_attrs = self.attrs.make_mut();
                let header = mut_attrs[0].header_mut().unwrap();
                if let Some(location_index) = header.location_index {
                    mut_attrs[location_index] = attr;
                    return;
                } else {
                    // Push at the end; keep track of the index.
                    header.location_index = Some(next_index);
                }
            }
            // The other attributes can have an arbitrary number of copies.
            _ => {}
        };

        self.attrs.push(attr);
    }

    /// Push several attributes to the attribute list,
    /// or replace them if needed.
    fn push_attrs(&mut self, attrs: impl IntoIterator<Item = Attr>) {
        // Use 'push_attr' to ensure 'style', 'span' and 'location'
        // are properly handled
        attrs.into_iter().for_each(|attr| self.push_attr(attr));
    }

    /// Ensures an attribute has priority by placing it at
    /// the beginning (or replacing the existing one, if applicable).
    fn prioritize_attr(&mut self, attr: Attr) {
        if attr.is_variadic() {
            self.init_header();
            self.attrs.insert(1, attr);
        } else {
            self.push_attr(attr);
        }
    }

    /// Gets the attribute at the given index, read-only.
    fn get(&self, index: usize) -> Option<&Attr> {
        self.attrs.get(index)
    }

    /// Gets the attribute at the given index, read-write.
    fn get_mut(&mut self, index: usize) -> Option<&mut Attr> {
        self.attrs.make_mut().get_mut(index)
    }

    pub(super) fn span(&self) -> Option<Span> {
        self.header()
            .and_then(|header| header.span_index)
            .and_then(|i| self.get(i))
            .and_then(Attr::span)
    }

    pub(super) fn styles(&self) -> Option<&Styles> {
        self.header()
            .and_then(|header| header.styles_index)
            .and_then(|i| self.get(i))
            .and_then(Attr::styles)
    }

    pub(super) fn location(&self) -> Option<Location> {
        self.header()
            .and_then(|header| header.location_index)
            .and_then(|i| self.get(i))
            .and_then(Attr::location)
    }

    pub(super) fn is_prepared(&self) -> bool {
        self.header().map_or(false, |header| header.prepared)
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = &Attr> {
        self.attrs.iter().filter(|attr| attr.is_variadic())
    }

    /// Attach a field to the content.
    pub(super) fn push_field(
        &mut self,
        name: impl Into<EcoString>,
        value: impl Into<Value>,
    ) {
        let name = name.into();
        if let Some(i) = self.attrs.iter().position(|attr| match attr {
            Attr::Field(field) => *field == name,
            _ => false,
        }) {
            self.attrs.make_mut()[i + 1] = Attr::Value(Prehashed::new(value.into()));
        } else {
            self.push_attr(Attr::Field(name));
            self.push_attr(Attr::Value(Prehashed::new(value.into())));
        }
    }

    /// Attach a guard to the content.
    pub(super) fn push_guard(&mut self, guard: Guard) {
        self.push_attr(Attr::Guard(guard));
    }

    /// Attach a child to the content.
    pub(super) fn push_child(&mut self, child: Content) {
        self.push_attr(Attr::Child(Prehashed::new(child)));
    }

    /// Attach a child to the beginning of the content.
    pub(super) fn prioritize_child(&mut self, child: Content) {
        self.prioritize_attr(Attr::Child(Prehashed::new(child)));
    }

    /// Attach several children to the content.
    pub(super) fn push_children(&mut self, children: impl Iterator<Item = Content>) {
        self.push_attrs(children.map(|child| Attr::Child(Prehashed::new(child))))
    }

    pub(super) fn children_ref(&self) -> impl Iterator<Item = &Content> {
        self.attrs.iter().filter_map(Attr::child)
    }

    /// Access a field on the content by reference.
    pub(super) fn fields_ref(&self) -> impl Iterator<Item = (&EcoString, &Value)> {
        let mut iter = self.iter();

        std::iter::from_fn(move || {
            let field = iter.find_map(Attr::field)?;
            let value = iter.next()?.value()?;
            Some((field, value))
        })
    }

    /// Whether there are no attributes besides the header.
    pub(super) fn is_empty(&self) -> bool {
        return !self.attrs.iter().any(|attr| matches!(attr, Attr::Header(_)));
    }

    pub(super) fn is_styled(&self) -> bool {
        self.header().and_then(|header| header.styles_index).is_some()
    }

    pub(super) fn is_guarded(&self, guard: Guard) -> bool {
        self.attrs.contains(&Attr::Guard(guard))
    }

    pub(super) fn is_pristine(&self) -> bool {
        !self.attrs.iter().any(|modifier| matches!(modifier, Attr::Guard(_)))
    }

    /// Traverse this content
    pub(super) fn traverse<'a, F>(&'a self, this_content: &'a Content, f: &mut F)
    where
        F: FnMut(&'a Content),
    {
        f(this_content);

        for attr in self.iter() {
            match attr {
                Attr::Child(child) => child.traverse(f),
                Attr::Value(value) => walk_value(value, f),
                _ => {}
            }
        }

        /// Walks a given value to find any content that matches the selector.
        fn walk_value<'a, F>(value: &'a Value, f: &mut F)
        where
            F: FnMut(&'a Content),
        {
            match value {
                Value::Content(content) => content.traverse(f),
                Value::Array(array) => {
                    for value in array {
                        walk_value(value, f);
                    }
                }
                _ => {}
            }
        }
    }

    pub(super) fn set_span(&mut self, span: Span) {
        self.push_attr(Attr::Span(span))
    }

    pub(super) fn set_styles(&mut self, styles: Styles) {
        self.push_attr(Attr::Styles(styles))
    }

    pub(super) fn set_prepared(&mut self) {
        self.init_header();
        self.header_mut().unwrap().prepared = true;
    }

    pub(super) fn set_location(&mut self, location: Location) {
        self.push_attr(Attr::Location(location))
    }

    pub(super) fn styles_mut(&mut self) -> Option<&mut Styles> {
        self.header()
            .and_then(|header| header.styles_index)
            .and_then(|i| self.get_mut(i))
            .and_then(Attr::styles_mut)
    }

    /// Joins this Content's variadic attributes with another's.
    pub(super) fn extend(&mut self, other: impl IntoIterator<Item = Attr>) {
        self.attrs.extend(other)
    }
}

impl IntoIterator for ContentAttrs {
    type Item = Attr;
    type IntoIter = Filter<<EcoVec<Attr> as IntoIterator>::IntoIter, fn(&Attr) -> bool>;

    fn into_iter(self) -> Self::IntoIter {
        self.attrs.into_iter().filter(|attr| attr.is_variadic())
    }
}

impl Attr {
    fn header(&self) -> Option<&ContentHeader> {
        match self {
            Self::Header(header) => Some(header),
            _ => None,
        }
    }

    fn header_mut(&mut self) -> Option<&mut ContentHeader> {
        match self {
            Self::Header(header) => Some(header),
            _ => None,
        }
    }

    fn location(&self) -> Option<Location> {
        match self {
            Self::Location(location) => Some(*location),
            _ => None,
        }
    }

    pub(super) fn child(&self) -> Option<&Content> {
        match self {
            Self::Child(child) => Some(child),
            _ => None,
        }
    }

    pub(super) fn styles(&self) -> Option<&Styles> {
        match self {
            Self::Styles(styles) => Some(styles),
            _ => None,
        }
    }

    pub(super) fn styles_mut(&mut self) -> Option<&mut Styles> {
        match self {
            Self::Styles(styles) => Some(styles),
            _ => None,
        }
    }

    pub(super) fn field(&self) -> Option<&EcoString> {
        match self {
            Self::Field(field) => Some(field),
            _ => None,
        }
    }

    pub(super) fn value(&self) -> Option<&Value> {
        match self {
            Self::Value(value) => Some(value),
            _ => None,
        }
    }

    pub(super) fn span(&self) -> Option<Span> {
        match self {
            Self::Span(span) => Some(*span),
            _ => None,
        }
    }

    /// Returns whether a content can have more than one of this attribute.
    fn is_variadic(&self) -> bool {
        matches!(self, Self::Child(_) | Self::Field(_) | Self::Value(_))
    }
}
